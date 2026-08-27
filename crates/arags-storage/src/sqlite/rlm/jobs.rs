//! Volunteer work queue: enqueue, claim with lease, complete (optionally
//! atomically persisting the node), fail, cancel and requeue.

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use rusqlite::params;

use super::super::conn::Storage;
use super::super::tokens::now_ms;
use super::{
    JOB_COLS, NewRlmJob, PRIORITY_CANCELLED, PRIORITY_PARKED, PRIORITY_RETRY, RlmJob, job_mapper,
    rlm_job_key,
};

impl Storage {
    /// Enqueue (or refresh) a job. Idempotent per `(project, level, subject)`:
    /// an existing pending/claimed job (or group of quorum slots) is left
    /// untouched; a finished/cancelled one is reset to `pending` with
    /// `generation + 1` and elevated priority. Returns the rowid of the first
    /// slot and its current generation.
    ///
    /// When `job.quorum_slots > 1` the subject is **fanned out** to that many
    /// independent physical job rows sharing a single `generation_group_id`.
    /// Each slot carries a distinct `job_key` (`<logical>#<slot>`) so they are
    /// independently claimable; a volunteer may claim at most one slot per
    /// group (enforced in [`Storage::claim_rlm_job`]). With `quorum_slots == 1`
    /// the classic single-row behaviour is preserved exactly (the `job_key`
    /// carries no slot suffix).
    ///
    /// `exclude` lists volunteers that must NOT be able to claim this group's
    /// slots (issue `agnostic-rlm-rs-f486`): it is written into
    /// `rlm_job_exclusions` keyed by the new `generation_group_id`. When
    /// non-empty, a brand-new generation group is always allocated (even for an
    /// existing subject) so a re-fan-out after total divergence starts clean.
    ///
    /// Returns `(rowid, generation, created_new)` where `created_new` is `true`
    /// only when a brand-new job row was inserted for this subject; an existing
    /// live job (or a reset of a finished one) yields `created_new = false`.
    /// Callers use this to report truthfully how much *new* work was enqueued
    /// (a repeated enqueue of an already-pending subject is not new work).
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    pub fn enqueue_rlm_job(&self, job: &NewRlmJob, exclude: &[String]) -> Result<(i64, i64, bool)> {
        let slots = job.quorum_slots.max(1);
        let logical = rlm_job_key(&job.project, job.level, &job.subject);
        let now = now_ms();
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            // Existing slots for this subject (all quorum rows share the
            // subject columns even though job_key is slot-suffixed).
            let existing: Vec<(i64, String, i64, Option<i64>)> = {
                let mut stmt = c
                    .prepare(
                        "SELECT id, status, generation, generation_group_id FROM rlm_jobs \
                          WHERE project = ?1 AND level = ?2 AND subject = ?3",
                    )
                    .context("prepare existing rlm_jobs probe")?;
                let rows = stmt
                    .query_map(params![job.project, job.level, job.subject], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, Option<i64>>(3)?,
                        ))
                    })
                    .context("query existing rlm_jobs")?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r.context("map existing rlm_job row")?);
                }
                out
            };
            if !existing.is_empty() {
                let live = existing
                    .iter()
                    .any(|(_, st, _, _)| st == "pending" || st == "claimed");
                if live {
                    // A live slot already owns the work — keep it authoritative.
                    // No new row was created; repeated enqueues are not new work.
                    return Ok((existing[0].0, existing[0].2, false));
                }
                // All slots finished/cancelled: reset, bump generation, recreate.
                let old_generation = existing[0].2;
                let group_id = if exclude.is_empty() {
                    existing[0].3.unwrap_or_else(|| existing[0].0)
                } else {
                    // Re-fan-out after divergence: fresh group so exclusions
                    // (and the generation counter used to cap rounds) advance.
                    c.query_row(
                        "SELECT COALESCE(MAX(generation_group_id), 0) + 1 FROM rlm_jobs",
                        [],
                        |r| r.get(0),
                    )
                    .context("allocate rlm generation group")?
                };
                c.execute(
                    "DELETE FROM rlm_jobs WHERE project = ?1 AND level = ?2 AND subject = ?3",
                    params![job.project, job.level, job.subject],
                )
                .context("reset rlm_job group")?;
                let new_generation = old_generation + 1;
                insert_rlm_exclusions(c, group_id, exclude).context("record rlm exclusions")?;
                let first =
                    insert_rlm_slots(c, job, slots, &logical, group_id, new_generation, now)
                        .context("recreate rlm_job slots")?;
                // Reset of a finished/cancelled job: not brand-new work.
                return Ok((first, new_generation, false));
            }
            // Fresh subject: allocate a new generation group and create slots.
            let group_id: i64 = c
                .query_row(
                    "SELECT COALESCE(MAX(generation_group_id), 0) + 1 FROM rlm_jobs",
                    [],
                    |r| r.get(0),
                )
                .context("allocate rlm generation group")?;
            // A re-fan-out (exclude non-empty) advances the generation counter
            // so the reassignment round can be capped by the caller.
            let prior_gen: i64 = c
                .query_row(
                    "SELECT COALESCE(MAX(generation), 0) FROM rlm_jobs \
                     WHERE project = ?1 AND level = ?2 AND subject = ?3",
                    params![job.project, job.level, job.subject],
                    |r| r.get(0),
                )
                .context("read prior rlm generation")?;
            let generation = if exclude.is_empty() { 0 } else { prior_gen + 1 };
            insert_rlm_exclusions(c, group_id, exclude).context("record rlm exclusions")?;
            let first = insert_rlm_slots(c, job, slots, &logical, group_id, generation, now)
                .context("create rlm_job slots")?;
            // Fresh subject: a genuinely new job row was created.
            Ok((first, generation, true))
        })
    }

    /// Report a failed attempt: the job returns to `pending` for retry unless
    /// it exceeded `max_attempts`, then it is marked `failed`.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn fail_rlm_job(
        &self,
        job_id: i64,
        worker: &str,
        error: &str,
        max_attempts: i64,
    ) -> Result<()> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let attempts: i64 = c
                .query_row(
                    "SELECT attempts FROM rlm_jobs WHERE id = ?1 AND claimed_by = ?2",
                    params![job_id, worker],
                    |r| r.get(0),
                )
                .optional()
                .context("probe failed rlm_job")?
                .unwrap_or_default();
            let (status, prio) = if attempts >= max_attempts {
                ("failed", PRIORITY_PARKED)
            } else {
                ("pending", PRIORITY_RETRY) // retry soon, slightly elevated
            };
            c.execute(
                "UPDATE rlm_jobs SET status = ?3, priority = ?4, last_error = ?5, \
                   claimed_by = NULL, claimed_at = NULL, lease_expires_at = NULL, updated_at = ?6 \
                 WHERE id = ?1 AND claimed_by = ?2 AND status = 'claimed'",
                params![job_id, worker, status, prio, error, now_ms()],
            )
            .context("fail rlm_job")?;
            Ok(())
        })?;
        Ok(())
    }

    /// Invalidate jobs covering the given `(level, subject)` pairs because
    /// their source data changed. The job row itself is reset to `pending`
    /// with front-of-queue priority and `generation + 1`: a volunteer still
    /// holding the old lease detects the cancellation via the generation
    /// mismatch on completion and discards its work. Returns how many live
    /// jobs were reset.
    ///
    /// # Errors
    ///
    /// Returns an error if any statement fails.
    pub fn cancel_rlm_jobs_for_subjects(
        &self,
        project: &str,
        subjects: &[(i64, String)],
    ) -> Result<usize> {
        if subjects.is_empty() {
            return Ok(0);
        }
        let now = now_ms();
        let mut cancelled = 0;
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            for (level, subject) in subjects {
                let n = c
                    .execute(
                        "UPDATE rlm_jobs SET status = 'pending', priority = ?3, \
                           generation = generation + 1, attempts = 0, last_error = 'source changed', \
                           claimed_by = NULL, claimed_at = NULL, lease_expires_at = NULL, \
                           updated_at = ?2 \
                         WHERE project = ?1 AND level = ?4 AND subject = ?5 \
                           AND status IN ('pending','claimed')",
                        params![project, now, PRIORITY_CANCELLED, *level, subject],
                    )
                    .context("reset rlm_job for changed source")?;
                cancelled += n;
            }
            Ok(())
        })?;
        if cancelled > 0 {
            tracing::info!(project, cancelled, "cancelled rlm jobs after source change");
        }
        Ok(cancelled)
    }

    /// Requeue claimed jobs whose lease expired without completion. Called by
    /// the maintenance loop so crashed volunteers do not strand work units.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn requeue_expired_rlm_leases(&self) -> Result<usize> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let n = c
                .execute(
                    "UPDATE rlm_jobs SET status = 'pending', claimed_by = NULL, \
                       claimed_at = NULL, lease_expires_at = NULL, updated_at = ?1 \
                     WHERE status = 'claimed' AND lease_expires_at < ?1",
                    params![now_ms()],
                )
                .context("requeue expired rlm leases")?;
            Ok(n)
        })
    }

    /// Overwrite the payload of a live job (motor refresh after source
    /// changes). No-op when the job is finished/cancelled.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn update_rlm_job_payload(
        &self,
        project: &str,
        level: i64,
        subject: &str,
        payload: &str,
    ) -> Result<()> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.execute(
                "UPDATE rlm_jobs SET payload = ?2, updated_at = ?3 \
                 WHERE job_key = ?1 AND status IN ('pending','claimed')",
                params![rlm_job_key(project, level, subject), payload, now_ms()],
            )
            .context("update_rlm_job_payload")
        })?;
        Ok(())
    }

    /// Fetch a live (pending/claimed) job for a subject. With quorum fan-out
    /// several slots may share the subject; this returns the first live one
    /// (callers only need to know whether live work exists).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_live_rlm_job_by_key(
        &self,
        project: &str,
        level: i64,
        subject: &str,
    ) -> Result<Option<RlmJob>> {
        let sql = format!(
            "SELECT {JOB_COLS} FROM rlm_jobs \
             WHERE project = ?1 AND level = ?2 AND subject = ?3 \
               AND status IN ('pending','claimed')"
        );
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.query_row(sql.as_str(), params![project, level, subject], job_mapper)
                .optional()
                .context("get_live_rlm_job_by_key")
        })
    }

    /// Fetch a single job by rowid.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_rlm_job(&self, job_id: i64) -> Result<Option<RlmJob>> {
        let sql = format!("SELECT {JOB_COLS} FROM rlm_jobs WHERE id = ?1");
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.query_row(sql.as_str(), params![job_id], job_mapper)
                .optional()
                .context("get rlm_job")
        })
    }

    /// Count jobs by status for a project (ops/monitoring).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn count_rlm_jobs(&self, project: &str, status: &str) -> Result<usize> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let n: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM rlm_jobs WHERE project = ?1 AND status = ?2",
                    params![project, status],
                    |r| r.get(0),
                )
                .context("count rlm_jobs")?;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            // COUNT(*) is small/non-negative
            Ok(n as usize)
        })
    }
}

/// Insert the `quorum_slots` physical job rows for a subject, sharing one
/// `generation_group_id`. Slot `0` keeps the bare logical `job_key`; slots
/// `1..` append a `#<slot>` suffix so the `job_key` UNIQUE constraint is never
/// violated. Returns the rowid of the first (slot 0) row.
///
/// # Errors
///
/// Returns an error if any insert fails.
fn insert_rlm_slots(
    c: &rusqlite::Connection,
    job: &NewRlmJob,
    slots: usize,
    logical: &str,
    group_id: i64,
    generation: i64,
    now: i64,
) -> rusqlite::Result<i64> {
    let mut first_id = 0i64;
    for slot in 0..slots {
        let key = if slots == 1 {
            logical.to_string()
        } else {
            format!("{logical}#{slot}")
        };
        let id: i64 = c.query_row(
            "INSERT INTO rlm_jobs \
                 (job_key, buffer_id, project, level, subject, payload, status, priority, \
                  generation, generation_group_id, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8, ?9, ?10, ?10) \
                 RETURNING id",
            params![
                key,
                job.buffer_id,
                job.project,
                job.level,
                job.subject,
                job.payload,
                job.priority,
                generation,
                group_id,
                now,
            ],
            |r| r.get(0),
        )?;
        if first_id == 0 {
            first_id = id;
        }
    }
    Ok(first_id)
}

/// Write `exclude` volunteers into `rlm_job_exclusions` for `group_id` so they
/// cannot claim the freshly fanned-out slots. Existing rows are ignored via
/// `ON CONFLICT DO NOTHING` (a volunteer may be excluded by several rounds).
///
/// # Errors
///
/// Returns an error if any insert fails.
fn insert_rlm_exclusions(
    c: &rusqlite::Connection,
    group_id: i64,
    exclude: &[String],
) -> rusqlite::Result<()> {
    for vol in exclude {
        if vol.is_empty() {
            continue;
        }
        c.execute(
            "INSERT INTO rlm_job_exclusions (generation_group_id, volunteer) \
             VALUES (?1, ?2) ON CONFLICT(generation_group_id, volunteer) DO NOTHING",
            params![group_id, vol],
        )?;
    }
    Ok(())
}
