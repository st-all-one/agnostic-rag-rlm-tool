#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::needless_borrow,
        clippy::unnecessary_literal_bound,
        clippy::float_cmp,
        clippy::duration_suboptimal_units,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )
)]
pub mod exploration;
pub mod logging;
pub mod qa_cache;
pub mod rlm;

/// Dimensionality of the project's fixed embedding model
/// (`all-MiniLM-L6-v2`). Single source of truth for sizing vector stores
/// and caches across the workspace.
pub const EMBEDDING_DIMS: usize = 384;

#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
