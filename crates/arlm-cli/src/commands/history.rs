use std::path::Path;

use anyhow::{Context, Result};

use crate::output::{self, Format};
use crate::util::data_dir;

pub struct HistoryConfig<'a> {
    pub limit: usize,
    pub project: &'a Path,
    pub format: Format,
}

#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
pub fn execute(config: HistoryConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_history");

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    let history = arlm_memory::HistoryManager::new(storage.clone());
    let limit = i64::try_from(config.limit).unwrap_or(i64::MAX);
    let records = history
        .recent(None, limit)
        .context("failed to get history")?;

    let runs = storage.list_runs(limit).context("failed to get runs")?;

    match config.format {
        Format::Json => {
            let query_items: Vec<serde_json::Value> = records
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id,
                        "query": r.query,
                        "query_type": r.query_type,
                        "results_count": r.results_count,
                        "duration_ms": r.duration_ms,
                        "used_by": r.used_by,
                        "created_at": r.created_at,
                    })
                })
                .collect();
            let run_items: Vec<serde_json::Value> = runs
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "run_id": r.id,
                        "task": r.task,
                        "status": r.status,
                        "backend": r.backend,
                        "duration_ms": r.duration_ms,
                        "total_cost": r.total_cost,
                        "total_tokens": r.total_tokens,
                        "nodes_visited": r.nodes_visited,
                    })
                })
                .collect();
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "queries": query_items,
                "queries_count": records.len(),
                "runs": run_items,
                "runs_count": runs.len(),
            }));
            output.print();
        }
        Format::Tree => {
            if !runs.is_empty() {
                output::info(&format!("Recent {} run(s):", runs.len()));
                for r in &runs {
                    let date = chrono::DateTime::from_timestamp(r.started_at.unwrap_or(0), 0)
                        .map_or_else(|| "-".to_string(), |dt| dt.format("%Y-%m-%d %H:%M").to_string());
                    let dur = r.duration_ms.map_or_else(|| "-".to_string(), |d| format!("{d}ms"));
                    println!(
                        "  {} — {} ({}, {}, ${:.4})",
                        r.id,
                        r.task,
                        r.status.as_deref().unwrap_or("unknown"),
                        dur,
                        r.total_cost,
                    );
                }
                println!();
            }

            if !records.is_empty() {
                output::info(&format!("Recent {} query(ies):", records.len()));
                let rows: Vec<crate::output::tree::HistoryRow> = records
                    .iter()
                    .map(|r| {
                        let date = chrono::DateTime::from_timestamp(r.created_at, 0).map_or_else(
                            || r.created_at.to_string(),
                            |dt| dt.format("%Y-%m-%d %H:%M").to_string(),
                        );
                        let duration = r
                            .duration_ms
                            .map_or_else(|| "-".to_string(), |d| format!("{d}ms"));
                        let results = r
                            .results_count
                            .map_or_else(|| "-".to_string(), |c| c.to_string());
                        crate::output::tree::HistoryRow {
                            date,
                            query: r.query.clone(),
                            duration,
                            results,
                        }
                    })
                    .collect();
                print!("{}", crate::output::tree::render_history_table(&rows));
            }

            if runs.is_empty() && records.is_empty() {
                output::warn("No history found.");
            }
        }
        Format::Markdown => {
            if !runs.is_empty() {
                println!("## Recent Runs\n");
                println!("| Run ID | Task | Status | Cost | Duration |");
                println!("|--------|------|--------|------|----------|");
                for r in &runs {
                    let task_display = if r.task.len() > 30 {
                        format!("{}...", &r.task[..30])
                    } else {
                        r.task.clone()
                    };
                    let dur = r.duration_ms.map_or_else(|| "-".to_string(), |d| format!("{d}ms"));
                    println!(
                        "| {} | {} | {} | ${:.4} | {} |",
                        r.id,
                        task_display,
                        r.status.as_deref().unwrap_or("-"),
                        r.total_cost,
                        dur,
                    );
                }
                println!();
            }

            if !records.is_empty() {
                println!("## Query History\n");
                println!("| Date | Query | Duration | Results |");
                println!("|------|-------|----------|---------|");
                for r in &records {
                    let date = chrono::DateTime::from_timestamp(r.created_at, 0).map_or_else(
                        || r.created_at.to_string(),
                        |dt| dt.format("%Y-%m-%d %H:%M").to_string(),
                    );
                    let dur = r
                        .duration_ms
                        .map_or_else(|| "-".to_string(), |d| format!("{d}ms"));
                    let res = r
                        .results_count
                        .map_or_else(|| "-".to_string(), |c| c.to_string());
                    let query_display = if r.query.len() > 37 {
                        format!("{}...", &r.query[..37])
                    } else {
                        r.query.clone()
                    };
                    println!("| {date} | {query_display} | {dur} | {res} |");
                }
            }

            if runs.is_empty() && records.is_empty() {
                println!("No history found.");
            }
        }
        Format::Prompt => {
            if !runs.is_empty() {
                println!("Recent runs:");
                for r in &runs {
                    println!(
                        "  {} — {} ({}, ${:.4})",
                        r.id,
                        r.task,
                        r.status.as_deref().unwrap_or("unknown"),
                        r.total_cost,
                    );
                }
                println!();
            }

            if !records.is_empty() {
                println!("Recent queries:");
                for r in &records {
                    let results = r.results_count.unwrap_or(0);
                    println!("  - [{}] {} ({results} results)", r.created_at, r.query);
                }
            }

            if runs.is_empty() && records.is_empty() {
                println!("No history found.");
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
    fn test_history_empty() {
        let tmp = TempDir::new().unwrap();
        // SAFETY: test-only, single-threaded
        unsafe { std::env::set_var("ARLM_DATA_DIR", tmp.path()) };
        let project_path = tmp.path().join("nonexistent");
        let config = HistoryConfig {
            limit: 10,
            project: project_path.as_path(),
            format: Format::Json,
        };
        let result = execute(config);
        assert!(result.is_ok());
    }
}
