#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use arags_search::types::{HybridResult, OutputFormat, SearchOptions, SearchTier};

#[test]
fn test_search_tier_display() {
    assert_eq!(SearchTier::Fts.to_string(), "fts");
    assert_eq!(SearchTier::Entity.to_string(), "entity");
    assert_eq!(SearchTier::Vector.to_string(), "vector");
}

#[test]
fn test_search_options_default() {
    let opts = SearchOptions::default();
    assert_eq!(opts.tier, SearchTier::Entity);
    assert_eq!(opts.top_k, 10);
}

#[test]
fn test_hybrid_result_clone() {
    let r = HybridResult {
        chunk_id: 1,
        score: 0.5,
    };
    let r2 = r.clone();
    assert_eq!(r.chunk_id, r2.chunk_id);
    assert_eq!(r.score, r2.score);
}

#[test]
fn test_output_format_variants() {
    let _ = OutputFormat::Prompt;
    let _ = OutputFormat::Json;
    let _ = OutputFormat::Markdown;
}
