//! Re-digest queue for stale QA answers (issue `agnostic-rag-rlm-tool-d172`,
//! plan `pl-783b` step 4).
//!
//! When a `qa_cache` row is marked stale (source chunk changed) the staleness
//! hook enqueues a `pending_qa_jobs` row. A volunteer client claims a lease
//! (preferring the original author via `preferred_user`, else any volunteer),
//! re-digests the answer locally with its own LLM, persists it through the
//! existing `StoreAnswer` RPC, then completes the job. A lease that is not
//! completed within `lease_secs` (default 300s) is reverted to `pending` by the
//! maintenance ticker so a crashed volunteer never strands the work unit.
//!
//! All access goes through [`super::conn::Storage::connection`], which is safe
//! in both single (CLI) and pooled (server) modes.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::OptionalExtension;
use rusqlite::params;
use std::time::Instant;
use tracing::debug;

use super::conn::Storage;

/// Default lease duration for a claimed re-digest job, in seconds.
pub const DEFAULT_PENDING_QA_LEASE_SECS: i64 = 300;

/// A pending QA re-digest job as stored.
#[derive(Debug, Clone)]
pub struct PendingQaJob {
    /// Numeric rowid.
    pub id: i64,
    /// Stable `qa_cache` answer id this job re-digests.
    pub cache_id: String,
    /// Project the answer belongs to.
    pub project: String,
    /// Original author; tried first when claiming.
    pub preferred_user: Option<String>,
    /// Lifecycle status: `pending` | `leased` | `completed`.
    pub status: String,
    /// Volunteer that currently holds the lease.
    pub leased_by: Option<String>,
    /// Lease expiry epoch seconds (NULL until leased).
    pub leased_until: Option<i64>,
}

/// Counts of pending QA jobs by status (ops/monitoring + maintenance logging).
#[derive(Debug, Clone, Default)]
pub struct PendingQaCounts {
    /// Jobs awaiting a volunteer.
    pub pending: u64,
    /// Jobs currently leased.
    pub leased: u64,
    /// Jobs completed this cycle.
    pub completed: u64,
    /// Jobs reverted from `leased` to `pending` by the last reclaim pass.
    pub expired: u64,
}

impl Storage {
    /// Enqueue a re-digest job for a stale QA answer. Idempotent: if an open
    /// (`pending`/`leased`) job already exists for `cache_id`, its id is
    /// returned and no duplicate is created.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    pub fn enqueue_pending_qa(
        &self,
        cache_id: &str,
        project: &str,
        preferred_user: Option<&str>,
    ) -> Result<i64> {
        let now = now_secs();
        let conn = self.connection().context("acquire connection")?;
        let start = Instant::now();
        let id: i64 = conn
            .execute(|c| {
                let open: Option<i64> = c
                    .query_row(
                        "SELECT id FROM pending_qa_jobs \
                         WHERE cache_id = ?1 AND status IN ('pending','leased') LIMIT 1",
                        params![cache_id],
                        |r| r.get(0),
                    )
                    .optional()
                    .context("probe open pending qa job")?;
                if let Some(id) = open {
                    return Ok(id);
                }
                let id: i64 = c
                    .query_row(
                        "INSERT INTO pending_qa_jobs \
                         (cache_id, project, preferred_user, status, created_at) \
                         VALUES (?1, ?2, ?3, 'pending', ?4) RETURNING id",
                        params![cache_id, project, preferred_user, now],
                        |r| r.get(0),
                    )
                    .context("insert pending qa job")?;
                Ok(id)
            })
            .context("execute enqueue pending qa")?;
        debug!(duration_ms = %start.elapsed().as_millis(), cache_id, "enqueued pending qa job");
        Ok(id)
    }

    /// Enqueue a re-digest job from a stale `qa_cache` row id, using its
    /// `cache_id`/`project` and original author (`created_by`) as the preferred
    /// volunteer. No-op when the row is missing.
    ///
    /// # Errors
    ///
    /// Returns an error if the lookup or enqueue fails.
    pub fn enqueue_pending_qa_for_stale(&self, qa_id: i64) -> Result<()> {
        let Some(row) = self.get_qa_by_rowid(qa_id)? else {
            return Ok(());
        };
        self.enqueue_pending_qa(&row.cache_id, &row.project, row.created_by.as_deref())?;
        Ok(())
    }

    /// Atomically claim the next `pending` job for a volunteer. The original
    /// author (`preferred_user = worker_user`) is preferred; otherwise the
    /// oldest pending job is taken. On success the job is `leased` to
    /// `worker_user` until `now + lease_secs`.
    ///
    /// # Errors
    ///
    /// Returns an error if the transaction fails.
    pub fn claim_pending_qa(
        &self,
        worker_user: &str,
        lease_secs: i64,
    ) -> Result<Option<PendingQaJob>> {
        let now = now_secs();
        let expires = now.saturating_add(lease_secs.max(1));
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let tx = c.unchecked_transaction().context("begin claim tx")?;
            // Prefer a job whose original author matches this volunteer.
            let preferred: Option<i64> = tx
                .query_row(
                    "SELECT id FROM pending_qa_jobs \
                     WHERE status = 'pending' AND preferred_user = ?1 \
                     ORDER BY created_at ASC LIMIT 1",
                    params![worker_user],
                    |r| r.get(0),
                )
                .optional()
                .context("select preferred pending qa job")?;
            let job_id = match preferred {
                Some(id) => Some(id),
                None => tx
                    .query_row(
                        "SELECT id FROM pending_qa_jobs \
                         WHERE status = 'pending' ORDER BY created_at ASC LIMIT 1",
                        [],
                        |r| r.get(0),
                    )
                    .optional()
                    .context("select pending qa job")?,
            };
            let Some(job_id) = job_id else {
                let _ = tx.finish();
                return Ok(None);
            };
            tx.execute(
                "UPDATE pending_qa_jobs SET status = 'leased', leased_by = ?1, \
                 leased_until = ?2 WHERE id = ?3 AND status = 'pending'",
                params![worker_user, expires, job_id],
            )
            .context("claim pending qa job")?;
            tx.commit().context("commit claim tx")?;
            let job = c
                .query_row(
                    "SELECT id, cache_id, project, preferred_user, status, leased_by, \
                     leased_until FROM pending_qa_jobs WHERE id = ?1",
                    params![job_id],
                    pending_qa_mapper,
                )
                .context("reload claimed qa job")?;
            Ok(Some(job))
        })
    }

    /// Revert `leased` jobs whose lease expired (`leased_until < now`) back to
    /// `pending`, freeing them for the next cycle.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn revert_expired_leases(&self, now: i64) -> Result<usize> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let n = c
                .execute(
                    "UPDATE pending_qa_jobs SET status = 'pending', leased_by = NULL, \
                     leased_until = NULL WHERE status = 'leased' AND leased_until < ?1",
                    params![now],
                )
                .context("revert expired pending qa leases")?;
            Ok(n)
        })
    }

    /// Revert expired leases and report job counts by status for observability.
    ///
    /// # Errors
    ///
    /// Returns an error if any statement fails.
    pub fn revert_expired_pending_qa(&self, now: i64) -> Result<PendingQaCounts> {
        let expired = self.revert_expired_leases(now)? as u64;
        let (pending, leased, completed) = self.count_pending_qa_by_status()?;
        Ok(PendingQaCounts {
            pending,
            leased,
            completed,
            expired,
        })
    }

    /// Mark a `leased` job `completed` (called after `StoreAnswer` succeeds).
    /// Returns `false` if the job was not `leased` (e.g. lease expired).
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn complete_pending_qa(&self, job_id: i64) -> Result<bool> {
        let now = now_secs();
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let n = c
                .execute(
                    "UPDATE pending_qa_jobs SET status = 'completed', completed_at = ?1 \
                     WHERE id = ?2 AND status = 'leased'",
                    params![now, job_id],
                )
                .context("complete pending qa job")?;
            Ok(n > 0)
        })
    }

    /// Count jobs by status: `(pending, leased, completed)`.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    fn count_pending_qa_by_status(&self) -> Result<(u64, u64, u64)> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let (pending, leased, completed): (i64, i64, i64) = c
                .query_row(
                    "SELECT \
                       COALESCE(SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END), 0), \
                       COALESCE(SUM(CASE WHEN status = 'leased' THEN 1 ELSE 0 END), 0), \
                       COALESCE(SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END), 0) \
                     FROM pending_qa_jobs",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .context("count pending qa jobs by status")?;
            #[allow(clippy::cast_sign_loss)]
            Ok((pending as u64, leased as u64, completed as u64))
        })
    }
}

/// Map a `pending_qa_jobs` row to [`PendingQaJob`].
fn pending_qa_mapper(r: &rusqlite::Row<'_>) -> rusqlite::Result<PendingQaJob> {
    Ok(PendingQaJob {
        id: r.get(0)?,
        cache_id: r.get(1)?,
        project: r.get(2)?,
        preferred_user: r.get(3)?,
        status: r.get(4)?,
        leased_by: r.get(5)?,
        leased_until: r.get(6)?,
    })
}

/// Current epoch seconds (UTC).
#[must_use]
fn now_secs() -> i64 {
    Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::cast_possible_truncation
    )]

    use super::*;
    use crate::sqlite::conn::Storage;
    use tempfile::TempDir;

    fn open() -> (TempDir, Storage) {
        let dir = TempDir::new().unwrap();
        let storage = Storage::open(dir.path()).unwrap();
        (dir, storage)
    }

    #[test]
    fn enqueue_pending_qa_is_idempotent() {
        let (_dir, storage) = open();
        let first = storage
            .enqueue_pending_qa("c1", "proj", Some("alice"))
            .unwrap();
        let second = storage
            .enqueue_pending_qa("c1", "proj", Some("alice"))
            .unwrap();
        assert_eq!(first, second, "re-enqueue must return the same open job");
        // Exactly one open job exists for this cache_id.
        let conn = storage.connection().unwrap();
        let n: i64 = conn
            .execute(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM pending_qa_jobs \
                     WHERE cache_id = 'c1' AND status IN ('pending','leased')",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn claim_pending_qa_prefers_preferred_user() {
        let (_dir, storage) = open();
        // alice is the original author (preferred).
        storage
            .enqueue_pending_qa("c1", "proj", Some("alice"))
            .unwrap();
        storage
            .enqueue_pending_qa("c2", "proj", Some("bob"))
            .unwrap();

        // bob claims first: he should NOT get alice's job.
        let bob_job = storage
            .claim_pending_qa("bob", DEFAULT_PENDING_QA_LEASE_SECS)
            .unwrap();
        assert!(bob_job.is_some());
        let bob_job = bob_job.unwrap();
        assert_eq!(bob_job.cache_id, "c2");
        assert_eq!(bob_job.leased_by.as_deref(), Some("bob"));

        // alice now claims: she should get her own preferred job (c1).
        let alice_job = storage
            .claim_pending_qa("alice", DEFAULT_PENDING_QA_LEASE_SECS)
            .unwrap();
        let alice_job = alice_job.unwrap();
        assert_eq!(alice_job.cache_id, "c1");
        assert_eq!(alice_job.leased_by.as_deref(), Some("alice"));
    }

    #[test]
    fn claim_pending_qa_lease_expires_and_requeues() {
        let (_dir, storage) = open();
        storage
            .enqueue_pending_qa("c1", "proj", Some("alice"))
            .unwrap();

        let claimed = storage.claim_pending_qa("bob", 300).unwrap().unwrap();
        assert_eq!(claimed.status, "leased");

        // Before expiry, bob still holds it.
        let now = now_secs();
        assert_eq!(storage.revert_expired_leases(now).unwrap(), 0);

        // After expiry, it returns to pending and another worker can claim.
        let later = now + 1000;
        assert_eq!(storage.revert_expired_leases(later).unwrap(), 1);
        let reclaimed = storage.claim_pending_qa("carol", 300).unwrap().unwrap();
        assert_eq!(reclaimed.cache_id, "c1");
        assert_eq!(reclaimed.leased_by.as_deref(), Some("carol"));
    }

    #[test]
    fn pending_qa_lifecycle_claim_store_complete() {
        let (_dir, storage) = open();
        storage
            .enqueue_pending_qa("c1", "proj", Some("alice"))
            .unwrap();

        // Volunteer claims.
        let job = storage.claim_pending_qa("bob", 300).unwrap().unwrap();
        assert_eq!(job.status, "leased");

        // Volunteer digests and stores a fresh answer (supersedes prior active).
        let input = crate::sqlite::qa_cache::StoreAnswerInput {
            buffer_id: None,
            project: "proj".to_string(),
            question_text: "what is x?".to_string(),
            question_hash: "h1".to_string(),
            answer_text: "fresh answer".to_string(),
            source_chunk_ids: vec!["1".to_string()],
            source_hashes: vec!["hsh".to_string()],
            model: Some("local".to_string()),
            tier_snapshot: None,
            token_count: 7,
            created_by: Some("bob".to_string()),
        };
        let stored = storage.store_answer(&input).unwrap();
        assert!(!stored.cache_id.is_empty());

        // Complete the job.
        assert!(storage.complete_pending_qa(job.id).unwrap());
        let conn = storage.connection().unwrap();
        let status: String = conn
            .execute(|c| {
                Ok(c.query_row(
                    "SELECT status FROM pending_qa_jobs WHERE id = ?1",
                    params![job.id],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(status, "completed");
    }

    #[test]
    fn enqueue_pending_qa_for_stale_autofills_author() {
        let (_dir, storage) = open();
        // Seed a qa_cache row owned by alice.
        storage
            .connection()
            .unwrap()
            .execute(|c| {
                c.execute(
                    "INSERT INTO qa_cache \
                     (id, cache_id, project, question_text, question_hash, answer_text, \
                      created_by, is_active, stale, created_at, last_accessed_at) \
                     VALUES (1, 'c1', 'proj', 'q', 'h', 'a', 'alice', 1, 1, 0, 0)",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        storage.enqueue_pending_qa_for_stale(1).unwrap();
        let conn = storage.connection().unwrap();
        let pref: Option<String> = conn
            .execute(|c| {
                Ok(c.query_row(
                    "SELECT preferred_user FROM pending_qa_jobs WHERE cache_id = 'c1'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(pref.as_deref(), Some("alice"));
    }
}
