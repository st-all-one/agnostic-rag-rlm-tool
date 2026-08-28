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
use std::time::Instant;
use tracing::debug;

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
        let start = Instant::now();
        let id: i64 = conn
            .execute(|c| {
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
            .context("execute insert submission")?;
        debug!(duration_ms = %start.elapsed().as_millis(), subject_type, subject_key, "inserted submission");
        Ok(id)
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

    /// Record a strike against a volunteer, returning the new `(strikes,
    /// trust_score)` pair. Each strike both increments the strike counter and
    /// decays `trust_score` by `0.2` (clamped at `0.0`). The `volunteer_trust`
    /// row is created on first contact with `strikes = 0, trust_score = 1.0`.
    ///
    /// # Errors
    ///
    /// Returns an error if the upsert or increment fails.
    #[allow(clippy::cast_possible_truncation)]
    pub fn record_strike(&self, volunteer: &str) -> Result<(u32, f64)> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.execute(
                "INSERT INTO volunteer_trust (username, strikes, trust_score) \
                 VALUES (?1, 0, 1.0) ON CONFLICT(username) DO NOTHING",
                params![volunteer],
            )
            .context("seed volunteer trust")?;
            c.execute(
                "UPDATE volunteer_trust SET strikes = strikes + 1, \
                    trust_score = MAX(0.0, trust_score - 0.2) WHERE username = ?1",
                params![volunteer],
            )
            .context("increment volunteer strike")?;
            let row: (i64, f64) = c
                .query_row(
                    "SELECT strikes, trust_score FROM volunteer_trust WHERE username = ?1",
                    params![volunteer],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .context("read volunteer trust")?;
            #[allow(clippy::cast_sign_loss)]
            Ok((row.0 as u32, row.1))
        })
    }

    /// Nudge a volunteer's trust up after one of their candidates was accepted
    /// by the quorum. `trust_score` is raised by `0.1` (clamped at `1.0`) and a
    /// single prior strike is forgiven (decremented, never below `0`) so steady
    /// good behaviour recovers from an occasional divergence.
    ///
    /// # Errors
    ///
    /// Returns an error if the upsert or update fails.
    pub fn bump_trust_on_accept(&self, volunteer: &str) -> Result<()> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.execute(
                "INSERT INTO volunteer_trust (username, strikes, trust_score) \
                 VALUES (?1, 0, 1.0) ON CONFLICT(username) DO NOTHING",
                params![volunteer],
            )
            .context("seed volunteer trust")?;
            c.execute(
                "UPDATE volunteer_trust \
                    SET trust_score = MIN(1.0, trust_score + 0.1), \
                        strikes = MAX(0, strikes - 1) \
                  WHERE username = ?1",
                params![volunteer],
            )
            .context("bump volunteer trust")?;
            Ok(())
        })
    }

    /// Whether a volunteer has reached the ban threshold (`strikes >=
    /// strikes_limit`). An unseen volunteer is never banned.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    #[allow(clippy::cast_possible_truncation)]
    pub fn is_banned(&self, volunteer: &str, strikes_limit: u32) -> Result<bool> {
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
            let strikes = strikes.unwrap_or(0) as u32;
            Ok(strikes >= strikes_limit)
        })
    }

    /// Read a volunteer's current `(strikes, trust_score)` (`(0, 1.0)` if never
    /// seen). Reused by later consumers (`64af`) that need both counters.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    #[allow(clippy::cast_possible_truncation)]
    pub fn read_trust(&self, volunteer: &str) -> Result<(u32, f64)> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let row: Option<(i64, f64)> = c
                .query_row(
                    "SELECT strikes, trust_score FROM volunteer_trust WHERE username = ?1",
                    params![volunteer],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .context("read volunteer trust")?;
            #[allow(clippy::cast_sign_loss)]
            Ok(match row {
                Some((s, t)) => (s as u32, t),
                None => (0, 1.0),
            })
        })
    }

    /// Rank volunteers by trust for claimer selection / observability:
    /// `(username, trust_score, strikes)` ordered by `trust_score DESC,
    /// strikes ASC`, limited to `limit` rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    #[allow(clippy::cast_possible_truncation)]
    pub fn list_volunteers_by_trust(&self, limit: u32) -> Result<Vec<(String, f64, u32)>> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT username, trust_score, strikes FROM volunteer_trust \
                     ORDER BY trust_score DESC, strikes ASC LIMIT ?1",
                )
                .context("prepare volunteer ranking")?;
            let rows = stmt
                .query_map(params![limit as i64], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, f64>(1)?,
                        #[allow(clippy::cast_sign_loss)]
                        {
                            r.get::<_, i64>(2)? as u32
                        },
                    ))
                })
                .context("query volunteer ranking")?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.context("map volunteer ranking row")?);
            }
            Ok(out)
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
        let (strikes, _trust) = storage.record_strike("mallory").unwrap();
        assert_eq!(strikes, 1);
        assert_eq!(storage.volunteer_strikes("mallory").unwrap(), 1);

        // A second strike accumulates.
        let (strikes, _trust) = storage.record_strike("mallory").unwrap();
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

    #[test]
    fn trust_score_decreases_on_strike_and_increases_on_accept() {
        let (_dir, storage) = open();
        // Fresh volunteer starts at trust 1.0.
        let (_, trust0) = storage.read_trust("alice").unwrap();
        assert_eq!(trust0, 1.0);

        // A strike decays trust by 0.2.
        let (_, trust1) = storage.record_strike("alice").unwrap();
        assert!((trust1 - 0.8).abs() < f64::EPSILON);
        let (_, trust2) = storage.record_strike("alice").unwrap();
        assert!((trust2 - 0.6).abs() < f64::EPSILON);

        // An accepted candidate nudges trust back up by 0.1.
        storage.bump_trust_on_accept("alice").unwrap();
        let (_, trust3) = storage.read_trust("alice").unwrap();
        assert!((trust3 - 0.7).abs() < f64::EPSILON);

        // trust_score is clamped at 1.0 on repeated accepts.
        for _ in 0..20 {
            storage.bump_trust_on_accept("alice").unwrap();
        }
        let (_, trust_max) = storage.read_trust("alice").unwrap();
        assert!((trust_max - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn list_volunteers_by_trust_ranks_correctly() {
        let (_dir, storage) = open();
        // alice stays clean (trust 1.0); bob takes two strikes (0.6); carol one
        // (0.8). Ties on trust break by fewer strikes first.
        storage.bump_trust_on_accept("alice").unwrap();
        storage.record_strike("bob").unwrap();
        storage.record_strike("bob").unwrap();
        storage.record_strike("carol").unwrap();

        let ranked = storage.list_volunteers_by_trust(10).unwrap();
        assert_eq!(ranked.len(), 3);
        // Highest trust first.
        assert_eq!(ranked[0].0, "alice");
        assert!((ranked[0].1 - 1.0).abs() < f64::EPSILON);
        // carol (0.8) outranks bob (0.6).
        assert_eq!(ranked[1].0, "carol");
        assert_eq!(ranked[2].0, "bob");

        // Limit is honoured.
        assert_eq!(storage.list_volunteers_by_trust(1).unwrap().len(), 1);
    }
}
