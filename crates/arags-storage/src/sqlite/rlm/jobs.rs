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
    /// Enqueue (or refresh) a job. Idempotent per `job_key`: an existing
    /// pending/claimed job is left untouched; a finished/cancelled one is
    /// reset to `pending` with `generation + 1` and elevated priority.
    /// Returns the job rowid and its current generation.
    ///
    /// # Errors
    ///
    /// Returns an error if the upsert fails.
    pub fn enqueue_rlm_job(&self, job: &NewRlmJob) -> Result<(i64, i64)> {
        let now = now_ms();
        let key = rlm_job_key(&job.project, job.level, &job.subject);
        let conn = self.connection().context("acquire connection")?;
        let inserted: Option<(i64, i64)> = conn.execute(|c| {
            c.query_row(
                "INSERT INTO rlm_jobs \
                 (job_key, buffer_id, project, level, subject, payload, status, priority, \
                  created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8, ?8) \
                 ON CONFLICT(job_key) DO UPDATE SET \
                   payload = excluded.payload, \
                   status = 'pending', \
                   priority = MIN(excluded.priority, rlm_jobs.priority), \
                   attempts = 0, \
                   last_error = NULL, \
                   generation = rlm_jobs.generation + 1, \
                   claimed_by = NULL, claimed_at = NULL, lease_expires_at = NULL, \
                   updated_at = excluded.updated_at \
                 WHERE rlm_jobs.status IN ('done','failed','cancelled') \
                 RETURNING id, generation",
                params![
                    key,
                    job.buffer_id,
                    job.project,
                    job.level,
                    job.subject,
                    job.payload,
                    job.priority,
                    now,
                ],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()
            .context("upsert rlm_job")
        })?;
        let (id, generation) = match inserted {
            Some(pair) => pair,
            None => {
                // Conflict hit but the reset WHERE filtered it out: an existing
                // pending/claimed job stays authoritative — fetch its identity.
                conn.execute(|c| {
                    c.query_row(
                        "SELECT id, generation FROM rlm_jobs WHERE job_key = ?1",
                        params![key],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                    )
                    .context("fetch existing rlm_job")
                })?
            }
        };
        Ok((id, generation))
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
                let key = rlm_job_key(project, *level, subject);
                let n = c
                    .execute(
                        "UPDATE rlm_jobs SET status = 'pending', priority = ?3, \
                           generation = generation + 1, attempts = 0, last_error = 'source changed', \
                           claimed_by = NULL, claimed_at = NULL, lease_expires_at = NULL, \
                           updated_at = ?2 \
                         WHERE job_key = ?1 AND status IN ('pending','claimed')",
                        params![key, now, PRIORITY_CANCELLED],
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

    /// Fetch a live (pending/claimed) job by its deterministic key.
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
             WHERE job_key = ?1 AND status IN ('pending','claimed')"
        );
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                sql.as_str(),
                params![rlm_job_key(project, level, subject)],
                job_mapper,
            )
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
