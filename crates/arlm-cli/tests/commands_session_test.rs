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

use arlm_cli::commands::session::{execute_create, execute_list};
use arlm_cli::output::Format;
use tempfile::TempDir;

#[test]
fn test_session_create_and_list() {
    let tmp = TempDir::new().unwrap();
    // SAFETY: test-only, single-threaded
    unsafe {
        std::env::set_var("ARLM_DATA_DIR", tmp.path());
    }
    let project = tmp.path().join("test-proj");
    std::fs::create_dir_all(&project).unwrap();

    let result = execute_create("My Analysis", &project, Format::Json);
    assert!(result.is_ok());

    let result = execute_list(&project, Format::Json);
    assert!(result.is_ok());
}
