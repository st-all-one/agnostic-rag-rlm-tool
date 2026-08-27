//! Candidate submissions + volunteer trust (Cluster B keystone, issue
//! `agnostic-rlm-rs-a5d7`).
//!
//! When a volunteer produces an answer for a subject (an RLM node, an
//! exploration map, or a QA), it is stored as a `candidate` [`Submission`]. The
//! quorum decision logic (issues `6d97`/`64af`) later inspects the pending
//! candidates for a subject, computes pairwise cosine similarity, and either
//! accepts the consensus (`accept_submission`) or rejects dissenting/rejected
//! candidates (`reject_submission`), recording the `similarity` used in the
//! decision. Volunteers that accrue `strikes_limit` strikes are
//! deprioritized/banned via the [`volunteer_trust`] table.
//!
//! All access goes through [`super::conn::Storage::connection`], safe in both
//! single (CLI) and pooled (server) modes.

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use rusqlite::params;

use super::conn::Storage;

/// A candidate submission as stored in the `submissions` table.
#[derive(Debug, Clone)]
pub struct Submission {
    /// Numeric rowid.
    pub id: i64,
    /// Project the submission belongs to.
    pub project: String,
    /// Subject kind: `rlm_node` | `exploration` | `qa`.
    pub subject_type: String,
    /// Subject id/key the submission targets.
    pub subject_key: String,
    /// Candidate answer text.
    pub candidate_text: String,
    /// Volunteer username (from auth refresh token).
    pub candidate_by: String,
    /// Cosine similarity to the accepted/consensus candidate (filled on
    /// decision; `None` while still `candidate`).
    pub similarity: Option<f64>,
    /// Lifecycle status: `candidate` | `accepted` | `rejected`.
    pub status: String,
    /// Epoch seconds the row was created.
    pub created_at: i64,
    /// Epoch seconds of the accept/reject decision (`None` while pending).
    pub decided_at: Option<i64>,
    /// Volunteer username that made the decision (`None` while pending).
    pub decided_by: Option<String>,
}

impl Storage {
    /// Insert a new candidate submission. Its status defaults to `candidate`.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    pub fn insert_submission(
        &self,
        project: &str,
        subject_type: &str,
        subject_key: &str,
        candidate_text: &str,
        candidate_by: &str,
    ) -> Result<i64> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let id: i64 = c
                .query_row(
                    "INSERT INTO submissions \
                     (project, subject_type, subject_key, candidate_text, candidate_by) \
                     VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
                    params![
                        project,
                        subject_type,
                        subject_key,
                        candidate_text,
                        candidate_by
                    ],
                    |r| r.get(0),
                )
                .context("insert submission")?;
            Ok(id)
        })
    }

    /// Accept a candidate submission: status becomes `accepted`, `decided_at`
    /// is set to now, and the optional `similarity` (to consensus) is recorded.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn accept_submission(
        &self,
        id: i64,
        decided_by: &str,
        similarity: Option<f64>,
    ) -> Result<()> {
        let now = now_secs();
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.execute(
                "UPDATE submissions SET status = 'accepted', decided_at = ?1, \
                 decided_by = ?2, similarity = ?3 WHERE id = ?4",
                params![now, decided_by, similarity, id],
            )
            .context("accept submission")?;
            Ok(())
        })
    }

    /// Reject a candidate submission: status becomes `rejected`, `decided_at`
    /// is set to now, `decided_by` is recorded, and the `similarity` (to the
    /// accepted/consensus candidate) is stored for audit.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn reject_submission(
        &self,
        id: i64,
        decided_by: &str,
        similarity: Option<f64>,
    ) -> Result<()> {
        let now = now_secs();
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.execute(
                "UPDATE submissions SET status = 'rejected', decided_at = ?1, \
                 decided_by = ?2, similarity = ?3 WHERE id = ?4",
                params![now, decided_by, similarity, id],
            )
            .context("reject submission")?;
            Ok(())
        })
    }

    /// List the still-`candidate` submissions for a subject (project + type +
    /// key). Used by the quorum decision logic to gather the agreeing set.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn list_pending(
        &self,
        project: &str,
        subject_type: &str,
        subject_key: &str,
    ) -> Result<Vec<Submission>> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT id, project, subject_type, subject_key, candidate_text, \
                     candidate_by, similarity, status, created_at, decided_at, decided_by \
                     FROM submissions \
                     WHERE project = ?1 AND subject_type = ?2 AND subject_key = ?3 \
                     AND status = 'candidate' ORDER BY created_at ASC",
                )
                .context("prepare list pending submissions")?;
            let rows = stmt
                .query_map(
                    params![project, subject_type, subject_key],
                    submission_mapper,
                )
                .context("query pending submissions")?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.context("map pending submission row")?);
            }
            Ok(out)
        })
    }

    /// List the `accepted` submissions for a subject (project + type + key).
    /// Used by the quorum decision worker for idempotency: once a subject's
    /// consensus has been published, later ticks must not re-decide it.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn list_accepted(
        &self,
        project: &str,
        subject_type: &str,
        subject_key: &str,
    ) -> Result<Vec<Submission>> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT id, project, subject_type, subject_key, candidate_text, \
                     candidate_by, similarity, status, created_at, decided_at, decided_by \
                     FROM submissions \
                     WHERE project = ?1 AND subject_type = ?2 AND subject_key = ?3 \
                     AND status = 'accepted' ORDER BY created_at ASC",
                )
                .context("prepare list accepted submissions")?;
            let rows = stmt
                .query_map(
                    params![project, subject_type, subject_key],
                    submission_mapper,
                )
                .context("query accepted submissions")?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.context("map accepted submission row")?);
            }
            Ok(out)
        })
    }

    /// Record a strike against a volunteer, returning the new strike count. The
    /// `volunteer_trust` row is created on first contact with `strikes = 0`.
    ///
    /// # Errors
    ///
    /// Returns an error if the upsert or increment fails.
    #[allow(clippy::cast_possible_truncation)]
    pub fn record_strike(&self, volunteer: &str) -> Result<u32> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.execute(
                "INSERT INTO volunteer_trust (username, strikes, trust_score) \
                 VALUES (?1, 0, 1.0) ON CONFLICT(username) DO NOTHING",
                params![volunteer],
            )
            .context("seed volunteer trust")?;
            c.execute(
                "UPDATE volunteer_trust SET strikes = strikes + 1 WHERE username = ?1",
                params![volunteer],
            )
            .context("increment volunteer strike")?;
            let strikes: i64 = c
                .query_row(
                    "SELECT strikes FROM volunteer_trust WHERE username = ?1",
                    params![volunteer],
                    |r| r.get(0),
                )
                .context("read volunteer strikes")?;
            #[allow(clippy::cast_sign_loss)]
            Ok(strikes as u32)
        })
    }

    /// Read a volunteer's current strike count (`0` if never seen).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    #[allow(clippy::cast_possible_truncation)]
    pub fn volunteer_strikes(&self, volunteer: &str) -> Result<u32> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let strikes: Option<i64> = c
                .query_row(
                    "SELECT strikes FROM volunteer_trust WHERE username = ?1",
                    params![volunteer],
                    |r| r.get(0),
                )
                .optional()
                .context("read volunteer strikes")?;
            #[allow(clippy::cast_sign_loss)]
            Ok(strikes.unwrap_or(0) as u32)
        })
    }
}

/// Map a `submissions` row to [`Submission`].
fn submission_mapper(r: &rusqlite::Row<'_>) -> rusqlite::Result<Submission> {
    Ok(Submission {
        id: r.get(0)?,
        project: r.get(1)?,
        subject_type: r.get(2)?,
        subject_key: r.get(3)?,
        candidate_text: r.get(4)?,
        candidate_by: r.get(5)?,
        similarity: r.get(6)?,
        status: r.get(7)?,
        created_at: r.get(8)?,
        decided_at: r.get(9)?,
        decided_by: r.get(10)?,
    })
}

/// Current epoch seconds (UTC).
#[must_use]
fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
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
    fn submissions_insert_and_transition_candidate_to_accepted() {
        let (_dir, storage) = open();
        let id = storage
            .insert_submission("proj", "qa", "q1", "answer A", "alice")
            .unwrap();
        // list_pending sees it.
        let pending = storage.list_pending("proj", "qa", "q1").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].status, "candidate");
        assert_eq!(pending[0].candidate_by, "alice");

        // Accept it.
        storage.accept_submission(id, "mod", Some(0.92)).unwrap();

        // No longer pending; status accepted.
        let pending = storage.list_pending("proj", "qa", "q1").unwrap();
        assert!(pending.is_empty());

        let conn = storage.connection().unwrap();
        let (status, sim, decided_by): (String, Option<f64>, Option<String>) = conn
            .execute(|c| {
                Ok(c.query_row(
                    "SELECT status, similarity, decided_by FROM submissions WHERE id = ?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?)
            })
            .unwrap();
        assert_eq!(status, "accepted");
        assert_eq!(sim, Some(0.92));
        assert_eq!(decided_by.as_deref(), Some("mod"));
    }

    #[test]
    fn submissions_reject_records_strike() {
        let (_dir, storage) = open();
        let id = storage
            .insert_submission("proj", "rlm_node", "n1", "bad", "mallory")
            .unwrap();
        storage.reject_submission(id, "mod", Some(0.1)).unwrap();

        // Rejecting records a strike against the candidate author.
        let strikes = storage.record_strike("mallory").unwrap();
        assert_eq!(strikes, 1);
        assert_eq!(storage.volunteer_strikes("mallory").unwrap(), 1);

        // A second strike accumulates.
        let strikes = storage.record_strike("mallory").unwrap();
        assert_eq!(strikes, 2);
        assert_eq!(storage.volunteer_strikes("mallory").unwrap(), 2);

        // An unseen volunteer reports zero strikes.
        assert_eq!(storage.volunteer_strikes("nobody").unwrap(), 0);
    }

    #[test]
    fn submissions_list_pending_scoped_by_subject() {
        let (_dir, storage) = open();
        storage
            .insert_submission("proj", "qa", "q1", "a1", "alice")
            .unwrap();
        storage
            .insert_submission("proj", "qa", "q2", "a2", "alice")
            .unwrap();
        storage
            .insert_submission("other", "qa", "q1", "a3", "bob")
            .unwrap();

        let pending = storage.list_pending("proj", "qa", "q1").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].candidate_text, "a1");
    }
}
