//! Query-Answer Cache RPCs (plan 017) + admin-gated `InvalidateCache`.
//!
//! The server is **deterministic**: it embeds, searches (BM25 + vector) and
//! stores — it never runs an LLM. Digestion happens client-side (plan 017
//! "digest-once"). `InvalidateCache` is admin-gated by plan 018: non-admins
//! receive `PERMISSION_DENIED`.
//!
//! `InvalidateCache` is backward-compatible with plan 018: when `cache_id` is
//! empty it purges the legacy `result_cache` by project (empty = all). When
//! `cache_id` is set it invalidates the semantic `qa_cache` (Stale/Delete, with
//! an optional similarity radius over `question_vectors`).

pub(crate) mod helpers;
pub(crate) mod invalidate;
pub(crate) mod pending;
pub(crate) mod query;
pub(crate) mod store;

pub use invalidate::handle_invalidate_cache;
pub use pending::{handle_claim_pending_qa, handle_complete_pending_qa};
pub use query::handle_query_with_cache;
pub use store::{handle_get_answer_by_id, handle_store_answer};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_trust;
