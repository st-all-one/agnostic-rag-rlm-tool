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

use arlm_cli::output::Format;

#[test]
fn test_format_display() {
    assert_eq!(Format::Json.to_string(), "json");
    assert_eq!(Format::Tree.to_string(), "tree");
    assert_eq!(Format::Markdown.to_string(), "markdown");
    assert_eq!(Format::Prompt.to_string(), "prompt");
}
