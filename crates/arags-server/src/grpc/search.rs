//! Search and context-building RPCs: `Search`, `BuildContext`.
//!
//! Both run a unified hybrid search (`arags_search::HybridSearch`) over the
//! project's chunks: BM25 (FTS5) is always the base tier, and the `entity`,
//! `vector` (semantic) tiers are fused on top via Reciprocal
//! Rank Fusion (RRF). The semantic tier is powered by the server's embedder
//! (native all-MiniLM-L6-v2; a hash fallback without weights), so vector
//! search degrades gracefully to BM25 when no vector store is configured.
//!
//! Result scores are min-max normalised to `[0, 1]` (higher = better) so that
//! `--min-score` thresholds and client ranking stay meaningful regardless of
//! which tiers contributed. Natural-language questions that return nothing
//! under FTS5's default AND semantics are retried with an OR pass.

pub(crate) mod context;
pub(crate) mod hybrid;
pub(crate) mod query;
pub(crate) mod summary;

pub(crate) use context::{apply_chunk_as_of, handle_build_context};
pub(crate) use hybrid::{buffer_id_for, hybrid_search};
pub(crate) use query::handle_search;

#[cfg(test)]
mod tests;
