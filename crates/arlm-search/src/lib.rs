pub mod bm25;
pub mod context;
pub mod hybrid;
pub mod semantic;
pub mod types;

pub use bm25::Bm25Search;
pub use context::{build_context, build_search_results};
pub use hybrid::HybridSearch;
pub use semantic::SemanticSearch;
pub use types::{
    ChunkWithText, HybridResult, OutputFormat, SearchOptions, SearchResult, SearchTier,
    SemanticResult,
};

#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
