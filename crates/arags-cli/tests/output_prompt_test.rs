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

use arags_cli::output::prompt::{PromptItem, render_search_context};

#[test]
fn test_render_search_context() {
    let items = vec![PromptItem {
        file_path: "src/main.rs".into(),
        score: 0.85,
        content: "fn main() {}".into(),
        language: Some("rust".into()),
    }];
    let ctx = render_search_context(&items);
    assert!(ctx.contains("## Project Context"));
    assert!(ctx.contains("src/main.rs"));
}
