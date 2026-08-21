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

use arlm_cli::commands::search::{SearchConfig, execute};
use arlm_cli::config::Config;
use arlm_cli::output::Format;
use tempfile::TempDir;

#[tokio::test]
async fn test_search_no_project() {
    let tmp = TempDir::new().unwrap();
    // SAFETY: test-only, single-threaded
    unsafe {
        std::env::set_var("ARLM_DATA_DIR", tmp.path());
    }
    let project_path = tmp.path().join("nonexistent");
    let config = SearchConfig {
        query: "test query",
        top_k: 10,
        file_pattern: None,
        min_score: None,
        all: false,
        tier: "auto",
        max_tokens: None,
        project: project_path.as_path(),
        format: Format::FullJson,
        verbose: false,
        persist: false,
        config: &Config::default(),
    };
    let result = execute(config).await;
    assert!(result.is_err());
}
