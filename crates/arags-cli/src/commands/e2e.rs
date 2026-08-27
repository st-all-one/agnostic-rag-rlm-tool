//! Gated end-to-end test against a REAL local LLM.
//!
//! This documents and exercises the full client-side digest/summary path
//! (`digest_chunks` + `generate_summary`) against the user's configured backend
//! (`~/.arags/arags.toml`). It is `#[ignore]`d by default and additionally
//! bails unless `ARAGS_TEST_REAL_LLM=1` is set, so it never runs in CI (where no
//! local LLM is available). A developer with Ollama/OpenAI/... configured can
//! run it explicitly:
//!
//! ```text
//! ARAGS_TEST_REAL_LLM=1 cargo test -p arags-cli --lib -- --ignored real_local_llm_e2e
//! ```
//!
//! The assertion contract is identical to the mock-backed tests: the stored
//! output must contain no leaked `<think>` chain-of-thought.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use arags_llm::LlmBackend;
use tokio::runtime::Runtime;

use crate::commands::persist::generate_summary;
use crate::commands::qa_cache::digest_chunks;
use crate::user_config::load;

fn build_real_backend() -> Arc<dyn LlmBackend> {
    let cfg = load().expect("failed to load ~/.arags/arags.toml for real-LLM test");
    crate::backend::resolve_backend(cfg.llm_config(), None, None)
        .expect("failed to resolve real LLM backend")
}

#[test]
#[ignore = "requires a configured local LLM (Ollama/OpenAI/...)"]
fn real_local_llm_e2e() {
    if std::env::var("ARAGS_TEST_REAL_LLM").as_deref() != Ok("1") {
        eprintln!("ARAGS_TEST_REAL_LLM=1 not set; skipping real-LLM E2E");
        return;
    }

    let rt = Runtime::new().expect("failed to build tokio runtime");
    let backend = build_real_backend();
    let backend_ref: &dyn LlmBackend = backend.as_ref();

    let model = backend
        .default_model()
        .unwrap_or_else(|| "llama3".to_string());

    let digest = digest_chunks(
        &rt,
        backend_ref,
        "Question: what does the project do?\n\nContext:\n# src/main.rs\n```\nfn main() {}\n```",
        &model,
    )
    .expect("real digest_chunks failed");
    assert!(
        !digest.contains("<think>"),
        "real digest must not leak chain-of-thought"
    );

    let summary = generate_summary(
        &rt,
        backend_ref,
        "e2e-project",
        &digest,
        "chunk_ids: (none)\nhashes: (none)",
        &model,
    )
    .expect("real generate_summary failed");
    assert!(
        !summary.contains("<think>"),
        "real summary must not leak chain-of-thought"
    );
    assert!(
        summary.contains("## Summary"),
        "real summary must contain the mandated section"
    );
}
