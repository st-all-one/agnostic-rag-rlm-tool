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

use arlm_cli::commands::consolidate::{ConsolidateConfig, execute};
use arlm_cli::output::Format;
use tempfile::TempDir;

#[test]
fn test_consolidate_no_project() {
    let tmp = TempDir::new().unwrap();
    let project_path = tmp.path().join("nonexistent");
    let config = ConsolidateConfig {
        project: project_path.as_path(),
        format: Format::Json,
        verbose: false,
    };
    let result = execute(config);
    assert!(result.is_err());
}
