//! RLM (Recursive Language Model) summaries persistence.
//!
//! Hierarchical summaries as a separate dataset (`rlm_nodes`, same pattern as
//! `qa_cache`): L1 = file summary, L2 = theme/module summary, L3 = project
//! overview. Volunteers claim work from `rlm_jobs` with a lease; completed
//! nodes pass a review gate before becoming searchable. Provenance lives in
//! `rlm_edges`; invalidation compares `source_hashes`.
//!
//! All queries go through [`super::conn::Storage::connection`], which is safe
//! in both single (CLI) and pooled (server) modes.

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use rusqlite::params;
use std::fmt::Write as _;

use super::conn::Storage;
use super::tokens::now_ms;

/// Default job lease in milliseconds (500s for every level).
pub const DEFAULT_RLM_LEASE_MS: i64 = 500_000;

/// Review states of an RLM node.
pub const REVIEW_PENDING: &str = "pending";
pub const REVIEW_APPROVED: &str = "approved";
pub const REVIEW_REJECTED: &str = "rejected";

/// A stored recursive summary node.
#[derive(Debug, Clone)]
pub struct RlmNode {
    /// Numeric rowid.
    pub id: i64,
    /// Stable UUIDv7 id.
    pub node_id: String,
    /// Scoping buffer id (project).
    pub buffer_id: Option<i64>,
    /// Project name.
    pub project: String,
    /// Hierarchy level: 1=file, 2=theme, 3=project.
    pub level: i64,
    /// File path (L1), theme name (L2) or project name (L3).
    pub subject: String,
    /// Summary text produced by the volunteer LLM.
    pub summary_text: String,
    /// Content hashes of the inputs that produced this summary.
    pub source_hashes: Vec<String>,
    /// LLM model used (metadata).
    pub model: Option<String>,
    /// Username of the volunteer who processed it (audit).
    pub volunteer_username: Option<String>,
    /// Prompt template version used.
    pub template_version: Option<String>,
    /// Token cost reported by the volunteer.
    pub token_count: i64,
    /// Quality/confidence score; decays over time and on staleness.
    pub confidence: f64,
    /// Review gate state.
    pub review_status: String,
    /// Admin who reviewed (audit).
    pub reviewed_by: Option<String>,
    /// Review timestamp (epoch ms, audit).
    pub reviewed_at: Option<i64>,
    pub access_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_accessed_at: i64,
    /// Whether source data changed after this summary was written.
    pub stale: bool,
}

/// Input for storing (upserting) a summary node.
#[derive(Debug, Clone)]
pub struct NewRlmNode {
    pub buffer_id: Option<i64>,
    pub project: String,
    pub level: i64,
    pub subject: String,
    pub summary_text: String,
    pub source_hashes: Vec<String>,
    pub model: Option<String>,
    pub volunteer_username: Option<String>,
    pub template_version: Option<String>,
    pub token_count: i64,
}

/// A pending/claimed unit of volunteer work.
#[derive(Debug, Clone)]
pub struct RlmJob {
    pub id: i64,
    pub job_key: String,
    pub buffer_id: Option<i64>,
    pub project: String,
    pub level: i64,
    pub subject: String,
    /// JSON payload with input refs (node ids / chunk hashes / texts).
    pub payload: String,
    /// Bumped on cancel/re-enqueue; volunteers echo it back on completion so
    /// stale results from a cancelled lease are rejected.
    pub generation: i64,
    pub status: String,
    pub priority: i64,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<i64>,
    pub lease_expires_at: Option<i64>,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Input for enqueueing a job. Idempotent per `job_key`.
#[derive(Debug, Clone)]
pub struct NewRlmJob {
    pub buffer_id: Option<i64>,
    pub project: String,
    pub level: i64,
    pub subject: String,
    pub payload: String,
    pub priority: i64,
}

/// A job handed to a volunteer by [`Storage::claim_rlm_job`].
#[derive(Debug, Clone)]
pub struct ClaimedRlmJob {
    pub id: i64,
    pub job_key: String,
    pub project: String,
    pub level: i64,
    pub subject: String,
    pub payload: String,
    pub generation: i64,
    /// Lease duration granted, in ms (echoed back to the worker).
    pub lease_ms: i64,
}

fn parse_json_array(text: Option<String>) -> Vec<String> {
    match text {
        Some(s) if !s.is_empty() => serde_json::from_str::<Vec<String>>(&s).unwrap_or_default(),
        _ => Vec::new(),
    }
}

const NODE_COLS: &str = "id, node_id, buffer_id, project, level, subject, summary_text, \
     source_hashes, model, volunteer_username, template_version, token_count, confidence, \
     review_status, reviewed_by, reviewed_at, access_count, created_at, updated_at, \
     last_accessed_at, stale";

fn node_mapper(r: &rusqlite::Row<'_>) -> rusqlite::Result<RlmNode> {
    Ok(RlmNode {
        id: r.get(0)?,
        node_id: r.get(1)?,
        buffer_id: r.get(2)?,
        project: r.get(3)?,
        level: r.get(4)?,
        subject: r.get(5)?,
        summary_text: r.get(6)?,
        source_hashes: parse_json_array(r.get::<_, Option<String>>(7)?),
        model: r.get(8)?,
        volunteer_username: r.get(9)?,
        template_version: r.get(10)?,
        token_count: r.get(11)?,
        confidence: r.get(12)?,
        review_status: r.get(13)?,
        reviewed_by: r.get(14)?,
        reviewed_at: r.get(15)?,
        access_count: r.get(16)?,
        created_at: r.get(17)?,
        updated_at: r.get(18)?,
        last_accessed_at: r.get(19)?,
        stale: r.get::<_, i64>(20)? != 0,
    })
}

const JOB_COLS: &str = "id, job_key, buffer_id, project, level, subject, payload, generation, \
     status, priority, claimed_by, claimed_at, lease_expires_at, attempts, last_error, \
     created_at, updated_at";

fn job_mapper(r: &rusqlite::Row<'_>) -> rusqlite::Result<RlmJob> {
    Ok(RlmJob {
        id: r.get(0)?,
        job_key: r.get(1)?,
        buffer_id: r.get(2)?,
        project: r.get(3)?,
        level: r.get(4)?,
        subject: r.get(5)?,
        payload: r.get(6)?,
        generation: r.get(7)?,
        status: r.get(8)?,
        priority: r.get(9)?,
        claimed_by: r.get(10)?,
        claimed_at: r.get(11)?,
        lease_expires_at: r.get(12)?,
        attempts: r.get(13)?,
        last_error: r.get(14)?,
        created_at: r.get(15)?,
        updated_at: r.get(16)?,
    })
}

/// Deterministic job key so re-enqueues replace rather than duplicate.
#[must_use]
pub fn rlm_job_key(project: &str, level: i64, subject: &str) -> String {
    format!("L{level}:{project}:{subject}")
}

impl Storage {
    /// Upsert a summary node keyed by `(project, level, subject)`. The new
    /// submission replaces the previous content and **resets
    /// `review_status` to `pending`** (quality gate); provenance edges must be
    /// written separately via [`Storage::add_rlm_edge`].
    ///
    /// # Errors
    ///
    /// Returns an error if the upsert fails or hashes cannot be serialized.
    pub fn store_rlm_node(&self, input: &NewRlmNode) -> Result<(i64, String)> {
        let now = now_ms();
        let node_id = uuid::Uuid::now_v7().to_string();
        let hashes_json =
            serde_json::to_string(&input.source_hashes).context("serialize source_hashes")?;
        let conn = self.connection().context("acquire connection")?;
        let (id, node_id): (i64, String) = conn.execute(|c| {
            c.query_row(
                "INSERT INTO rlm_nodes \
                 (node_id, buffer_id, project, level, subject, summary_text, source_hashes, \
                  model, volunteer_username, template_version, token_count, confidence, \
                  review_status, created_at, updated_at, last_accessed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1.0, 'pending', ?12, ?12, ?12) \
                 ON CONFLICT(project, level, subject) DO UPDATE SET \
                   summary_text = excluded.summary_text, \
                   source_hashes = excluded.source_hashes, \
                   model = excluded.model, \
                   volunteer_username = excluded.volunteer_username, \
                   template_version = excluded.template_version, \
                   token_count = excluded.token_count, \
                   confidence = 1.0, \
                   review_status = 'pending', \
                   reviewed_by = NULL, \
                   reviewed_at = NULL, \
                   updated_at = excluded.updated_at, \
                   last_accessed_at = excluded.last_accessed_at, \
                   stale = 0 \
                 RETURNING id, node_id",
                params![
                    node_id,
                    input.buffer_id,
                    input.project,
                    input.level,
                    input.subject,
                    input.summary_text,
                    hashes_json,
                    input.model,
                    input.volunteer_username,
                    input.template_version,
                    input.token_count,
                    now,
                ],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .context("upsert rlm_node")
        })?;
        tracing::info!(
            node_id = %node_id,
            level = input.level,
            project = %input.project,
            "stored rlm node"
        );
        Ok((id, node_id))
    }

    /// Get an approved (or at least non-rejected) node by stable `node_id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_rlm_node(&self, node_id: &str) -> Result<Option<RlmNode>> {
        let conn = self.connection().context("acquire connection")?;
        let sql = format!("SELECT {NODE_COLS} FROM rlm_nodes WHERE node_id = ?1");
        conn.execute(|c| {
            c.query_row(sql.as_str(), params![node_id], node_mapper)
                .optional()
                .context("get rlm_node")
        })
    }

    /// List nodes for a project, optionally filtered by level and staleness.
    /// Only `approved` nodes are returned unless `include_pending` is set
    /// (admin review queue).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn list_rlm_nodes(
        &self,
        project: &str,
        level: Option<i64>,
        include_pending: bool,
    ) -> Result<Vec<RlmNode>> {
        let mut sql = format!(
            "SELECT {NODE_COLS} FROM rlm_nodes \
             WHERE project = ?1 AND review_status != '{REVIEW_REJECTED}'"
        );
        if !include_pending {
            let _ = write!(sql, " AND review_status = '{REVIEW_APPROVED}'");
        }
        if level.is_some() {
            sql.push_str(" AND level = ?2");
        }
        sql.push_str(" ORDER BY level, subject");
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c.prepare(&sql).context("prepare list_rlm_nodes")?;
            let rows = match level {
                Some(l) => stmt
                    .query_map(params![project, l], node_mapper)?
                    .collect::<std::result::Result<Vec<_>, _>>()?,
                None => stmt
                    .query_map(params![project], node_mapper)?
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            };
            Ok(rows)
        })
    }

    /// Record a provenance edge. Exactly one of `child_node_id`/`chunk_id`
    /// must be `Some`.
    ///
    /// # Errors
    ///
    /// Returns an error if both/neither reference is set or the insert fails.
    pub fn add_rlm_edge(
        &self,
        parent_rowid: i64,
        child_node_id: Option<i64>,
        chunk_id: Option<i64>,
    ) -> Result<()> {
        anyhow::ensure!(
            child_node_id.is_some() != chunk_id.is_some(),
            "rlm edge needs exactly one of child_node_id/chunk_id"
        );
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.execute(
                "INSERT OR IGNORE INTO rlm_edges (parent_id, child_node_id, chunk_id) \
                 VALUES (?1, ?2, ?3)",
                params![parent_rowid, child_node_id, chunk_id],
            )
            .context("insert rlm_edge")
        })?;
        Ok(())
    }

    /// Resolve the parent chain bottom-up: which node rowids depend directly
    /// or transitively on the given node rowids.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn rlm_parent_chain(&self, node_ids: &[i64]) -> Result<Vec<i64>> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            // Recursive CTE walking child -> parent edges upward.
            let list = node_ids
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "WITH RECURSIVE up(id) AS ( \
                   SELECT value FROM json_each('[{list}]') \
                   UNION \
                   SELECT e.parent_id FROM rlm_edges e \
                     JOIN up ON e.child_node_id = up.id \
                 ) SELECT DISTINCT id FROM up WHERE id NOT IN ({list})"
            );
            let mut stmt = c.prepare(&sql).context("prepare rlm_parent_chain")?;
            let rows = stmt
                .query_map([], |r| r.get::<_, i64>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Mark nodes stale when their recorded `source_hashes` intersect the
    /// changed hashes (same mechanism as qa_cache). Returns affected rows as
    /// `(rowid, project, level, subject)` so the caller can enqueue rework.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or the update fails.
    pub fn mark_rlm_stale_by_hashes(
        &self,
        buffer_id: i64,
        changed_hashes: &[String],
    ) -> Result<Vec<(i64, String, i64, String)>> {
        if changed_hashes.is_empty() {
            return Ok(Vec::new());
        }
        let hashes_json =
            serde_json::to_string(changed_hashes).context("serialize changed hashes")?;
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT id, project, level, subject FROM rlm_nodes \
                     WHERE buffer_id = ?1 AND stale = 0 \
                     AND EXISTS (SELECT 1 FROM json_each(rlm_nodes.source_hashes) j \
                         WHERE j.value IN (SELECT value FROM json_each(?2)))",
                )
                .context("prepare select stale rlm")?;
            let affected = stmt
                .query_map(params![buffer_id, hashes_json], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })
                .context("query stale rlm")?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if !affected.is_empty() {
                c.execute(
                    "UPDATE rlm_nodes SET stale = 1, confidence = 0 \
                     WHERE buffer_id = ?1 AND stale = 0 \
                     AND EXISTS (SELECT 1 FROM json_each(rlm_nodes.source_hashes) j \
                         WHERE j.value IN (SELECT value FROM json_each(?2)))",
                    params![buffer_id, hashes_json],
                )
                .context("mark rlm stale")?;
            }
            Ok(affected)
        })
    }

    /// Apply the quality-gate verdict to a node.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn review_rlm_node(
        &self,
        node_id: &str,
        approved: bool,
        reviewer: &str,
        reason: Option<&str>,
    ) -> Result<bool> {
        let status = if approved {
            REVIEW_APPROVED
        } else {
            REVIEW_REJECTED
        };
        let conn = self.connection().context("acquire connection")?;
        let n = conn.execute(|c| {
            c.execute(
                "UPDATE rlm_nodes SET review_status = ?1, reviewed_by = ?2, reviewed_at = ?3, \
                   confidence = CASE WHEN ?1 = 'approved' THEN confidence ELSE 0 END \
                 WHERE node_id = ?4",
                params![status, reviewer, now_ms(), node_id],
            )
            .context("review rlm_node")
        })?;
        let _ = reason; // recorded in tracing only for now (schema keeps it minimal)
        tracing::info!(node_id, status, reviewer, "rlm node reviewed");
        Ok(n > 0)
    }

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

    /// Atomically claim the next pending job for a volunteer. The lease is
    /// client-supplied (default [`DEFAULT_RLM_LEASE_MS`] = 500s); while the
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

    /// Complete a claimed job. Rejects the result if the lease expired, the
    /// claimant differs, or the job was cancelled/re-enqueued meanwhile
    /// (`generation` mismatch) — the caller should discard its work.
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
                ("failed", 9)
            } else {
                ("pending", 1) // retry soon, slightly elevated
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
    /// with `priority = 0` (front of the queue) and `generation + 1`: a
    /// volunteer still holding the old lease detects the cancellation via the
    /// generation mismatch on completion and discards its work. Returns how
    /// many live jobs were reset.
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
                        "UPDATE rlm_jobs SET status = 'pending', priority = 0, \
                           generation = generation + 1, attempts = 0, last_error = 'source changed', \
                           claimed_by = NULL, claimed_at = NULL, lease_expires_at = NULL, \
                           updated_at = ?2 \
                         WHERE job_key = ?1 AND status IN ('pending','claimed')",
                        params![key, now],
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

    /// Current chunk snapshot of a file: `(chunk_id, sha256 hex hash, text)`.
    /// Drives the L1 job payload and change detection.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn rlm_chunks_snapshot(
        &self,
        buffer_id: i64,
        file_path: &str,
    ) -> Result<Vec<(i64, String, Option<String>)>> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT c.id, hex(c.hash), t.content FROM chunks c \
                     LEFT JOIN chunk_texts t ON t.chunk_id = c.id \
                     WHERE c.buffer_id = ?1 AND c.file_path = ?2 ORDER BY c.id",
                )
                .context("prepare rlm_chunks_snapshot")?;
            let rows = stmt
                .query_map(params![buffer_id, file_path], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Mark a single node stale by `(buffer_id, level, subject)` — used when
    /// the motor already knows which subjects changed. Returns whether a live
    /// node was affected.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub fn mark_rlm_stale_by_subject(
        &self,
        buffer_id: i64,
        level: i64,
        subject: &str,
    ) -> Result<bool> {
        let conn = self.connection().context("acquire connection")?;
        let n = conn.execute(|c| {
            c.execute(
                "UPDATE rlm_nodes SET stale = 1, confidence = 0 \
                 WHERE buffer_id = ?1 AND level = ?2 AND subject = ?3 AND stale = 0",
                params![buffer_id, level, subject],
            )
            .context("mark_rlm_stale_by_subject")
        })?;
        Ok(n > 0)
    }

    /// Get a node by natural key `(project, level, subject)` regardless of
    /// review status (motor change-detection path).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_rlm_node_by_subject(
        &self,
        project: &str,
        level: i64,
        subject: &str,
    ) -> Result<Option<RlmNode>> {
        let sql = format!(
            "SELECT {NODE_COLS} FROM rlm_nodes \
             WHERE project = ?1 AND level = ?2 AND subject = ?3"
        );
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.query_row(sql.as_str(), params![project, level, subject], node_mapper)
                .optional()
                .context("get_rlm_node_by_subject")
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

    /// Resolve `(project, level, subject)` of a node by stable id (for job
    /// keys / cancellation).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn rlm_subject_of(&self, node_id: &str) -> Result<Option<(String, i64, String)>> {
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            c.query_row(
                "SELECT project, level, subject FROM rlm_nodes WHERE node_id = ?1",
                params![node_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .context("rlm_subject_of")
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

    /// Lexical search over approved, non-stale summaries via the `rlm_fts`
    /// index. `query` must already be FTS5-sanitised by the caller.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn search_rlm_fts(
        &self,
        buffer_id: i64,
        fts_query: &str,
        limit: usize,
    ) -> Result<Vec<RlmNode>> {
        let sql = format!(
            "SELECT {NODE_COLS} FROM rlm_nodes \
             WHERE rlm_nodes.rowid IN \
               (SELECT rowid FROM rlm_fts WHERE rlm_fts MATCH ?1 ORDER BY rank LIMIT ?3) \
               AND buffer_id = ?2 AND stale = 0 AND review_status = '{REVIEW_APPROVED}'"
        );
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            #[allow(clippy::cast_possible_wrap)] // limit is small
            let mut stmt = c.prepare(&sql).context("prepare search_rlm_fts")?;
            let rows = stmt
                .query_map(params![fts_query, buffer_id, limit as i64], node_mapper)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Fetch specific nodes by rowid (vector-search hydration). Only approved,
    /// non-stale nodes are returned.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_approved_rlm_nodes(&self, ids: &[u64]) -> Result<Vec<RlmNode>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let list = ids
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT {NODE_COLS} FROM rlm_nodes \
             WHERE id IN ({list}) AND stale = 0 AND review_status = '{REVIEW_APPROVED}'"
        );
        let conn = self.connection().context("acquire connection")?;
        conn.execute(|c| {
            let mut stmt = c.prepare(&sql).context("prepare get_approved_rlm_nodes")?;
            let rows = stmt
                .query_map([], node_mapper)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_storage() -> Storage {
        let dir = tempfile::tempdir().expect("tempdir");
        Storage::open(dir.path()).expect("open storage")
    }

    fn node(project: &str, level: i64, subject: &str, hashes: &[&str]) -> NewRlmNode {
        NewRlmNode {
            buffer_id: Some(1),
            project: project.into(),
            level,
            subject: subject.into(),
            summary_text: format!("summary of {subject}"),
            source_hashes: hashes.iter().map(|h| (*h).to_string()).collect(),
            model: Some("llama3.2".into()),
            volunteer_username: Some("alice".into()),
            template_version: Some("v1".into()),
            token_count: 42,
        }
    }

    fn job(project: &str, level: i64, subject: &str) -> NewRlmJob {
        NewRlmJob {
            buffer_id: Some(1),
            project: project.into(),
            level,
            subject: subject.into(),
            payload: "{}".into(),
            priority: 5,
        }
    }

    #[test]
    fn store_node_upsert_resets_review_gate() {
        let storage = temp_storage();
        let (id1, nid1) = storage
            .store_rlm_node(&node("p", 1, "src/main.rs", &["h1"]))
            .expect("store");
        assert!(!nid1.is_empty());

        let n = storage.get_rlm_node(&nid1).expect("get").expect("some");
        assert_eq!(n.review_status, REVIEW_PENDING);
        assert_eq!(n.level, 1);

        // Approve, then resubmit: review resets to pending, node_id stays stable.
        assert!(
            storage
                .review_rlm_node(&nid1, true, "admin", None)
                .expect("review")
        );
        let (id2, nid2) = storage
            .store_rlm_node(&node("p", 1, "src/main.rs", &["h2"]))
            .expect("resubmit");
        assert_eq!(id1, id2);
        assert_eq!(nid1, nid2);
        let n = storage.get_rlm_node(&nid1).expect("get").expect("some");
        assert_eq!(n.review_status, REVIEW_PENDING);
        assert_eq!(n.source_hashes, vec!["h2".to_string()]);
        assert!(!n.stale);
    }

    #[test]
    fn list_nodes_filters_by_review_and_level() {
        let storage = temp_storage();
        let (_, l3) = storage.store_rlm_node(&node("p", 3, "p", &[])).expect("l3");
        storage
            .review_rlm_node(&l3, true, "admin", None)
            .expect("approve");
        let _ = storage
            .store_rlm_node(&node("p", 1, "a.rs", &[]))
            .expect("l1");

        let approved = storage.list_rlm_nodes("p", None, false).expect("list");
        assert_eq!(approved.len(), 1);
        assert_eq!(approved[0].level, 3);

        let all = storage.list_rlm_nodes("p", Some(1), true).expect("list");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].subject, "a.rs");

        assert!(
            storage
                .list_rlm_nodes("other", None, true)
                .expect("x")
                .is_empty()
        );
    }

    #[test]
    fn edges_and_parent_chain_walk_up() {
        let storage = temp_storage();
        let (l1_id, _) = storage
            .store_rlm_node(&node("p", 1, "a.rs", &[]))
            .expect("l1");
        let (l2_id, _) = storage
            .store_rlm_node(&node("p", 2, "core", &[]))
            .expect("l2");
        let (l3_id, _) = storage.store_rlm_node(&node("p", 3, "p", &[])).expect("l3");
        storage
            .add_rlm_edge(l2_id, Some(l1_id), None)
            .expect("edge l2->l1");
        storage
            .add_rlm_edge(l3_id, Some(l2_id), None)
            .expect("edge l3->l2");

        // Exactly-one-reference guard.
        assert!(storage.add_rlm_edge(l1_id, None, None).is_err());

        let chain = storage.rlm_parent_chain(&[l1_id]).expect("chain");
        assert_eq!(chain, vec![l2_id, l3_id]);
    }

    #[test]
    fn staleness_marks_affected_nodes_with_hashes() {
        let storage = temp_storage();
        let (_, nid) = storage
            .store_rlm_node(&node("p", 1, "a.rs", &["h1", "h2"]))
            .expect("store");
        let affected = storage
            .mark_rlm_stale_by_hashes(1, &["zzz".to_string()])
            .expect("stale");
        assert!(affected.is_empty());
        let affected = storage
            .mark_rlm_stale_by_hashes(1, &["h2".to_string()])
            .expect("stale");
        assert_eq!(affected.len(), 1);
        let n = storage.get_rlm_node(&nid).expect("get").expect("some");
        assert!(n.stale);
        assert_eq!(n.confidence, 0.0);
    }

    #[test]
    fn enqueue_is_idempotent_for_pending_and_resets_finished() {
        let storage = temp_storage();
        let (id1, gen1) = storage
            .enqueue_rlm_job(&job("p", 1, "a.rs"))
            .expect("enqueue");
        assert_eq!(gen1, 0);
        let (id2, _) = storage
            .enqueue_rlm_job(&job("p", 1, "a.rs"))
            .expect("re-enqueue");
        assert_eq!(id1, id2);

        // Claim then finish; a new enqueue bumps generation and re-opens it.
        let claimed = storage
            .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None)
            .expect("claim")
            .expect("job");
        assert_eq!(claimed.subject, "a.rs");
        assert!(
            storage
                .complete_rlm_job(claimed.id, "bob", claimed.generation)
                .expect("complete")
        );
        assert_eq!(storage.count_rlm_jobs("p", "done").expect("count"), 1);

        let (_, gen3) = storage
            .enqueue_rlm_job(&job("p", 1, "a.rs"))
            .expect("reset");
        assert_eq!(gen3, 1);
        assert_eq!(storage.count_rlm_jobs("p", "pending").expect("count"), 1);
    }

    #[test]
    fn claim_locks_work_unit_until_completion() {
        let storage = temp_storage();
        storage
            .enqueue_rlm_job(&job("p", 1, "a.rs"))
            .expect("enqueue");

        let first = storage
            .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None)
            .expect("claim1")
            .expect("job1");
        // While the lease is live no other volunteer can claim the same unit.
        assert!(
            storage
                .claim_rlm_job("carol", DEFAULT_RLM_LEASE_MS, None)
                .expect("claim2")
                .is_none()
        );

        // Wrong worker or generation is rejected.
        assert!(
            !storage
                .complete_rlm_job(first.id, "carol", first.generation)
                .expect("wrong worker")
        );
        assert!(
            !storage
                .complete_rlm_job(first.id, "bob", first.generation + 7)
                .expect("wrong gen")
        );
        assert!(
            storage
                .complete_rlm_job(first.id, "bob", first.generation)
                .expect("complete")
        );
    }

    #[test]
    fn expired_lease_requeues() {
        let storage = temp_storage();
        storage
            .enqueue_rlm_job(&job("p", 1, "a.rs"))
            .expect("enqueue");
        let _ = storage
            .claim_rlm_job("bob", 1_000, None)
            .expect("claim")
            .expect("job");
        // Simulate lease expiry.
        let conn = storage.connection().expect("conn");
        conn.execute(|c| {
            c.execute(
                "UPDATE rlm_jobs SET lease_expires_at = ?1 - 10",
                params![now_ms()],
            )
            .context("backdate lease")
        })
        .expect("backdate");
        drop(conn);

        assert_eq!(storage.requeue_expired_rlm_leases().expect("requeue"), 1);
        assert_eq!(storage.count_rlm_jobs("p", "pending").expect("pending"), 1);
    }

    #[test]
    fn cancel_bumps_generation_and_elevates_priority() {
        let storage = temp_storage();
        storage
            .enqueue_rlm_job(&job("p", 1, "a.rs"))
            .expect("enqueue");
        let claimed = storage
            .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None)
            .expect("claim")
            .expect("job");

        let n = storage
            .cancel_rlm_jobs_for_subjects("p", &[(1, "a.rs".into())])
            .expect("cancel");
        assert_eq!(n, 1);

        // Old lease completion is rejected (generation mismatch).
        assert!(
            !storage
                .complete_rlm_job(claimed.id, "bob", claimed.generation)
                .expect("stale complete")
        );
        // Job is back at the front of the queue for reprocessing.
        let next = storage
            .claim_rlm_job("carol", DEFAULT_RLM_LEASE_MS, None)
            .expect("claim")
            .expect("priority job");
        assert_eq!(next.id, claimed.id);
        assert_eq!(next.generation, claimed.generation + 1);
    }

    #[test]
    fn fail_returns_to_pending_then_parks_after_max_attempts() {
        let storage = temp_storage();
        storage
            .enqueue_rlm_job(&job("p", 1, "a.rs"))
            .expect("enqueue");
        let j1 = storage
            .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, None)
            .expect("claim")
            .expect("j1");
        storage
            .fail_rlm_job(j1.id, "bob", "llm timeout", 3)
            .expect("fail");
        assert_eq!(storage.count_rlm_jobs("p", "pending").expect("pending"), 1);

        for worker in ["c1", "c2"] {
            let j = storage
                .claim_rlm_job(worker, DEFAULT_RLM_LEASE_MS, None)
                .expect("claim")
                .expect("j");
            storage
                .fail_rlm_job(j.id, worker, "llm timeout", 3)
                .expect("fail");
        }
        assert_eq!(storage.count_rlm_jobs("p", "failed").expect("failed"), 1);
    }

    #[test]
    fn max_level_filter_limits_claims() {
        let storage = temp_storage();
        storage
            .enqueue_rlm_job(&job("p", 3, "p-overview"))
            .expect("enqueue l3");
        // Volunteer that only accepts L1/L2 gets nothing.
        assert!(
            storage
                .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, Some(2))
                .expect("claim capped")
                .is_none()
        );
        assert!(
            storage
                .claim_rlm_job("bob", DEFAULT_RLM_LEASE_MS, Some(3))
                .expect("claim full")
                .is_some()
        );
    }

    #[test]
    fn job_key_is_deterministic_per_level_subject() {
        assert_eq!(
            rlm_job_key("proj", 2, "core"),
            rlm_job_key("proj", 2, "core")
        );
        assert_ne!(
            rlm_job_key("proj", 1, "core"),
            rlm_job_key("proj", 2, "core")
        );
    }
}
