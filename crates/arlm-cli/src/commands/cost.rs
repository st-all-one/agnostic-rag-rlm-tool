use std::path::Path;

use anyhow::{Context, Result};

use crate::output::{self, Format};
use crate::util::data_dir;

pub fn execute(run_id: Option<&str>, agent: Option<&str>, _project: &Path, format: Format) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_cost");

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    if let Some(rid) = run_id {
        let run = storage.get_run(rid).context("failed to query run")?;
        let usage = storage.get_run_model_usage(rid).context("failed to query model usage")?;

        match run {
            Some(r) => match format {
                Format::Json => {
                    let models: Vec<serde_json::Value> = usage
                        .iter()
                        .map(|u| {
                            serde_json::json!({
                                "model": u.model,
                                "calls": u.calls,
                                "input_tokens": u.input_tokens,
                                "output_tokens": u.output_tokens,
                                "cost": u.cost,
                            })
                        })
                        .collect();
                    let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                        "run_id": r.id,
                        "task": r.task,
                        "agent": r.agent,
                        "total_cost_usd": r.total_cost,
                        "total_tokens": r.total_tokens,
                        "models": models,
                    }));
                    output.print();
                }
                Format::Tree => {
                    output::success(&format!("Cost for run {}:", r.id));
                    println!("  Task: {}", r.task);
                    if let Some(ref a) = r.agent {
                        println!("  Agent: {a}");
                    }
                    println!("  Total cost: ${:.4}", r.total_cost);
                    println!("  Total tokens: {}", r.total_tokens);
                    if !usage.is_empty() {
                        println!("\n  Model breakdown:");
                        for u in &usage {
                            println!(
                                "    {} — {} calls, ${:.4}, {} tokens",
                                u.model, u.calls, u.cost, u.input_tokens
                            );
                        }
                    }
                }
                Format::Markdown => {
                    println!("# Cost for Run {}\n", r.id);
                    println!("- **Task:** {}", r.task);
                    if let Some(ref a) = r.agent {
                        println!("- **Agent:** {a}");
                    }
                    println!("- **Total Cost:** ${:.4}", r.total_cost);
                    println!("- **Total Tokens:** {}", r.total_tokens);
                    if !usage.is_empty() {
                        println!("\n## Model Usage\n");
                        println!("| Model | Calls | Input Tokens | Cost |");
                        println!("|-------|-------|--------------|------|");
                        for u in &usage {
                            println!("| {} | {} | {} | ${:.4} |", u.model, u.calls, u.input_tokens, u.cost);
                        }
                    }
                }
                Format::Prompt => {
                    println!("Cost for run {}: ${:.4} ({} tokens)", r.id, r.total_cost, r.total_tokens);
                    for u in &usage {
                        println!("  {} — {} calls, ${:.4}", u.model, u.calls, u.cost);
                    }
                }
            },
            None => {
                match format {
                    Format::Json => {
                        let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                            "run_id": rid,
                            "status": "not_found",
                            "message": "Run not found",
                        }));
                        output.print();
                    }
                    _ => {
                        output::warn(&format!("Run {rid} not found"));
                    }
                }
            }
        }
        return Ok(());
    }

    // No run_id: show total cost across all runs (optionally filtered by agent)
    let total = storage.total_cost().context("failed to get total cost")?;
    let runs = storage.list_runs(50).context("failed to list runs")?;

    let filtered_runs: Vec<_> = if let Some(a) = agent {
        runs.iter().filter(|r| r.agent.as_deref() == Some(a)).collect()
    } else {
        runs.iter().collect()
    };

    match format {
        Format::Json => {
            let items: Vec<serde_json::Value> = filtered_runs
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "run_id": r.id,
                        "task": r.task,
                        "agent": r.agent,
                        "total_cost_usd": r.total_cost,
                        "total_tokens": r.total_tokens,
                        "duration_ms": r.duration_ms,
                    })
                })
                .collect();
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "total_cost_usd": total,
                "runs": items,
                "runs_count": filtered_runs.len(),
                "agent_filter": agent,
            }));
            output.print();
        }
        Format::Tree => {
            if let Some(a) = agent {
                output::success(&format!("Cost for agent '{a}':"));
            } else {
                output::success(&format!("Total cost: ${:.4}", total));
            }
            println!("  Across {} run(s)\n", filtered_runs.len());
            if !filtered_runs.is_empty() {
                for r in &filtered_runs {
                    println!(
                        "  {} — ${:.4} ({})",
                        r.id,
                        r.total_cost,
                        r.task,
                    );
                }
            }
        }
        Format::Markdown => {
            println!("# Cost Summary\n");
            if let Some(a) = agent {
                println!("**Agent:** {a}");
            }
            println!("**Total:** ${:.4} across {} run(s)\n", total, filtered_runs.len());
            if !filtered_runs.is_empty() {
                println!("| Run ID | Task | Agent | Cost | Tokens |");
                println!("|--------|------|-------|------|--------|");
                for r in filtered_runs.iter() {
                    let task_display = if r.task.len() > 30 {
                        format!("{}...", &r.task[..30])
                    } else {
                        r.task.clone()
                    };
                    let agent_display = r.agent.as_deref().unwrap_or("-");
                    println!("| {} | {} | {} | ${:.4} | {} |", r.id, task_display, agent_display, r.total_cost, r.total_tokens);
                }
            }
        }
        Format::Prompt => {
            if let Some(a) = agent {
                println!("Cost for agent '{a}': ${:.4} across {} run(s)", total, filtered_runs.len());
            } else {
                println!("Total cost: ${:.4} across {} run(s)", total, filtered_runs.len());
            }
            for r in &filtered_runs {
                println!("  {} — ${:.4} ({})", r.id, r.total_cost, r.task);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_cost_empty() {
        let tmp = TempDir::new().unwrap();
        // SAFETY: test-only, single-threaded
        unsafe { std::env::set_var("ARLM_DATA_DIR", tmp.path()) };
        let result = execute(None, None, tmp.path(), Format::Json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cost_with_run() {
        let tmp = TempDir::new().unwrap();
        // SAFETY: test-only, single-threaded
        unsafe { std::env::set_var("ARLM_DATA_DIR", tmp.path()) };

        let storage = arlm_storage::Storage::open(tmp.path()).unwrap();
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

        let result = execute(Some("run-001"), None, tmp.path(), Format::Json);
        assert!(result.is_ok());
    }
}
