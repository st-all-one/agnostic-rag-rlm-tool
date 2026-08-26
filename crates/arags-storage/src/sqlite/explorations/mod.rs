//! Explorations persistence (plan 022).
//!
//! Dense, goal-driven maps of how code entities connect, produced by explorer
//! agents with their local LLM and stored fire-and-forget (same pattern as
//! `qa_cache`). The server is deterministic: it anchors each cited file with a
//! content hash ([`anchors`]), compresses the body (zstd), embeds the summary
//! into the dedicated `exploration_vectors` index and serves maps with a
//! composite confidence score computed by `arags_core::exploration`.
//!
//! Split by concern:
//! - `store`: persist/get/FTS-search/touch/count/list
//! - `staleness`: project epochs and anchor-based invalidation
//! - `feedback`: confirm/contradict loop, retirement and admin invalidation
//!
//! All queries go through [`crate::sqlite::conn::Storage::connection`], which
//! is safe in both single (CLI) and pooled (server) modes.

pub mod feedback;
pub mod staleness;
pub mod store;

use anyhow::Context as _;
use anyhow::Result;

pub use feedback::{FeedbackKind, FeedbackOutcome};
pub use staleness::BrokenAnchor;

pub use arags_core::exploration::{
    ROLE_CITED, ROLE_CONTEXT, STATUS_FRESH, STATUS_PENDING, STATUS_RETIRED, STATUS_STALE,
    TEMPLATE_VERSION_V1,
};

/// zstd compression level for map bodies (matches CLI upload level).
const BODY_COMPRESSION_LEVEL: i32 = 3;

/// A stored exploration map row.
#[derive(Debug, Clone)]
pub struct ExplorationRow {
    /// Numeric rowid (also the `exploration_vectors` key).
    pub id: i64,
    /// Stable UUIDv7 id.
    pub exploration_id: String,
    /// Project name.
    pub project: String,
    /// Scoping buffer id.
    pub buffer_id: Option<i64>,
    /// Objective that drove the exploration.
    pub goal: String,
    /// Decompressed markdown body (the full contract document).
    pub body: String,
    /// Short digest used for embedding.
    pub summary: String,
    /// Agent username that persisted it (audit/provenance).
    pub created_by: String,
    /// LLM model that produced the map (metadata).
    pub model: Option<String>,
    /// Contract version used at persist time.
    pub template_version: String,
    /// `project_epochs` value when the map was created (confidence drift input).
    pub epoch_created: i64,
    /// Lifecycle state (`fresh` | `stale` | `retired`).
    pub status: String,
    /// Broken anchor paths (empty when fresh; advisory — recheck at read time).
    pub stale_reason: Vec<String>,
    /// Consumer verifications.
    pub confirmed: i64,
    /// Consumer contradictions.
    pub contradicted: i64,
    pub access_count: i64,
    pub token_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_accessed_at: i64,
}

/// One resolved anchor: a cited/context file with its content hash at persist
/// time. Resolution (path → buffer_id + current chunk hash) happens in the
/// server layer; storage persists what it is given.
#[derive(Debug, Clone)]
pub struct ExplorationAnchor {
    /// Buffer owning the file.
    pub buffer_id: i64,
    /// File path relative to the project root (chunks.file_path form).
    pub path: String,
    /// Current chunk content hash for the file at persist time.
    pub content_hash: String,
    /// Anchor role (`cited` invalidates, `context` is provenance-only).
    pub role: String,
}

/// Input for [`store::Storage::persist_exploration`].
#[derive(Debug, Clone)]
pub struct PersistExplorationInput {
    /// Project name.
    pub project: String,
    /// Scoping buffer id.
    pub buffer_id: Option<i64>,
    /// Objective that drove the exploration.
    pub goal: String,
    /// Full markdown contract document (compressed before insert).
    pub body_markdown: String,
    /// Short digest used for embedding.
    pub summary: String,
    /// Resolved anchors (cited + context files).
    pub anchors: Vec<ExplorationAnchor>,
    /// Agent username (audit/provenance).
    pub created_by: String,
    /// LLM model (metadata).
    pub model: Option<String>,
    /// Contract version (defaults to [`TEMPLATE_VERSION_V1`] when empty).
    pub template_version: String,
    /// Reported token cost of producing the map.
    pub token_count: i64,
}

/// Result of storing an exploration.
#[derive(Debug, Clone)]
pub struct StoredExploration {
    /// Stable UUIDv7 id.
    pub exploration_id: String,
    /// Numeric rowid (`exploration_vectors` key).
    pub id: i64,
}

/// Compress a markdown body for storage.
///
/// # Errors
///
/// Returns an error if compression fails.
pub fn compress_body(markdown: &str) -> Result<Vec<u8>> {
    zstd::stream::encode_all(markdown.as_bytes(), BODY_COMPRESSION_LEVEL)
        .context("failed to compress exploration body")
}

/// Decompress a stored body back to markdown.
///
/// # Errors
///
/// Returns an error if decompression fails or the bytes are not valid zstd.
pub fn decompress_body(bytes: &[u8]) -> Result<String> {
    let raw = zstd::stream::decode_all(bytes).context("failed to decompress exploration body")?;
    String::from_utf8(raw).context("exploration body is not valid UTF-8")
}

/// Parse the `stale_reason` JSON column into a list of broken paths.
#[must_use]
pub fn parse_stale_reason(text: Option<String>) -> Vec<String> {
    match text {
        Some(s) if !s.is_empty() => serde_json::from_str::<Vec<String>>(&s).unwrap_or_default(),
        _ => Vec::new(),
    }
}
