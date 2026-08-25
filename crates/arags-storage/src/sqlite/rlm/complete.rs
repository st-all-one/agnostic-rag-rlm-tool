//! Completion paths for claimed RLM jobs.
//!
//! Two entry points: bare completion (flip to `done`) and the atomic
//! complete+persist used by the server so volunteer work can never be lost
//! by a half-applied submission.

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use rusqlite::params;

use super::super::conn::Storage;
use super::super::tokens::now_ms;
use super::{ClaimedRlmJob, JOB_COLS, NewRlmNode, job_mapper};

impl Storage {
    /// Complete a claimed job. Rejects the result if the lease expired, the
    /// claimant differs, or the job was cancelled/re-enqueued meanwhile
    /// (`generation` mismatch) — the caller should discard its work.
    ///
    /// Prefer [`Storage::complete_rlm_job_with_node`] when a summary node was
    /// produced: it persists both writes atomically.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn complete_rlm_job(&self, job_id: i64, worker: &str, generation: i64) -> Result<bool> {
        let conn = self.connection().context("acquire connection")?;
        let ok = conn.execute(|c| {
            let current: Option<(Option<String>, Option<i64>, i64)> = c
                .query_row(
                    "SELECT claimed_by, lease_expires_at, generation FROM rlm_jobs \
                     WHERE id = ?1 AND status = 'claimed'",
                    params![job_id],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<i64>>(1)?,
                            r.get::<_, i64>(2)?,
                        ))
                    },
                )
                .optional()
                .context("probe claimed rlm_job")?;
            let Some((by, lease, stored_gen)) = current else {
                return Ok(false);
            };
            let now = now_ms();
            let valid = by.as_deref() == Some(worker)
                && stored_gen == generation
                && lease.is_some_and(|l| l >= now);
            if !valid {
                return Ok(false);
            }
            let n = c
                .execute(
                    "UPDATE rlm_jobs SET status = 'done', updated_at = ?1 WHERE id = ?2",
                    params![now, job_id],
                )
                .context("complete rlm_job")?;
            Ok(n > 0)
        })?;
        if ok {
            tracing::info!(job_id, worker, "rlm job completed");
        } else {
            tracing::warn!(
                job_id,
                worker,
                "rlm job completion rejected (stale lease/generation)"
            );
        }
        Ok(ok)
    }

    /// Atomically complete a claimed job **and** persist its summary node.
    ///
    /// Lease validation (owner, expiry, generation) and both writes happen in
    /// a single transaction: if the node insert fails the transaction rolls
    /// back and the job stays `claimed` for retry/requeue, so volunteer work
    /// can never be lost by a half-applied completion. Returns
    /// `Ok(None)` when the caller no longer owns the job — nothing is written.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or any statement fails.
    pub fn complete_rlm_job_with_node(
        &self,
        job_id: i64,
        worker: &str,
        generation: i64,
        input: &NewRlmNode,
    ) -> Result<Option<(i64, String)>> {
        let conn = self.connection().context("acquire connection")?;
        let outcome = conn.execute(|c| {
            let tx = c.unchecked_transaction().context("begin complete tx")?;
            let current: Option<(Option<String>, Option<i64>, i64)> = tx
                .query_row(
                    "SELECT claimed_by, lease_expires_at, generation FROM rlm_jobs \
                     WHERE id = ?1 AND status = 'claimed'",
                    params![job_id],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<i64>>(1)?,
                            r.get::<_, i64>(2)?,
                        ))
                    },
                )
                .optional()
                .context("probe claimed rlm_job")?;
            let Some((by, lease, stored_gen)) = current else {
                let _ = tx.finish();
                return Ok(None);
            };
            let valid = by.as_deref() == Some(worker)
                && stored_gen == generation
                && lease.is_some_and(|l| l >= now_ms());
            if !valid {
                let _ = tx.finish();
                return Ok(None);
            }
            // Node first: on failure the rollback keeps the job claimable.
            let now = now_ms();
            let fresh_node_id = uuid::Uuid::now_v7().to_string();
            let hashes_json =
                serde_json::to_string(&input.source_hashes).context("serialize source_hashes")?;
            let pair = super::upsert_node_stmt(&tx, &fresh_node_id, &hashes_json, input, now)
                .context("upsert rlm_node")?;
            tx.execute(
                "UPDATE rlm_jobs SET status = 'done', updated_at = ?1 WHERE id = ?2",
                params![now, job_id],
            )
            .context("complete rlm_job")?;
            tx.commit().context("commit complete tx")?;
            Ok(Some(pair))
        })?;
        if let Some((rowid, node_id)) = &outcome {
            tracing::info!(job_id, worker, rowid, node_id = %node_id, "rlm job completed");
        } else {
            tracing::warn!(
                job_id,
                worker,
                "rlm job completion rejected (stale lease/generation)"
            );
        }
        Ok(outcome)
    }
}

impl Storage {
    /// Atomically claim the next pending job for a volunteer. The lease is
    /// client-supplied (default 500s, `DEFAULT_RLM_LEASE_MS`); while the
    /// lease is valid no other volunteer can claim the same work unit.
    ///
    /// # Errors
    ///
    /// Returns an error if the transaction fails.
    pub fn claim_rlm_job(
        &self,
        volunteer: &str,
        lease_ms: i64,
        max_level: Option<i64>,
    ) -> Result<Option<ClaimedRlmJob>> {
        let now = now_ms();
        let expires = now.saturating_add(lease_ms.max(1_000));
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let tx = c.unchecked_transaction().context("begin claim tx")?;
            // Volunteers may cap the level they accept (quota); `i64::MAX`
            // accepts everything.
            let sql = "SELECT id FROM rlm_jobs WHERE status = 'pending' AND level <= ?1 \
                 ORDER BY priority ASC, level ASC, created_at ASC LIMIT 1";
            let job_id: Option<i64> = tx
                .query_row(sql, params![max_level.unwrap_or(i64::MAX)], |r| r.get(0))
                .optional()
                .context("select candidate rlm_job")?;
            let Some(job_id) = job_id else {
                let _ = tx.finish();
                return Ok(None);
            };
            tx.execute(
                "UPDATE rlm_jobs SET status = 'claimed', claimed_by = ?1, claimed_at = ?2, \
                   lease_expires_at = ?3, attempts = attempts + 1, updated_at = ?2 \
                 WHERE id = ?4 AND status = 'pending'",
                params![volunteer, now, expires, job_id],
            )
            .context("claim rlm_job")?;
            tx.commit().context("commit claim tx")?;
            let sql = format!("SELECT {JOB_COLS} FROM rlm_jobs WHERE id = ?1");
            let job = c
                .query_row(sql.as_str(), params![job_id], job_mapper)
                .context("reload claimed rlm_job")?;
            Ok(Some(ClaimedRlmJob {
                id: job.id,
                job_key: job.job_key,
                project: job.project,
                level: job.level,
                subject: job.subject,
                payload: job.payload,
                generation: job.generation,
                lease_ms: lease_ms.max(1_000),
            }))
        })
    }
}
