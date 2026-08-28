//! RLM (Recursive Language Model) summaries persistence.
//!
//! Hierarchical summaries as a separate dataset (`rlm_nodes`, same pattern as
//! `qa_cache`): L1 = file summary, L2 = theme/module summary, L3 = project
//! overview. Volunteers claim work from `rlm_jobs` with a lease; completed
//! nodes pass a review gate before becoming searchable. Provenance lives in
//! `rlm_edges`; invalidation compares `source_hashes`.
//!
//! Split by concern:
//! - [`nodes`]: summary node CRUD, review gate, FTS/vector hydration
//! - [`jobs`]: volunteer work queue (enqueue/claim/complete/fail/cancel)
//! - [`graph`]: provenance edges and staleness invalidation
//!
//! All queries go through [`crate::sqlite::conn::Storage::connection`], which
//! is safe in both single (CLI) and pooled (server) modes.

pub mod complete;
pub mod graph;
pub mod jobs;
pub mod nodes;

use tracing::warn;

use rusqlite::OptionalExtension;
use rusqlite::params;

pub use arags_core::rlm::{
    DEFAULT_RLM_LEASE_MS, PRIORITY_CANCELLED, PRIORITY_CASCADE, PRIORITY_FRESH, PRIORITY_PARKED,
    PRIORITY_RETRY, RlmJobPayload,
};

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
    /// Authenticated session username that created the row (issue
    /// `agnostic-rlm-rs-786a`).
    pub created_by: Option<String>,
    /// Whether this is the live revision (issue `agnostic-rlm-rs-e210`).
    pub is_active: bool,
    /// Rowid of the newer revision that superseded this one (`is_active = 0`
    /// rows only); `None` for the live row (issue `agnostic-rlm-rs-e210`).
    pub superseded_by: Option<i64>,
    /// Project epoch at write time (drift / time-travel, plan 021).
    pub epoch: i64,
    /// Revision counter; starts at 1, bumped on supersede (plan 021).
    pub version: i64,
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
    pub created_by: Option<String>,
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

/// Input for enqueueing a job. Idempotent per `(project, level, subject)`.
#[derive(Debug, Clone)]
pub struct NewRlmJob {
    pub buffer_id: Option<i64>,
    pub project: String,
    pub level: i64,
    pub subject: String,
    pub payload: String,
    pub priority: i64,
    /// Number of independent volunteer slots to fan the subject out to when the
    /// cosine quorum is enabled (`> 1`). `1` (the default) keeps the classic
    /// single-volunteer behaviour. All `quorum_slots` rows share one
    /// `generation_group_id` so the quorum can treat them as one decision unit.
    pub quorum_slots: usize,
}

/// A job handed to a volunteer by [`jobs`]' claim operation.
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

/// Parse a nullable JSON string-array column, tolerating malformed data
/// (logged, treated as empty) and NULL/empty values.
pub(super) fn parse_json_array(text: Option<String>) -> Vec<String> {
    match text {
        Some(s) if !s.is_empty() => match serde_json::from_str::<Vec<String>>(&s) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    error = %e,
                    raw_len = s.len(),
                    "malformed json array in rlm column; treating as empty"
                );
                Vec::new()
            }
        },
        _ => Vec::new(),
    }
}

pub(super) const NODE_COLS: &str = "id, node_id, buffer_id, project, level, subject, \
     summary_text, source_hashes, model, volunteer_username, template_version, token_count, \
     confidence, review_status, reviewed_by, reviewed_at, access_count, created_at, \
     updated_at, last_accessed_at, stale, created_by, is_active, superseded_by, epoch, version";

pub(super) fn node_mapper(r: &rusqlite::Row<'_>) -> rusqlite::Result<RlmNode> {
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
        created_by: r.get(21)?,
        is_active: r.get::<_, i64>(22)? != 0,
        superseded_by: r.get(23)?,
        epoch: r.get(24)?,
        version: r.get(25)?,
    })
}

pub(super) const JOB_COLS: &str = "id, job_key, buffer_id, project, level, subject, payload, \
     generation, status, priority, claimed_by, claimed_at, lease_expires_at, attempts, \
     last_error, created_at, updated_at";

pub(super) fn job_mapper(r: &rusqlite::Row<'_>) -> rusqlite::Result<RlmJob> {
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

/// Superseding insert for a summary node (issue `agnostic-rlm-rs-e210`).
///
/// Runs on any connection/transaction handle; returns `(rowid, node_id)`.
///
/// If an active node already exists for `(project, level, subject)` it is
/// *retired* (`is_active = 0`) and a brand-new active row is inserted
/// (`version = old + 1`, `is_active = 1`); the retired row's `superseded_by` is
/// then linked to the new rowid. No active node → a fresh active row
/// (`version = 1`) is inserted. The retire-before-insert ordering keeps the
/// partial unique index (one active per subject) satisfied at all times.
pub(super) fn upsert_node_stmt(
    conn: &rusqlite::Connection,
    node_id: &str,
    hashes_json: &str,
    input: &NewRlmNode,
    now: i64,
) -> rusqlite::Result<(i64, String)> {
    // Find the current active revision for this subject (if any).
    let existing: Option<(i64, i64)> = conn
        .query_row(
            "SELECT id, version FROM rlm_nodes \
             WHERE project = ?1 AND level = ?2 AND subject = ?3 AND is_active = 1 LIMIT 1",
            params![input.project, input.level, input.subject],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?;

    // Retire the previous active row first so the partial unique index (one
    // active per subject) is never violated by the insert below.
    let old_id = if let Some((old_id, _)) = existing {
        conn.execute(
            "UPDATE rlm_nodes SET is_active = 0 WHERE id = ?1 AND is_active = 1",
            params![old_id],
        )?;
        Some(old_id)
    } else {
        None
    };

    let new_id: i64 = conn.query_row(
        "INSERT INTO rlm_nodes \
             (node_id, buffer_id, project, level, subject, summary_text, source_hashes, \
              model, volunteer_username, created_by, template_version, token_count, confidence, \
              review_status, version, is_active, created_at, updated_at, last_accessed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1.0, 'pending', ?13, 1, \
                     ?14, ?14, ?14) \
             RETURNING id",
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
            input.created_by,
            input.template_version,
            input.token_count,
            existing.map_or(1, |(_, v)| v + 1),
            now,
        ],
        |r| r.get(0),
    )?;

    // Link the retired revision to the new one.
    if let Some(old_id) = old_id {
        conn.execute(
            "UPDATE rlm_nodes SET superseded_by = ?1 WHERE id = ?2",
            params![new_id, old_id],
        )?;
    }

    Ok((new_id, node_id.to_string()))
}
