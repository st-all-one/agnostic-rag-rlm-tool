#![allow(
    unsafe_code,
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
)]

use arlm_cli::output::tree::{
    HistoryRow, render_history_table, render_search_results, render_tree,
};

#[test]
fn test_render_tree() {
    let tree = render_tree("run-001", "analyze project", 3);
    assert!(tree.contains("run-001"));
    assert!(tree.contains("analyze project"));
}

#[test]
fn test_render_search_results_empty() {
    let results = render_search_results(&[]);
    assert!(results.contains("Search Results (0)"));
}

#[test]
fn test_render_history_table() {
    let rows = vec![HistoryRow {
        date: "2024-01-15 10:30".into(),
        query: "find bugs".into(),
        duration: "2.3s".into(),
        results: "5".into(),
    }];
    let table = render_history_table(&rows);
    assert!(table.contains("find bugs"));
}
