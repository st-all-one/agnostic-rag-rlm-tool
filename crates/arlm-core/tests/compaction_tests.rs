#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::single_char_pattern
)]

use arlm_core::compaction::{Compaction, SearchResult, split_into_chunks};

fn make_result(score: f32, file_path: &str) -> SearchResult {
    SearchResult {
        score,
        content: format!("content of {file_path}"),
        file_path: file_path.to_string(),
    }
}

#[test]
fn test_new() {
    let c = Compaction::new(1000);
    assert_eq!(c.max_tokens(), 1000);
}

#[test]
fn test_with_recency_keep() {
    let c = Compaction::with_recency_keep(1000, 5);
    assert_eq!(c.max_tokens(), 1000);
}

#[test]
fn test_compact_empty_context() {
    let c = Compaction::new(1000);
    let result = c.compact("", &[]);
    assert!(result.is_empty());
}

#[test]
fn test_compact_empty_results() {
    let c = Compaction::new(1000);
    let ctx = "## Section 1\nHello world";
    let result = c.compact(ctx, &[]);
    assert_eq!(result, ctx);
}

#[test]
fn test_compact_within_budget() {
    let c = Compaction::new(10_000);
    let ctx = "## Section 1\nHello world";
    let results = vec![make_result(0.9, "a.rs")];
    let result = c.compact(ctx, &results);
    assert_eq!(result, ctx);
}

#[test]
fn test_compact_splits_by_headers() {
    let c = Compaction::new(50);
    let ctx = "## Section 1\nSome content here\n## Section 2\nMore content here\n## Section 3\nEven more content\n";
    let results = vec![
        make_result(0.5, "a.rs"),
        make_result(0.9, "b.rs"),
        make_result(0.3, "c.rs"),
    ];
    let result = c.compact(ctx, &results);
    assert!(result.contains("Section 2"));
    assert!(result.contains("Section 1"));
}

#[test]
fn test_compact_keeps_highest_scored() {
    let c = Compaction::with_recency_keep(5, 0);
    let ctx = "## Low\nshort\n## High\nshort\n## Mid\nshort\n";
    let results = vec![
        make_result(0.2, "low.rs"),
        make_result(0.9, "high.rs"),
        make_result(0.5, "mid.rs"),
    ];
    let result = c.compact(ctx, &results);
    assert!(result.contains("High"));
    assert!(!result.contains("Low"));
}

#[test]
fn test_compact_recency_bias() {
    let c = Compaction::with_recency_keep(80, 2);
    let ctx = "## Old1\nshort\n## Old2\nshort\n## Recent1\nshort\n## Recent2\nshort\n";
    let results = vec![
        make_result(0.1, "old1.rs"),
        make_result(0.1, "old2.rs"),
        make_result(0.1, "recent1.rs"),
        make_result(0.1, "recent2.rs"),
    ];
    let result = c.compact(ctx, &results);
    assert!(result.contains("Recent1"));
    assert!(result.contains("Recent2"));
}

#[test]
fn test_compact_falls_back_to_context() {
    let c = Compaction::with_recency_keep(10, 0);
    let ctx = "## A\nxx\n## B\nyy\n## C\nzz\n";
    let results = vec![
        make_result(0.5, "a.rs"),
        make_result(0.5, "b.rs"),
        make_result(0.5, "c.rs"),
    ];
    let result = c.compact(ctx, &results);
    assert!(!result.is_empty());
}

#[test]
fn test_split_into_chunks() {
    let ctx = "## A\nfoo\n## B\nbar\n";
    let chunks = split_into_chunks(ctx);
    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].contains("A"));
    assert!(chunks[1].contains("B"));
}

#[test]
fn test_split_no_headers() {
    let ctx = "just some text\nno headers here\n";
    let chunks = split_into_chunks(ctx);
    assert_eq!(chunks.len(), 1);
}
