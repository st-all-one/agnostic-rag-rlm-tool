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

use arags_cli::output::markdown::{SuperItem, render_run_result, render_search_results};

#[test]
fn test_render_search_results() {
    let items = vec![SuperItem {
        file_path: "src/main.rs".into(),
        score: 0.9,
        content: "fn main() {}".into(),
        language: Some("rust".into()),
    }];
    let md = render_search_results(&items);
    assert!(md.contains("# Search Results"));
    assert!(md.contains("src/main.rs"));
}

#[test]
fn test_render_run_result() {
    let md = render_run_result("analyze code", "found 3 issues", 1234);
    assert!(md.contains("analyze code"));
    assert!(md.contains("found 3 issues"));
}
