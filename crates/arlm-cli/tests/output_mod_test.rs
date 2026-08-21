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
    assert_eq!(Format::FullJson.to_string(), "full_json");
    assert_eq!(Format::Path.to_string(), "path");
    assert_eq!(Format::Markdown.to_string(), "markdown");
    assert_eq!(Format::Text.to_string(), "text");
    assert_eq!(Format::Jsonl.to_string(), "jsonl");
}
