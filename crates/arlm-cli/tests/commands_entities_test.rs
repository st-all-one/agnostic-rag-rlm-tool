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

use arlm_cli::commands::entities::execute;
use arlm_cli::output::Format;
use tempfile::TempDir;

#[test]
fn test_entities_no_project() {
    let tmp = TempDir::new().unwrap();
    let result = execute("test", tmp.path(), Format::FullJson);
    assert!(result.is_err());
}
