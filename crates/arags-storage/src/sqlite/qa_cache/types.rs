//! Public QA-cache row types and the canonical hashing helpers.

use sha2::{Digest, Sha256};

/// A stored query-answer cache row.
#[derive(Debug, Clone)]
pub struct QaCacheRow {
    /// Numeric rowid (also the `question_vectors` key).
    pub id: i64,
    /// Stable UUIDv7 answer id (anti-drift, propagated to sub-agents).
    pub cache_id: String,
    /// Scoping buffer id (project).
    pub buffer_id: Option<i64>,
    /// Project name (redundant for fast lookup).
    pub project: String,
    /// Original question text.
    pub question_text: String,
    /// Exact-hit hash of the question.
    pub question_hash: String,
    /// Digested answer text.
    pub answer_text: String,
    /// Provenance: chunk ids that produced the answer.
    pub source_chunk_ids: Vec<String>,
    /// Invalidation: content hashes of source chunks.
    pub source_hashes: Vec<String>,
    /// LLM model that synthesized (metadata).
    pub model: Option<String>,
    /// Confidence (decays to 0 when stale).
    pub confidence: f64,
    /// Thresholds snapshot (JSON, for reproducibility).
    pub tier_snapshot: Option<String>,
    /// Token cost of the answer.
    pub token_count: i64,
    /// Access count (for weighted LRU eviction).
    pub access_count: i64,
    /// Created epoch ms.
    pub created_at: i64,
    /// Last accessed epoch ms.
    pub last_accessed_at: i64,
    /// Whether the entry is stale.
    pub stale: bool,
    /// Epoch ms of manual invalidation (audit).
    pub invalidated_at: Option<i64>,
    /// Who invalidated (audit).
    pub invalidated_by: Option<String>,
    /// Why invalidated (audit).
    pub invalidated_reason: Option<String>,
    /// Whether this is the live revision (issue `agnostic-rlm-rs-e210`).
    pub is_active: bool,
    /// Rowid of the newer revision that superseded this one (`is_active = 0`
    /// rows only); `None` for the live row (issue `agnostic-rlm-rs-e210`).
    pub superseded_by: Option<i64>,
    /// Project epoch at write time (drift / time-travel, plan 021).
    pub epoch: i64,
    /// Agent username that stored the answer (audit/provenance).
    pub created_by: Option<String>,
    /// Revision counter; starts at 1, bumped on supersede (plan 021).
    pub version: i64,
}

/// Input for [`super::store::store_answer`].
#[derive(Debug, Clone)]
pub struct StoreAnswerInput {
    /// Scoping buffer id.
    pub buffer_id: Option<i64>,
    /// Project name.
    pub project: String,
    /// Original question text.
    pub question_text: String,
    /// Exact-hit hash of the question.
    pub question_hash: String,
    /// Digested answer text.
    pub answer_text: String,
    /// Provenance: chunk ids.
    pub source_chunk_ids: Vec<String>,
    /// Invalidation: chunk content hashes.
    pub source_hashes: Vec<String>,
    /// LLM model (metadata).
    pub model: Option<String>,
    /// Thresholds snapshot (JSON).
    pub tier_snapshot: Option<String>,
    /// Token cost.
    pub token_count: i64,
    /// Authenticated session username that stored the answer (issue
    /// `agnostic-rlm-rs-786a`). `None` when the store is used outside an
    /// authenticated session (e.g. CLI hermetic paths).
    pub created_by: Option<String>,
}

/// Result of storing an answer.
#[derive(Debug, Clone)]
pub struct StoredAnswer {
    /// Stable answer id.
    pub cache_id: String,
    /// Numeric rowid (question_vectors key).
    pub id: i64,
    /// Whether this was a brand-new entry (vs. an idempotent reuse).
    pub created: bool,
}

/// Compute the exact-hit hash for a question (normalized, lowercased).
#[must_use]
pub fn question_hash(question: &str) -> String {
    let normalized: String = question
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    hex::encode(hasher.finalize())
}

/// Canonical content hash for a chunk (sha256 hex). Clients must use this exact
/// function when computing `source_hashes` so the server's staleness hook can
/// compare against stored chunk hashes.
///
/// Re-exported from [`arags_core::qa_cache::chunk_content_hash`] so client and
/// server share one implementation (plan 020: CLI has no storage dependency).
#[must_use]
pub fn chunk_content_hash(content: &str) -> String {
    arags_core::qa_cache::chunk_content_hash(content)
}
