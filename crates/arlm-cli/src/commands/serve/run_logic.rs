use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::Value;
use tracing::{debug, instrument};

use crate::commands::serve::requests::RunRequest;
use crate::commands::serve::state::AppState;

/// Run the RLM engine recursively for a task.
///
/// # Errors
/// Returns an error if the backend cannot be parsed or created, or if the RLM
/// engine itself fails during execution.
#[instrument(skip_all)]
pub async fn handle_run(state: &AppState, req: &RunRequest) -> Result<Value> {
    let start = Instant::now();

    let backend_name = req.backend.as_deref().unwrap_or("ollama");
    let kind: arlm_llm::BackendKind = backend_name.parse().context("failed to parse backend")?;

    let api_key = std::env::var(match kind {
        arlm_llm::BackendKind::OpenAI => "OPENAI_API_KEY",
        arlm_llm::BackendKind::Anthropic => "ANTHROPIC_API_KEY",
        arlm_llm::BackendKind::Gemini => "GEMINI_API_KEY",
        arlm_llm::BackendKind::Ollama => "",
        arlm_llm::BackendKind::DeepSeek => "DEEPSEEK_API_KEY",
        arlm_llm::BackendKind::MiMo => "MIMO_API_KEY",
    })
    .ok();

    let llm_backend =
        arlm_llm::get_backend(&kind, api_key, None).context("failed to create LLM backend")?;

    let run_id = format!("run-{}", uuid::Uuid::now_v7().as_simple());

    let input = arlm_core::StartRunInput {
        run_id: Arc::from(run_id.as_str()),
        task: req.task.clone(),
        backend: match kind {
            arlm_llm::BackendKind::OpenAI => arlm_core::RlmBackend::OpenAi,
            arlm_llm::BackendKind::Anthropic => arlm_core::RlmBackend::Anthropic,
            arlm_llm::BackendKind::Gemini => arlm_core::RlmBackend::Gemini,
            arlm_llm::BackendKind::Ollama => arlm_core::RlmBackend::Ollama,
            arlm_llm::BackendKind::DeepSeek => arlm_core::RlmBackend::DeepSeek,
            arlm_llm::BackendKind::MiMo => arlm_core::RlmBackend::MiMo,
        },
        model: req.model.clone(),
        project: state.project_name.clone(),
        max_depth: req.depth,
        max_nodes: req.max_nodes,
        ..Default::default()
    };

    let result =
        arlm_core::run_rlm_engine_with_events(input, llm_backend, state.event_bus.clone(), None)
            .await
            .context("RLM engine failed")?;

    state.metrics.record_node();

    // Record agent metrics if agent name provided
    if let Some(ref agent) = req.agent {
        state.metrics.record_agent_request(agent, 0);
    }

    debug!(elapsed_ms = %start.elapsed().as_millis(), run_id = %run_id, "run completed");
    Ok(serde_json::json!({
        "run_id": result.run_id,
        "task": req.task,
        "result": result.final_output,
        "duration_ms": result.stats.duration_ms,
        "nodes_visited": result.stats.nodes_visited,
        "max_depth": result.stats.max_depth_seen,
    }))
}
