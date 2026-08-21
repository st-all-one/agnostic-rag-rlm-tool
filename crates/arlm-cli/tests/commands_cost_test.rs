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

use arlm_cli::commands::cost::execute;
use arlm_cli::output::Format;
use arlm_storage::Storage;
use tempfile::TempDir;

#[test]
fn test_cost_empty() {
    let tmp = TempDir::new().unwrap();
    // SAFETY: test-only, single-threaded
    unsafe {
        std::env::set_var("ARLM_DATA_DIR", tmp.path());
    }
    let result = execute(None, None, tmp.path(), Format::FullJson);
    assert!(result.is_ok());
}

#[test]
fn test_cost_with_run() {
    let tmp = TempDir::new().unwrap();
    // SAFETY: test-only, single-threaded
    unsafe {
        std::env::set_var("ARLM_DATA_DIR", tmp.path());
    }

    let storage = Storage::open(tmp.path()).unwrap();
    storage
        .insert_run(
            "run-001",
            "test task",
            "openai",
            "auto",
            "completed",
            "arlm",
            1000,
            500,
            0.05,
            150,
            3,
            2,
            5,
            None,
            None,
            None,
        )
        .unwrap();

    let result = execute(Some("run-001"), None, tmp.path(), Format::FullJson);
    assert!(result.is_ok());
}
