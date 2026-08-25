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
                tracing::warn!(
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
     updated_at, last_accessed_at, stale";

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

/// Shared `INSERT .. ON CONFLICT` upsert for summary nodes (review gate reset).
/// Runs on any connection/transaction handle; returns `(rowid, node_id)`.
pub(super) fn upsert_node_stmt(
    conn: &rusqlite::Connection,
    node_id: &str,
    hashes_json: &str,
    input: &NewRlmNode,
    now: i64,
) -> rusqlite::Result<(i64, String)> {
    conn.query_row(
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
}
