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

use arlm_cli::commands::run::{RunConfig, execute};
use arlm_cli::output::Format;
use tempfile::TempDir;

#[tokio::test]
async fn test_run_without_llm_fails() {
    let tmp = TempDir::new().unwrap();
    let config = RunConfig {
        task: "test task",
        llm: false,
        backend: None,
        model: None,
        depth: 3,
        max_nodes: 50,
        concurrency: 4,
        max_budget: 1.0,
        project: tmp.path(),
        format: Format::Json,
        verbose: false,
        live: false,
        agent: None,
        custom_tools: Vec::new(),
        session_id: None,
        repl: false,
        persist: false,
    };
    let result = execute(config).await;
    assert!(result.is_err());
}
