//! QA-cache read/insert handlers: `store_answer` plus the exact/as-of/cache-id
//! lookups.

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

use super::row::{QA_COLS, row_mapper, store_answer_inner};
use super::types::{QaCacheRow, StoreAnswerInput, StoredAnswer};
use crate::sqlite::conn::Storage;
use crate::sqlite::tokens::now_ms;
use tracing::debug;

impl Storage {
    /// Store a digested answer, returning its stable `cache_id` and rowid.
    ///
    /// **Superseding (issue `agnostic-rag-rlm-tool-e210`):** if an active row already
    /// exists for `(project, buffer_id, question_hash)` a *new* row is inserted
    /// (`version = old + 1`, `is_active = 1`) and the previous active row is
    /// retired (`is_active = 0`, `superseded_by = new_id`). Reads therefore see
    /// only the latest active revision, while the prior answer remains available
    /// through [`Self::get_answer_history`]. No active row → a brand-new
    /// active row (`version = 1`) is inserted.
    ///
    /// The pre-existing staleness invalidation (`invalidate_stale_cache_for_buffer`
    /// / `mark_stale_by_hashes`) is preserved: a stale active row is still the
    /// "active" revision until superseded, so an exact-hit read treats it as a
    /// MISS and forces re-digest.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or any statement fails.
    pub fn store_answer(&self, input: &StoreAnswerInput) -> Result<StoredAnswer> {
        let conn = self.connection().context("failed to acquire connection")?;
        let now = now_ms();
        let start = std::time::Instant::now();
        let (cache_id, id) = conn
            .execute(|c| store_answer_inner(c, input, now))
            .context("store_answer tx")?;
        let elapsed_ms = start.elapsed().as_millis();
        debug!(
            phase = "store_answer",
            rowid = id,
            cache_id = %cache_id,
            buffer_id = input.buffer_id.unwrap_or(-1),
            project = %input.project,
            elapsed_ms,
            "qa answer stored (superseding prior active revision)"
        );

        Ok(StoredAnswer {
            cache_id,
            id,
            created: true,
        })
    }

    /// Exact-hit lookup by `(project, question_hash)`. Returns `None` on miss
    /// or if the only match is stale (caller should treat stale as a MISS that
    /// forces re-digest). Only the latest **active** revision is considered.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_cached_answer(
        &self,
        project: &str,
        question_hash: &str,
    ) -> Result<Option<QaCacheRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                &format!(
                    "SELECT {QA_COLS} FROM qa_cache \
                         WHERE project = ?1 AND question_hash = ?2 AND is_active = 1 \
                           AND stale = 0 LIMIT 1"
                ),
                params![project, question_hash],
                row_mapper,
            )
            .optional()
            .context("failed to get cached answer")
        })
    }

    /// Time-travel: return the cached answer for `(project, question_hash,
    /// buffer_id)` that was **active** at `as_of_epoch`. The active revision at
    /// time T is the one with the greatest `epoch <= T` among every revision
    /// sharing that subject (newest → oldest). Staleness is ignored: an answer
    /// later marked stale was still the live answer at T, so it is the correct
    /// time-travel result. If no revision predates T, `None` is returned.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_cached_answer_as_of(
        &self,
        project: &str,
        question_hash: &str,
        buffer_id: Option<i64>,
        as_of_epoch: i64,
    ) -> Result<Option<QaCacheRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                &format!(
                    "SELECT {QA_COLS} FROM qa_cache \
                          WHERE project = ?1 AND buffer_id IS ?2 AND question_hash = ?3 \
                            AND epoch <= ?4 ORDER BY epoch DESC, id DESC LIMIT 1"
                ),
                params![project, buffer_id, question_hash, as_of_epoch],
                row_mapper,
            )
            .optional()
            .context("failed to get cached answer as_of")
        })
    }

    /// Look up a cached answer by its stable `cache_id` (scoped by project).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_qa_by_id(&self, cache_id: &str, project: &str) -> Result<Option<QaCacheRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                &format!(
                    "SELECT {QA_COLS} FROM qa_cache WHERE cache_id = ?1 AND project = ?2 LIMIT 1"
                ),
                params![cache_id, project],
                row_mapper,
            )
            .optional()
            .context("failed to get qa_cache by id")
        })
    }

    /// Look up a cached answer by its stable `cache_id` globally (admin
    /// invalidation does not require the caller to know the project).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_qa_by_cache_id(&self, cache_id: &str) -> Result<Option<QaCacheRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                &format!("SELECT {QA_COLS} FROM qa_cache WHERE cache_id = ?1 LIMIT 1"),
                params![cache_id],
                row_mapper,
            )
            .optional()
            .context("failed to get qa_cache by cache_id")
        })
    }

    /// Look up a cached answer by numeric rowid (used by radius invalidation).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_qa_by_rowid(&self, id: i64) -> Result<Option<QaCacheRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                &format!("SELECT {QA_COLS} FROM qa_cache WHERE id = ?1 LIMIT 1"),
                params![id],
                row_mapper,
            )
            .optional()
            .context("failed to get qa_cache by rowid")
        })
    }

    /// Walk the supersede chain starting from `id`, returning every revision in
    /// oldest→newest order (issue `agnostic-rag-rlm-tool-e210`). The starting row need
    /// not be the oldest; only the forward chain reachable via `superseded_by`
    /// is returned. Retired (`is_active = 0`) revisions are included so callers
    /// can audit the full answer history.
    ///
    /// # Errors
    ///
    /// Returns an error if any query fails.
    pub fn get_answer_history(&self, id: i64) -> Result<Vec<QaCacheRow>> {
        let conn = self.connection().context("failed to acquire connection")?;
        conn.execute(|c| {
            let mut chain = Vec::new();
            let mut current: Option<i64> = Some(id);
            while let Some(cid) = current {
                let Some(row) = c
                    .query_row(
                        &format!("SELECT {QA_COLS} FROM qa_cache WHERE id = ?1"),
                        params![cid],
                        row_mapper,
                    )
                    .optional()
                    .context("failed to read qa_cache history row")?
                else {
                    break;
                };
                let next: Option<i64> = c
                    .query_row(
                        "SELECT superseded_by FROM qa_cache WHERE id = ?1",
                        params![cid],
                        |r| r.get(0),
                    )
                    .context("failed to read qa_cache superseded_by")?;
                chain.push(row);
                current = next;
            }
            Ok(chain)
        })
    }
}
