//! Semantic query-answer cache persistence (plan 017).
//!
//! The server stores *digested* answers here (synthesized client-side). Lookup
//! by `(project, question_hash)` gives exact hits; similarity hits are resolved
//! by the caller against the dedicated `question_vectors` index
//! (`crate::qa_vectors`). `source_hashes` drive staleness: when a source chunk
//! changes, the lifecycle hook marks the row `stale` so the next query forces a
//! re-digest.
//!
//! All queries go through [`crate::sqlite::conn::Storage::connection`], which is
//! safe in both single (CLI) and pooled (server) modes.

pub(crate) mod embed;
pub(crate) mod evict;
pub(crate) mod mutate;
pub(crate) mod row;
pub(crate) mod store;
pub(crate) mod types;

pub use types::{QaCacheRow, StoreAnswerInput, StoredAnswer, chunk_content_hash, question_hash};
