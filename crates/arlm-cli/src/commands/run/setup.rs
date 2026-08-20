use std::sync::Arc;

use anyhow::{Context, Result};

use arlm_core::{RlmBackend, RlmMode, StartRunInput};
use arlm_llm::BackendKind;

use crate::commands::run::config::RunConfig;
use crate::util::data_dir;

/// Resolve the on-disk project name from a path (its last component).
#[must_use]
pub fn project_name(project: &std::path::Path) -> &str {
    project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default")
}

/// Parse the backend name (defaulting to `ollama`) into a [`BackendKind`].
#[allow(clippy::missing_errors_doc)]
pub fn parse_backend(backend: Option<&str>) -> Result<BackendKind> {
    let name = backend.unwrap_or("ollama");
    name.parse().context("failed to parse backend")
}

/// Look up the API-key environment variable for a backend, if applicable.
#[must_use]
pub fn load_api_key(kind: BackendKind) -> Option<String> {
    let var = match kind {
        BackendKind::OpenAI => "OPENAI_API_KEY",
        BackendKind::Anthropic => "ANTHROPIC_API_KEY",
        BackendKind::Gemini => "GEMINI_API_KEY",
        BackendKind::Ollama => "",
        BackendKind::DeepSeek => "DEEPSEEK_API_KEY",
        BackendKind::MiMo => "MIMO_API_KEY",
    };
    if var.is_empty() {
        None
    } else {
        std::env::var(var).ok()
    }
}

/// Resolve the effective task string, injecting prior session context when a
/// `--session` id is supplied. Falls back to the raw task on any failure.
#[must_use]
pub fn resolve_effective_task(config: &RunConfig<'_>) -> String {
    let Some(sid) = config.session_id else {
        return config.task.to_string();
    };
    let Ok(stor) = arlm_storage::Storage::open(&data_dir()) else {
        return config.task.to_string();
    };
    let Ok(mgr) = arlm_memory::SessionManager::new(stor) else {
        return config.task.to_string();
    };
    match mgr.get_latest_context(sid) {
        Ok(Some(ctx)) => format!(
            "Previous context (session {sid}, version {}):\n{}\n\n---\n\nCurrent task: {}",
            ctx.version, ctx.payload, config.task
        ),
        _ => config.task.to_string(),
    }
}

/// Build the [`StartRunInput`] consumed by the RLM engine from CLI config.
#[must_use]
pub fn build_run_input(
    config: &RunConfig<'_>,
    kind: BackendKind,
    project_name: &str,
    run_id: &str,
    abort: arlm_core::AbortSignal,
    effective_task: &str,
) -> StartRunInput {
    StartRunInput {
        run_id: Arc::from(run_id),
        task: effective_task.to_string(),
        backend: match kind {
            BackendKind::OpenAI => RlmBackend::OpenAi,
            BackendKind::Anthropic => RlmBackend::Anthropic,
            BackendKind::Gemini => RlmBackend::Gemini,
            BackendKind::Ollama => RlmBackend::Ollama,
            BackendKind::DeepSeek => RlmBackend::DeepSeek,
            BackendKind::MiMo => RlmBackend::MiMo,
        },
        mode: if config.repl {
            RlmMode::Repl
        } else {
            RlmMode::Auto
        },
        model: config.model.map(String::from),
        project: project_name.to_string(),
        max_depth: config.depth,
        max_nodes: config.max_nodes,
        concurrency: config.concurrency,
        max_budget: config.max_budget,
        agent: config.agent.unwrap_or("arlm").to_string(),
        abort,
        custom_tools: config.custom_tools.clone(),
        ..Default::default()
    }
}
