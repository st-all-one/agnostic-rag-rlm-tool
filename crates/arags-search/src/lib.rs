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
pub mod bm25;
pub mod context;
pub mod decay;
pub mod entity;
pub mod hybrid;
pub mod qa_cache;
pub mod semantic;
pub mod types;

pub use bm25::Bm25Search;
pub use context::{build_context, build_search_results};
pub use decay::DecayConfig;
pub use entity::EntitySearch;
pub use hybrid::HybridSearch;
pub use semantic::SemanticSearch;
pub use types::{
    ChunkWithText, EntityResult, HybridResult, OutputFormat, SearchOptions, SearchResult,
    SearchTier, SemanticResult,
};

#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
