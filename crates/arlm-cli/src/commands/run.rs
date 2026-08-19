use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::output::{self, Format};

pub struct RunConfig<'a> {
    pub task: &'a str,
    pub llm: bool,
    pub backend: Option<&'a str>,
    pub model: Option<&'a str>,
    pub depth: u32,
    pub max_nodes: u32,
    pub concurrency: usize,
    pub max_budget: f64,
    pub project: &'a Path,
    pub format: Format,
    pub verbose: bool,
}

pub async fn execute(config: RunConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_run");

    if !config.llm {
        anyhow::bail!(
            "`arlm run` requires --llm flag. Use `arlm search` or `arlm context` for deterministic operations."
        );
    }

    let project_name = config
        .project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");

    let backend_name = config.backend.unwrap_or("ollama");
    let kind: arlm_llm::BackendKind = backend_name
        .parse()
        .context("failed to parse backend")?;

    let api_key = std::env::var(match kind {
        arlm_llm::BackendKind::OpenAI => "OPENAI_API_KEY",
        arlm_llm::BackendKind::Anthropic => "ANTHROPIC_API_KEY",
        arlm_llm::BackendKind::Gemini => "GEMINI_API_KEY",
        arlm_llm::BackendKind::Ollama => "",
    })
    .ok();

    let llm_backend = arlm_llm::get_backend(&kind, api_key, None)
        .context("failed to create LLM backend")?;

    let run_id = format!("run-{}", uuid::Uuid::new_v4().as_simple());

    if config.verbose {
        output::info(&format!(
            "Starting RLM run {run_id} (backend={backend_name}, depth={}, max_nodes={})",
            config.depth, config.max_nodes
        ));
    }

    let progress = indicatif::ProgressBar::new_spinner();
    progress.set_style(
        indicatif::ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg} [{elapsed_precise}]")
            .map_err(|e| anyhow::anyhow!("invalid template: {e}"))?,
    );
    progress.set_message(format!("Running RLM: {}", config.task));

    let input = arlm_core::StartRunInput {
        run_id: Arc::from(run_id.as_str()),
        task: config.task.to_string(),
        backend: match kind {
            arlm_llm::BackendKind::OpenAI => arlm_core::RlmBackend::OpenAi,
            arlm_llm::BackendKind::Anthropic => arlm_core::RlmBackend::Anthropic,
            arlm_llm::BackendKind::Gemini => arlm_core::RlmBackend::Gemini,
            arlm_llm::BackendKind::Ollama => arlm_core::RlmBackend::Ollama,
        },
        model: config.model.map(String::from),
        project: project_name.to_string(),
        max_depth: config.depth,
        max_nodes: config.max_nodes,
        concurrency: config.concurrency,
        max_budget: config.max_budget,
        ..Default::default()
    };

    let result = arlm_core::run_rlm_engine(input, llm_backend)
        .await
        .context("RLM engine failed")?;

    progress.finish_and_clear();

    match config.format {
        Format::Json => {
            let output = crate::output::json::JsonOutput::ok()
                .with_data(serde_json::json!({
                    "run_id": result.run_id,
                    "task": config.task,
                    "result": result.final_output,
                    "duration_ms": result.stats.duration_ms,
                    "nodes_visited": result.stats.nodes_visited,
                    "max_depth": result.stats.max_depth_seen,
                }));
            output.print();
        }
        Format::Tree => {
            let tree = crate::output::tree::render_tree(
                &result.run_id,
                config.task,
                result.stats.max_depth_seen,
            );
            print!("{tree}");
            println!("\n{}", result.final_output);
        }
        Format::Markdown => {
            let md = crate::output::markdown::render_run_result(
                config.task,
                &result.final_output,
                result.stats.duration_ms,
            );
            print!("{md}");
        }
        Format::Prompt => {
            println!("## RLM Result\n\n**Task:** {}\n\n{}", config.task, result.final_output);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        };
        let result = execute(config).await;
        assert!(result.is_err());
    }
}
