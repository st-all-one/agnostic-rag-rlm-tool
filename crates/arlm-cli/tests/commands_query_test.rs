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

use arlm_cli::commands::query::{QueryConfig, execute};
use arlm_cli::config::Config;
use arlm_cli::output::Format;
use tempfile::TempDir;

#[tokio::test]
async fn test_query_no_project() {
    let tmp = TempDir::new().unwrap();
    let project_path = tmp.path().join("nonexistent");
    // Without --llm, query succeeds even if project doesn't exist (returns empty context)
    let config = QueryConfig {
        question: "what is auth?",
        backend: Some("ollama"),
        model: None,
        project: project_path.as_path(),
        format: Format::FullJson,
        verbose: false,
        llm: false,
        config: &Config::default(),
    };
    let result = execute(config).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_query_with_llm_no_project() {
    let tmp = TempDir::new().unwrap();
    let project_path = tmp.path().join("nonexistent");
    // With --llm but no backend configured, should fail
    let config = QueryConfig {
        question: "what is auth?",
        backend: Some("ollama"),
        model: None,
        project: project_path.as_path(),
        format: Format::FullJson,
        verbose: false,
        llm: true,
        config: &Config::default(),
    };
    let result = execute(config).await;
    assert!(result.is_err());
}
