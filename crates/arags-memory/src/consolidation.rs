use anyhow::{Context, Result};

use arags_storage::Storage;

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
}

impl ConsolidationEngine {
    /// Create a new `ConsolidationEngine`.
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
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
            result.duplicate_chunks_removed =
                self.remove_duplicate_chunks(buffer_id, options.dry_run)?;
        }

        result.low_confidence_patterns_removed = self.remove_low_confidence_patterns(
            buffer_id,
            options.min_pattern_confidence,
            options.dry_run,
        )?;

        tracing::info!(
            buffer_id,
            dry_run = options.dry_run,
            duplicates_removed = result.duplicate_chunks_removed,
            patterns_removed = result.low_confidence_patterns_removed,
            "consolidation completed"
        );

        Ok(result)
    }

    fn remove_duplicate_chunks(&self, buffer_id: i64, dry_run: bool) -> Result<u64> {
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
                // Keep the first chunk, remove the rest
                let mut del_stmt = conn
                    .prepare(
                        "DELETE FROM chunks WHERE id IN (
                            SELECT id FROM chunks WHERE buffer_id = ?1 AND hash = ?2
                            ORDER BY id DESC
                            LIMIT -1 OFFSET 1
                        )",
                    )
                    .context("failed to prepare delete duplicates")?;

                let deleted = del_stmt
                    .execute(rusqlite::params![buffer_id, hash])
                    .context("failed to delete duplicates")?;

                removed += u64::try_from(deleted).unwrap_or(0);
            }
        }

        Ok(removed)
    }

    fn remove_low_confidence_patterns(
        &self,
        buffer_id: i64,
        min_confidence: f64,
        dry_run: bool,
    ) -> Result<u64> {
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
            return Ok(count);
        }

        let deleted = conn
            .execute(
                "DELETE FROM patterns WHERE buffer_id = ?1 AND confidence < ?2 AND confidence IS NOT NULL",
                rusqlite::params![buffer_id, min_confidence],
            )
            .context("failed to remove low confidence patterns")?;

        Ok(u64::try_from(deleted).unwrap_or(0))
    }
}
