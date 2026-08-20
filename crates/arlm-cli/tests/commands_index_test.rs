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

use arlm_cli::commands::index::{IndexConfig, execute};
use arlm_cli::output::Format;
use tempfile::TempDir;

#[test]
fn test_index_empty_dir() {
    let tmp = TempDir::new().unwrap();
    // SAFETY: test-only, single-threaded
    unsafe {
        std::env::set_var("ARLM_DATA_DIR", tmp.path());
    }
    let project = TempDir::new().unwrap();
    let project_path = tmp.path().join("test-project");
    let config = IndexConfig {
        path: project.path(),
        chunk_size: 512,
        ignore_patterns: &[],
        watch: false,
        project: project_path.as_path(),
        format: Format::Json,
        verbose: false,
    };
    let result = execute(config);
    assert!(result.is_ok());
}
