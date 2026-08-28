//! Server-side memory maintenance (plan 019, C.1).
//!
//! Two operations keep the shared store healthy:
//!
//! - [`consolidate`]: deduplicate chunks and drop low-confidence patterns via
//!   [`arags_memory::ConsolidationEngine`].
//! - [`decay`]: remove chunks whose salience (computed by
//!   [`arags_search::decay::DecayConfig`] from `last_accessed_at`) has fallen
//!   below `score_floor`.
//!
//! Both report counts through [`MaintenanceReport`] (mirrors the proto message)
//! and honor a `dry_run` that computes the report without deleting anything.

use std::sync::Arc;

use anyhow::{Context, Result};
use arags_memory::consolidation::{ConsolidateOptions, ConsolidationEngine};
use arags_search::decay::DecayConfig;
use arags_storage::{Storage, VectorStore};
use rusqlite::params;
use tracing::warn;

use crate::store;

/// Result of a maintenance pass (mirrors the proto `MaintenanceReport`).
#[derive(Debug, Clone, Default)]
pub struct MaintenanceReport {
    /// Duplicate chunks removed during consolidation.
    pub duplicate_chunks_removed: u64,
    /// Low-confidence patterns removed during consolidation.
    pub low_confidence_patterns_removed: u64,
    /// Chunks removed by decay.
    pub decayed_chunks: u64,
    /// Chunks kept by decay (evaluated but above the floor).
    pub kept: u64,
}

impl MaintenanceReport {
    fn merge(&mut self, other: &MaintenanceReport) {
        self.duplicate_chunks_removed += other.duplicate_chunks_removed;
        self.low_confidence_patterns_removed += other.low_confidence_patterns_removed;
        self.decayed_chunks += other.decayed_chunks;
        self.kept += other.kept;
    }
}

/// Resolve the buffer ids to operate on. An empty `project` means "all
/// projects".
fn resolve_buffer_ids(storage: &Storage, project: &str) -> Result<Vec<i64>> {
    if project.is_empty() {
        let projects = store::list_projects(storage)?;
        Ok(projects.into_iter().map(|p| p.id).collect())
    } else {
        Ok(store::buffer_id_for_project(storage, project)?
            .into_iter()
            .collect())
    }
}

/// Consolidate memory for a project (or every project when `project` is empty).
///
/// `vector_store` is the chunk usearch space; when provided, deduplicated chunks
/// also have their vectors purged (issue `agnostic-rlm-rs-fa25`) so the semantic
/// index stays in sync with SQLite.
///
/// # Errors
///
/// Returns an error if storage access fails.
pub fn consolidate(
    project: &str,
    storage: &Storage,
    vector_store: Option<Arc<VectorStore>>,
    dry_run: bool,
) -> Result<MaintenanceReport> {
    let buffer_ids = resolve_buffer_ids(storage, project)?;
    let mut engine = ConsolidationEngine::new(storage.clone());
    if let Some(vs) = vector_store {
        engine = engine.with_vector_store(vs);
    }
    let options = ConsolidateOptions {
        dry_run,
        ..ConsolidateOptions::default()
    };

    let mut report = MaintenanceReport::default();
    for bid in buffer_ids {
        let res = engine.consolidate(bid, &options)?;
        report.duplicate_chunks_removed += res.duplicate_chunks_removed;
        report.low_confidence_patterns_removed += res.low_confidence_patterns_removed;
    }
    Ok(report)
}

/// Decay memory for a project (or every project when `project` is empty),
/// removing chunks whose salience is below `score_floor`.
///
/// `vector_store` is the chunk usearch space; when provided, decayed chunks also
/// have their vectors purged (issue `agnostic-rlm-rs-fa25`) so the semantic index
/// stays in sync with SQLite.
///
/// # Errors
///
/// Returns an error if storage access fails.
pub async fn decay(
    project: &str,
    storage: &Storage,
    vector_store: Option<Arc<VectorStore>>,
    score_floor: f32,
    dry_run: bool,
) -> Result<MaintenanceReport> {
    let buffer_ids = resolve_buffer_ids(storage, project)?;
    let decay_cfg = DecayConfig::default();

    let mut report = MaintenanceReport::default();
    for bid in buffer_ids {
        let count = run_decay_for_buffer(
            storage.clone(),
            vector_store.clone(),
            bid,
            decay_cfg,
            score_floor,
            dry_run,
        )
        .await?;
        report.decayed_chunks += count.decayed_chunks;
        report.kept += count.kept;
    }
    Ok(report)
}

#[derive(Debug, Clone, Default)]
struct DecayCount {
    decayed_chunks: u64,
    kept: u64,
}

/// Evaluate every chunk in a buffer, deleting those whose decayed salience is
/// below the floor. Runs entirely inside one pooled/single connection so the
/// deletes never re-lock the shared SQLite mutex.
async fn run_decay_for_buffer(
    storage: Storage,
    vector_store: Option<Arc<VectorStore>>,
    buffer_id: i64,
    decay_cfg: DecayConfig,
    score_floor: f32,
    dry_run: bool,
) -> Result<DecayCount> {
    store::blocking(move || -> Result<DecayCount> {
        let conn = storage.connection()?;
        conn.execute(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, last_accessed_at FROM chunks WHERE buffer_id = ?1")
                .context("failed to prepare decay select")?;
            let rows: Vec<(i64, i64)> = stmt
                .query_map(params![buffer_id], |r| {
                    Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0)))
                })?
                .filter_map(std::result::Result::ok)
                .collect();

            let mut count = DecayCount::default();
            // Chunk ids physically removed (non-dry-run) so their vectors can
            // be purged from the usearch store afterwards.
            let mut removed_chunk_ids: Vec<u64> = Vec::new();
            for (id, last_accessed) in rows {
                let age_hours = DecayConfig::age_hours(last_accessed);
                let decayed = decay_cfg.score(1.0, age_hours);
                if decayed < score_floor {
                    count.decayed_chunks += 1;
                    if !dry_run {
                        conn.execute("DELETE FROM chunks_fts WHERE rowid = ?1", params![id])
                            .context("failed to delete chunk fts")?;
                        conn.execute("DELETE FROM chunk_texts WHERE chunk_id = ?1", params![id])
                            .context("failed to delete chunk text")?;
                        conn.execute(
                            "DELETE FROM chunk_entities WHERE chunk_id = ?1",
                            params![id],
                        )
                        .context("failed to delete chunk entities")?;
                        conn.execute("DELETE FROM chunks WHERE id = ?1", params![id])
                            .context("failed to delete chunk")?;
                        removed_chunk_ids.push(id as u64);
                    }
                } else {
                    count.kept += 1;
                }
            }

            // Drop the orphan vectors so the usearch chunk count matches SQLite
            // and the server bootstrap no longer sees a divergence (issue
            // `agnostic-rlm-rs-fa25`). Best-effort: a failure is logged.
            if let Some(vs) = &vector_store {
                if !removed_chunk_ids.is_empty() {
                    if let Err(e) = vs.delete_chunk_ids_blocking(&removed_chunk_ids) {
                        warn!(
                            error = %e,
                            buffer_id,
                            count = removed_chunk_ids.len(),
                            "failed to purge orphan vectors for decayed chunks"
                        );
                    }
                }
            }

            Ok(count)
        })
    })
    .await
}

/// Run both consolidate and decay for a project (or all projects), merging the
/// reports.
///
/// # Errors
///
/// Returns an error if either pass fails.
pub async fn run_maintenance(
    project: &str,
    storage: &Storage,
    vector_store: Option<Arc<VectorStore>>,
    score_floor: f32,
    dry_run: bool,
) -> Result<MaintenanceReport> {
    let mut report = consolidate(project, storage, vector_store.clone(), dry_run)?;
    let decay_report = decay(project, storage, vector_store, score_floor, dry_run).await?;
    report.merge(&decay_report);
    Ok(report)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use std::sync::Arc;

    use arags_storage::VectorStore;

    /// End-to-end check that `consolidate` keeps the usearch chunk space in sync
    /// with SQLite: deduplicated chunks also lose their vectors (issue
    /// `agnostic-rlm-rs-fa25`), so the server bootstrap no longer sees a
    /// count divergence that forces a full re-embed on restart.
    #[tokio::test]
    async fn consolidate_purges_orphan_vectors_for_duplicate_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).unwrap();
        let dims = 384usize;
        let vs = Arc::new(VectorStore::open_with_dims(dir.path(), dims).await.unwrap());

        // One buffer, three chunks sharing the same content hash (duplicates).
        storage
            .connection()
            .unwrap()
            .execute(|c| {
                c.execute(
                    "INSERT INTO buffers (id, name, path) VALUES (1, 'p', '/tmp/p')",
                    [],
                )?;
                for i in 0..3_i64 {
                    c.execute(
                        "INSERT INTO chunks \
                         (id, buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash, status) \
                         VALUES (?1, 1, 'f.rs', 0, 1, 1, 1, X'07', 'active')",
                        [i],
                    )?;
                }
                Ok(())
            })
            .unwrap();

        // Mirror SQLite in the vector store so the space starts in sync.
        let entries: Vec<arags_storage::VectorEntry> = (0..3)
            .map(|i| arags_storage::VectorEntry {
                chunk_id: i as u64,
                buffer_id: 1,
                vector: vec![0.0f32; dims],
            })
            .collect();
        vs.insert_vectors(&entries).await.unwrap();
        assert_eq!(vs.count().await, 3);

        let report = consolidate("", &storage, Some(vs.clone()), false).unwrap();
        assert_eq!(report.duplicate_chunks_removed, 2);

        // Only the kept chunk's vector remains; the two consolidated-away chunks
        // had their vectors purged, so the store no longer diverges.
        assert_eq!(vs.count().await, 1);
    }
}
