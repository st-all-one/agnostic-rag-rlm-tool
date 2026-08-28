use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};

use arags_storage::{Storage, VectorStore};
use tracing::{debug, info, warn};

use crate::ScopedTimer;

/// Options for consolidation.
#[derive(Debug, Clone)]
pub struct ConsolidateOptions {
    /// Remove chunks with duplicate hashes.
    pub deduplicate: bool,
    /// Remove patterns with confidence below this threshold.
    pub min_pattern_confidence: f64,
    /// When true, only count what *would* be removed; perform no deletes.
    pub dry_run: bool,
}

impl Default for ConsolidateOptions {
    fn default() -> Self {
        Self {
            deduplicate: true,
            min_pattern_confidence: 0.3,
            dry_run: false,
        }
    }
}

/// Result of a consolidation operation.
#[derive(Debug, Clone)]
pub struct ConsolidateResult {
    pub duplicate_chunks_removed: u64,
    pub low_confidence_patterns_removed: u64,
}

/// Handles memory consolidation: deduplication, cleanup, aggregation.
pub struct ConsolidationEngine {
    storage: Storage,
    /// Optional chunk `VectorStore`. When present, chunks removed during
    /// deduplication also have their usearch vectors purged so the semantic
    /// index stays in sync with canonical `SQLite` (issue `agnostic-rlm-rs-fa25`).
    /// When absent (e.g. unit tests), vector cleanup is skipped.
    vector_store: Option<Arc<VectorStore>>,
}

impl ConsolidationEngine {
    /// Create a new `ConsolidationEngine` without a vector store.
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self {
            storage,
            vector_store: None,
        }
    }

    /// Attach the chunk `VectorStore` so deduplicated chunks also have their
    /// vectors purged. Consumes `self` to keep construction fluent.
    #[must_use]
    pub fn with_vector_store(mut self, vs: Arc<VectorStore>) -> Self {
        self.vector_store = Some(vs);
        self
    }

    /// Consolidate memory for a project.
    ///
    /// Removes duplicate chunks and low-confidence patterns.
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub fn consolidate(
        &self,
        buffer_id: i64,
        options: &ConsolidateOptions,
    ) -> Result<ConsolidateResult> {
        let _timer = ScopedTimer::new("consolidation");

        let mut result = ConsolidateResult {
            duplicate_chunks_removed: 0,
            low_confidence_patterns_removed: 0,
        };

        if options.deduplicate {
            let stage = Instant::now();
            result.duplicate_chunks_removed =
                self.remove_duplicate_chunks(buffer_id, options.dry_run)?;
            info!(
                buffer_id,
                dry_run = options.dry_run,
                removed = result.duplicate_chunks_removed,
                duration_ms = %stage.elapsed().as_millis(),
                "duplicate chunk stage complete"
            );
        }

        let stage = Instant::now();
        result.low_confidence_patterns_removed = self.remove_low_confidence_patterns(
            buffer_id,
            options.min_pattern_confidence,
            options.dry_run,
        )?;
        info!(
            buffer_id,
            dry_run = options.dry_run,
            removed = result.low_confidence_patterns_removed,
            duration_ms = %stage.elapsed().as_millis(),
            "low confidence pattern stage complete"
        );

        info!(
            buffer_id,
            dry_run = options.dry_run,
            duplicates_removed = result.duplicate_chunks_removed,
            patterns_removed = result.low_confidence_patterns_removed,
            "consolidation completed"
        );

        Ok(result)
    }

    fn remove_duplicate_chunks(&self, buffer_id: i64, dry_run: bool) -> Result<u64> {
        let start = Instant::now();
        let conn = self.storage.conn();
        let conn = conn.lock();

        // Find duplicate hashes within the same buffer
        let mut stmt = conn
            .prepare(
                "SELECT hash, COUNT(*) as cnt FROM chunks WHERE buffer_id = ?1 GROUP BY hash HAVING cnt > 1",
            )
            .context("failed to prepare duplicate query")?;

        let duplicate_hashes: Vec<Vec<u8>> = stmt
            .query_map([buffer_id], |row| row.get(0))?
            .filter_map(std::result::Result::ok)
            .collect();

        let mut removed: u64 = 0;
        // Chunk ids physically removed (non-dry-run), so their vectors can be
        // purged from the usearch store afterwards.
        let mut purged_chunk_ids: Vec<u64> = Vec::new();

        for hash in &duplicate_hashes {
            if dry_run {
                // Keep the first chunk, so everything beyond it would be removed.
                let mut count_stmt = conn
                    .prepare("SELECT COUNT(*) FROM chunks WHERE buffer_id = ?1 AND hash = ?2")
                    .context("failed to prepare count duplicates")?;
                let total: u64 = count_stmt
                    .query_row(rusqlite::params![buffer_id, hash], |r| r.get(0))
                    .context("failed to count duplicates")?;
                removed += total.saturating_sub(1);
            } else {
                // Keep the first chunk, remove the rest. Several child tables
                // reference `chunks(id)` through FKs that are NOT declared
                // `ON DELETE CASCADE`, so deleting a chunk directly fails with a
                // foreign-key constraint violation. Purge the dependent rows
                // first, in FK-respecting order (findings before tasks, both
                // before chunk_texts, then the chunks themselves).
                let dup_ids = "SELECT id FROM chunks WHERE buffer_id = ?1 AND hash = ?2 ORDER BY id DESC LIMIT -1 OFFSET 1";

                // Capture the ids that will be removed (everything but the
                // kept first row) so we can drop their vectors too.
                let mut id_stmt = conn
                    .prepare(dup_ids)
                    .context("failed to prepare duplicate id query")?;
                let ids: Vec<u64> = id_stmt
                    .query_map(rusqlite::params![buffer_id, hash], |r| {
                        r.get::<_, i64>(0).map(|v| u64::try_from(v).unwrap_or(0))
                    })?
                    .filter_map(std::result::Result::ok)
                    .collect();
                purged_chunk_ids.extend(ids);

                conn.execute(
                    &format!(
                        "DELETE FROM findings WHERE chunk_id IN ({dup_ids}) \
                         OR task_id IN (SELECT id FROM tasks WHERE chunk_id IN ({dup_ids}))"
                    ),
                    rusqlite::params![buffer_id, hash],
                )
                .context("failed to delete duplicate finding rows")?;
                conn.execute(
                    &format!("DELETE FROM tasks WHERE chunk_id IN ({dup_ids})"),
                    rusqlite::params![buffer_id, hash],
                )
                .context("failed to delete duplicate task rows")?;
                conn.execute(
                    &format!("DELETE FROM chunk_texts WHERE chunk_id IN ({dup_ids})"),
                    rusqlite::params![buffer_id, hash],
                )
                .context("failed to delete duplicate chunk_texts rows")?;

                let deleted = conn
                    .execute(
                        &format!("DELETE FROM chunks WHERE id IN ({dup_ids})"),
                        rusqlite::params![buffer_id, hash],
                    )
                    .context("failed to delete duplicates")?;

                removed += u64::try_from(deleted).unwrap_or(0);
            }
        }

        // Drop the orphan vectors so the usearch chunk count matches SQLite and
        // the server bootstrap no longer sees a divergence (and thus no
        // expensive full rebuild). Best-effort: a failure is logged, never fatal.
        if let Some(vs) = &self.vector_store {
            if !purged_chunk_ids.is_empty() {
                match vs.delete_chunk_ids_blocking(&purged_chunk_ids) {
                    Ok(()) => info!(
                        buffer_id,
                        count = purged_chunk_ids.len(),
                        "purged orphan vectors for consolidated chunks"
                    ),
                    Err(e) => warn!(
                        error = %e,
                        buffer_id,
                        count = purged_chunk_ids.len(),
                        "failed to purge orphan vectors for consolidated chunks"
                    ),
                }
            }
        }

        debug!(
            buffer_id,
            dry_run,
            removed,
            duration_ms = %start.elapsed().as_millis(),
            "duplicate chunk removal complete"
        );

        Ok(removed)
    }

    fn remove_low_confidence_patterns(
        &self,
        buffer_id: i64,
        min_confidence: f64,
        dry_run: bool,
    ) -> Result<u64> {
        let start = Instant::now();
        let conn = self.storage.conn();
        let conn = conn.lock();

        if dry_run {
            let count: u64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM patterns WHERE buffer_id = ?1 AND confidence < ?2 AND confidence IS NOT NULL",
                    rusqlite::params![buffer_id, min_confidence],
                    |r| r.get(0),
                )
                .context("failed to count low confidence patterns")?;
            debug!(
                buffer_id,
                min_confidence,
                count,
                duration_ms = %start.elapsed().as_millis(),
                "low confidence pattern dry run complete"
            );
            return Ok(count);
        }

        let deleted = conn
            .execute(
                "DELETE FROM patterns WHERE buffer_id = ?1 AND confidence < ?2 AND confidence IS NOT NULL",
                rusqlite::params![buffer_id, min_confidence],
            )
            .context("failed to remove low confidence patterns")?;

        let removed = u64::try_from(deleted).unwrap_or(0);
        debug!(
            buffer_id,
            min_confidence,
            removed,
            duration_ms = %start.elapsed().as_millis(),
            "low confidence pattern removal complete"
        );

        Ok(removed)
    }
}
