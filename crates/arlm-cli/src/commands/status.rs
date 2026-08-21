use std::path::Path;

use anyhow::{Context, Result};

use crate::output::{self, Format};
use crate::util::data_dir;

pub fn execute(run_id: Option<&str>, _project: &Path, format: Format) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_status");

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    storage.ensure_uuids().ok();

    if let Some(rid) = run_id {
        let run = storage.get_run(rid).context("failed to query run")?;

        match run {
            Some(r) => match format {
                Format::FullJson | Format::Jsonl => {
                    let output =
                        crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                            "run_id": r.id,
                            "task": r.task,
                            "status": r.status,
                            "backend": r.backend,
                            "duration_ms": r.duration_ms,
                            "total_cost": r.total_cost,
                            "total_tokens": r.total_tokens,
                            "nodes_visited": r.nodes_visited,
                            "error": r.error,
                        }));
                    output.print();
                }
                Format::Path => {
                    output::success(&format!("Run {}", r.id));
                    println!("  Task: {}", r.task);
                    println!("  Status: {}", r.status.as_deref().unwrap_or("unknown"));
                    println!("  Backend: {}", r.backend.as_deref().unwrap_or("-"));
                    if let Some(dur) = r.duration_ms {
                        println!("  Duration: {dur}ms");
                    }
                    println!("  Cost: ${:.4}", r.total_cost);
                    println!("  Tokens: {}", r.total_tokens);
                    println!("  Nodes: {}", r.nodes_visited.unwrap_or(0));
                    if let Some(e) = &r.error {
                        println!("  Error: {e}");
                    }
                }
                Format::Markdown => {
                    println!("# Run {}\n", r.id);
                    println!("- **Task:** {}", r.task);
                    println!("- **Status:** {}", r.status.as_deref().unwrap_or("unknown"));
                    println!("- **Backend:** {}", r.backend.as_deref().unwrap_or("-"));
                    println!("- **Cost:** ${:.4}", r.total_cost);
                    println!("- **Tokens:** {}", r.total_tokens);
                    println!("- **Nodes:** {}", r.nodes_visited.unwrap_or(0));
                }
                Format::Text => {
                    println!(
                        "Run {}: {} ({})",
                        r.id,
                        r.task,
                        r.status.as_deref().unwrap_or("unknown")
                    );
                    println!(
                        "  Cost: ${:.4}, Tokens: {}, Nodes: {}",
                        r.total_cost,
                        r.total_tokens,
                        r.nodes_visited.unwrap_or(0)
                    );
                }
            },
            None => match format {
                Format::FullJson | Format::Jsonl => {
                    let output =
                        crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                            "run_id": rid,
                            "status": "not_found",
                            "message": "Run not found",
                        }));
                    output.print();
                }
                _ => {
                    output::warn(&format!("Run {rid} not found"));
                }
            },
        }
        return Ok(());
    }

    // No run_id: show project status + recent runs
    let buffers = storage.list_buffers().context("failed to list buffers")?;
    let runs = storage.list_runs(10).context("failed to list runs")?;

    match format {
        Format::FullJson | Format::Jsonl => {
            let projects: Vec<serde_json::Value> = buffers
                .iter()
                .map(|b| {
                    serde_json::json!({
                        "id": b.id,
                        "name": b.name,
                        "path": b.path,
                        "total_chunks": b.total_chunks,
                        "total_files": b.total_files,
                        "last_indexed_at": b.last_indexed_at,
                    })
                })
                .collect();
            let recent_runs: Vec<serde_json::Value> = runs
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "run_id": r.id,
                        "task": r.task,
                        "status": r.status,
                        "duration_ms": r.duration_ms,
                        "total_cost": r.total_cost,
                    })
                })
                .collect();
            let total_cost = storage.total_cost().unwrap_or(0.0);
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "projects": projects,
                "projects_count": buffers.len(),
                "recent_runs": recent_runs,
                "runs_count": runs.len(),
                "total_cost_usd": total_cost,
            }));
            output.print();
        }
        Format::Path => {
            if buffers.is_empty() {
                output::warn("No indexed projects found.");
            } else {
                output::success(&format!("{} indexed project(s):", buffers.len()));
                for b in &buffers {
                    let last_idx = b.last_indexed_at.map_or_else(
                        || "never indexed".to_string(),
                        |t| format!("indexed at {t}"),
                    );
                    println!(
                        "  {} — {} chunks, {} files ({})",
                        console::Style::new().bold().apply_to(&b.name),
                        b.total_chunks,
                        b.total_files,
                        last_idx,
                    );
                }
            }
            if !runs.is_empty() {
                println!();
                output::info(&format!("Recent {} run(s):", runs.len()));
                for r in &runs {
                    let _dur = r
                        .duration_ms
                        .map_or_else(|| "-".to_string(), |d| format!("{d}ms"));
                    println!(
                        "  {} — {} ({}, ${:.4})",
                        r.id,
                        r.task,
                        r.status.as_deref().unwrap_or("unknown"),
                        r.total_cost,
                    );
                }
            }
        }
        Format::Markdown => {
            println!("# Project Status\n");
            if buffers.is_empty() {
                println!("No indexed projects found.");
            } else {
                for b in &buffers {
                    println!(
                        "## {}\n- **Chunks:** {}\n- **Files:** {}\n",
                        b.name, b.total_chunks, b.total_files
                    );
                }
            }
            if !runs.is_empty() {
                println!("\n## Recent Runs\n");
                println!("| Run ID | Task | Status | Cost |");
                println!("|--------|------|--------|------|");
                for r in &runs {
                    let task_display = if r.task.len() > 30 {
                        format!("{}...", &r.task[..30])
                    } else {
                        r.task.clone()
                    };
                    println!(
                        "| {} | {} | {} | ${:.4} |",
                        r.id,
                        task_display,
                        r.status.as_deref().unwrap_or("-"),
                        r.total_cost,
                    );
                }
            }
        }
        Format::Text => {
            if buffers.is_empty() {
                println!("No indexed projects found. Run `arlm index` first.");
            } else {
                println!("Indexed projects:");
                for b in &buffers {
                    println!(
                        "  {} ({} chunks, {} files)",
                        b.name, b.total_chunks, b.total_files
                    );
                }
            }
            if !runs.is_empty() {
                println!("\nRecent runs:");
                for r in &runs {
                    println!(
                        "  {} — {} ({})",
                        r.id,
                        r.task,
                        r.status.as_deref().unwrap_or("unknown"),
                    );
                }
            }
        }
    }

    Ok(())
}
