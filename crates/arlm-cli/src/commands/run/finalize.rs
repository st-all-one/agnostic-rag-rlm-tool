use anyhow::{Context, Result};
use tracing::{debug, instrument};

use arlm_core::RlmRunResult;

use crate::commands::run::config::RunConfig;
use crate::output::{Format, json, markdown, tree};
use crate::util::data_dir;

/// Persist a completed run (and its node tree) to the local storage database.
#[allow(clippy::missing_errors_doc)]
#[instrument(skip(result, config), fields(run_id = %result.run_id))]
pub fn persist_run(result: &RlmRunResult, config: &RunConfig<'_>) -> Result<()> {
    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    let total_usage = result.root.total_usage();
    let partial = if result.final_output.is_empty() {
        None
    } else {
        Some(result.final_output.as_str())
    };

    fn convert_node(node: &arlm_core::RlmNode) -> arlm_storage::sqlite::runs::FlatNode {
        arlm_storage::sqlite::runs::FlatNode {
            node_id: node.id.clone(),
            depth: node.depth,
            task: node.task.clone(),
            status: node.status.to_string(),
            node_type: node.decision.as_ref().map(|d| d.action.to_string()),
            cost_usd: node.usage.cost_usd,
            tokens: node.usage.tokens,
            errors: node.usage.errors,
            started_at_ms: node.started_at_ms,
            finished_at_ms: node.finished_at_ms,
            result: node.result.clone(),
            error: node.error.clone(),
            children: node.children.iter().map(convert_node).collect(),
        }
    }

    let flat_root = convert_node(&result.root);
    storage.insert_run(
        &result.run_id,
        config.task,
        &result.backend,
        "auto",
        "completed",
        "arlm",
        result.root.started_at_ms,
        result.stats.duration_ms,
        total_usage.cost_usd,
        total_usage.tokens,
        result.stats.nodes_visited,
        result.stats.max_depth_seen,
        result.stats.nodes_visited,
        partial,
        None,
        Some(&flat_root),
    )?;
    debug!("persisted run to storage");
    Ok(())
}

/// Record the run result into the session store, if a `--session` id was given.
pub fn save_session(result: &RlmRunResult, config: &RunConfig<'_>) {
    let Some(sid) = config.session_id else {
        return;
    };
    let Ok(stor) = arlm_storage::Storage::open(&data_dir()) else {
        return;
    };
    let Ok(mgr) = arlm_memory::SessionManager::new(stor) else {
        return;
    };
    let _ = mgr.add_context(sid, &result.final_output);
    let _ = mgr.record_query(sid, config.task, Some(&result.final_output));
}

/// Render the run result according to the requested output [`Format`].
#[must_use]
pub fn print_output(result: &RlmRunResult, config: &RunConfig<'_>) -> String {
    match config.format {
        Format::Json => {
            let output = json::JsonOutput::ok().with_data(serde_json::json!({
                "run_id": result.run_id,
                "task": config.task,
                "result": result.final_output,
                "duration_ms": result.stats.duration_ms,
                "nodes_visited": result.stats.nodes_visited,
                "max_depth": result.stats.max_depth_seen,
            }));
            output.to_json_string()
        }
        Format::Tree => {
            let rendered =
                tree::render_tree(&result.run_id, config.task, result.stats.max_depth_seen);
            format!("{rendered}\n{}", result.final_output)
        }
        Format::Markdown => {
            markdown::render_run_result(config.task, &result.final_output, result.stats.duration_ms)
        }
        Format::Prompt => {
            format!(
                "## RLM Result\n\n**Task:** {}\n\n{}",
                config.task, result.final_output
            )
        }
    }
}
