//! Consumer feedback loop, retirement and admin invalidation (plan 022).
//!
//! The cheapest verifier of a map is the agent that just used it:
//! `record_feedback` accumulates `confirm`/`contradict` counters. Confirmed
//! maps rank higher for future consumers; accumulated contradictions lower the
//! confidence score and, at the configured limit, retire the map pending
//! manual review. Retirement is soft: the row stays as auditable history and
//! is excluded from default search.

use anyhow::Context as _;
use anyhow::Result;
use rusqlite::OptionalExtension as _;
use rusqlite::params;

use super::super::conn::Storage;
use super::super::tokens::now_ms;
use tracing::info;

/// Outcome of a feedback submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedbackOutcome {
    /// The map was confirmed; counter updated.
    Confirmed {
        /// Total confirms after this submission.
        confirmed: i64,
    },
    /// The map was contradicted; counter updated.
    Contradicted {
        /// Total contradictions after this submission.
        contradicted: i64,
        /// Whether this contradiction crossed the limit and retired the map.
        auto_retired: bool,
    },
}

/// Direction of consumer feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackKind {
    /// Consumer verified the described mechanism in current code.
    Confirm,
    /// Consumer found evidence contradicting the map.
    Contradict,
}

impl Storage {
    /// Record consumer feedback on a map by its stable UUIDv7 id.
    ///
    /// When `contradiction_limit` is positive and the running total of
    /// contradictions reaches it, the map is atomically retired
    /// (`status='retired'`, audit fields set to `system/feedback-limit`) and
    /// the outcome reports `auto_retired = true`.
    ///
    /// Returns `None` when no map exists for the given id.
    ///
    /// # Errors
    ///
    /// Returns an error if any statement fails.
    pub fn record_feedback(
        &self,
        exploration_id: &str,
        kind: FeedbackKind,
        contradiction_limit: i64,
    ) -> Result<Option<FeedbackOutcome>> {
        let conn = self.connection().context("failed to acquire connection")?;
        let now = now_ms();
        conn.execute(|c| {
            let tx = c.unchecked_transaction().context("begin feedback tx")?;
            let id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM explorations WHERE exploration_id = ?1",
                    params![exploration_id],
                    |r| r.get(0),
                )
                .optional()
                .context("probe exploration for feedback")?;
            let Some(id) = id else {
                let _ = tx.finish();
                return Ok(None);
            };

            match kind {
                FeedbackKind::Confirm => {
                    tx.execute(
                        "UPDATE explorations SET confirmed = confirmed + 1, updated_at = ?1 \
                         WHERE id = ?2",
                        params![now, id],
                    )
                    .context("confirm exploration")?;
                    let confirmed: i64 = tx.query_row(
                        "SELECT confirmed FROM explorations WHERE id = ?1",
                        params![id],
                        |r| r.get(0),
                    )?;
                    tx.commit().context("commit feedback tx")?;
                    Ok(Some(FeedbackOutcome::Confirmed { confirmed }))
                }
                FeedbackKind::Contradict => {
                    tx.execute(
                        "UPDATE explorations SET contradicted = contradicted + 1, updated_at = ?1 \
                         WHERE id = ?2 AND status != 'retired'",
                        params![now, id],
                    )
                    .context("contradict exploration")?;
                    let contradicted: i64 = tx.query_row(
                        "SELECT contradicted FROM explorations WHERE id = ?1",
                        params![id],
                        |r| r.get(0),
                    )?;
                    let auto_retired =
                        if contradiction_limit > 0 && contradicted >= contradiction_limit {
                            let n = tx
                                .execute(
                                    "UPDATE explorations SET status = 'retired', retired_at = ?1, \
                                 retired_by = 'system', updated_at = ?1 \
                                 WHERE id = ?2 AND status != 'retired'",
                                    params![now, id],
                                )
                                .context("auto-retire exploration")?;
                            n > 0
                        } else {
                            false
                        };
                    tx.commit().context("commit feedback tx")?;
                    info!(
                        exploration_id = %exploration_id,
                        contradicted,
                        auto_retired,
                        "exploration contradicted"
                    );
                    Ok(Some(FeedbackOutcome::Contradicted {
                        contradicted,
                        auto_retired,
                    }))
                }
            }
        })
    }

    /// Soft-invalidate a map (admin `Stale` mode): mark stale with an audit
    /// trail. Returns `false` when the map does not exist or was already not
    /// fresh.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn invalidate_exploration_stale(
        &self,
        exploration_id: &str,
        invalidated_by: &str,
        reason: &str,
    ) -> Result<bool> {
        let conn = self.connection().context("failed to acquire connection")?;
        let now = now_ms();
        let changed = conn
            .execute(|c| {
                let n = c.execute(
                    "UPDATE explorations SET status = 'stale', \
                     stale_reason = COALESCE(stale_reason, json_array(?3)), updated_at = ?1 \
                     WHERE exploration_id = ?2 AND status = 'fresh'",
                    params![now, exploration_id, reason],
                )?;
                Ok(n > 0)
            })
            .context("failed to invalidate exploration")?;
        if changed {
            info!(
                exploration_id = %exploration_id,
                invalidated_by = %invalidated_by,
                reason = %reason,
                "exploration invalidated (stale)"
            );
        }
        Ok(changed)
    }

    /// Quality-gate verdict (plan 023, borrowed from the RLM review gate):
    /// approval flips a `pending_review` map to `fresh`; rejection retires it.
    /// Works from any non-retired status. Returns `false` when the map does
    /// not exist or was already retired.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn review_exploration(
        &self,
        exploration_id: &str,
        approved: bool,
        reviewer: &str,
    ) -> Result<bool> {
        let conn = self.connection().context("failed to acquire connection")?;
        let now = now_ms();
        let (status, audit_val) = if approved {
            ("fresh", reviewer.to_string())
        } else {
            ("retired", reviewer.to_string())
        };
        let changed = conn
            .execute(|c| {
                let n = if approved {
                    c.execute(
                        "UPDATE explorations SET status = ?1, updated_at = ?2 \
                         WHERE exploration_id = ?3 AND status != 'retired'",
                        params![status, now, exploration_id],
                    )?
                } else {
                    c.execute(
                        "UPDATE explorations SET status = ?1, retired_at = ?2, \
                         retired_by = ?4, updated_at = ?2 \
                         WHERE exploration_id = ?3 AND status != 'retired'",
                        params![status, now, exploration_id, audit_val],
                    )?
                };
                Ok(n > 0)
            })
            .context("failed to review exploration")?;
        info!(exploration_id = %exploration_id, status, reviewer = %reviewer, "exploration reviewed");
        Ok(changed)
    }

    /// Move a just-persisted map into the `pending_review` queue (called by
    /// the server when `[exploration] require_review` is set and the
    /// submitter is not an admin). Returns `false` for unknown ids.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn mark_exploration_pending(&self, rowid: i64) -> Result<bool> {
        use crate::sqlite::explorations::STATUS_PENDING;
        let conn = self.connection().context("failed to acquire connection")?;
        let changed = conn
            .execute(|c| {
                let n = c.execute(
                    "UPDATE explorations SET status = ?1 WHERE id = ?2 AND status = 'fresh'",
                    params![STATUS_PENDING, rowid],
                )?;
                Ok(n > 0)
            })
            .context("failed to mark exploration pending")?;
        Ok(changed)
    }
}
