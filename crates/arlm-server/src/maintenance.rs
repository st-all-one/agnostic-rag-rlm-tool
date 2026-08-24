//! Server-side memory maintenance (plan 019, C.1).
//!
//! Two operations keep the shared store healthy:
//!
//! - [`consolidate`]: deduplicate chunks and drop low-confidence patterns via
//!   [`arlm_memory::ConsolidationEngine`].
//! - [`decay`]: remove chunks whose salience (computed by
//!   [`arlm_search::decay::DecayConfig`] from `last_accessed_at`) has fallen
//!   below `score_floor`.
//!
//! Both report counts through [`MaintenanceReport`] (mirrors the proto message)
//! and honor a `dry_run` that computes the report without deleting anything.

use anyhow::{Context, Result};
use arlm_memory::consolidation::{ConsolidateOptions, ConsolidationEngine};
use arlm_search::decay::DecayConfig;
use arlm_storage::Storage;
use rusqlite::params;

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
/// # Errors
///
/// Returns an error if storage access fails.
pub fn consolidate(project: &str, storage: &Storage, dry_run: bool) -> Result<MaintenanceReport> {
    let buffer_ids = resolve_buffer_ids(storage, project)?;
    let engine = ConsolidationEngine::new(storage.clone());
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
/// # Errors
///
/// Returns an error if storage access fails.
pub async fn decay(
    project: &str,
    storage: &Storage,
    score_floor: f32,
    dry_run: bool,
) -> Result<MaintenanceReport> {
    let buffer_ids = resolve_buffer_ids(storage, project)?;
    let decay_cfg = DecayConfig::default();

    let mut report = MaintenanceReport::default();
    for bid in buffer_ids {
        let count =
            run_decay_for_buffer(storage.clone(), bid, decay_cfg, score_floor, dry_run).await?;
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
                    }
                } else {
                    count.kept += 1;
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
    score_floor: f32,
    dry_run: bool,
) -> Result<MaintenanceReport> {
    let mut report = consolidate(project, storage, dry_run)?;
    let decay_report = decay(project, storage, score_floor, dry_run).await?;
    report.merge(&decay_report);
    Ok(report)
}
