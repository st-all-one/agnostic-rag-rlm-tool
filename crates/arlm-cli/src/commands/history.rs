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

    let history = arlm_memory::HistoryManager::new(storage);
    let limit = i64::try_from(config.limit).unwrap_or(i64::MAX);
    let records = history
        .recent(None, limit)
        .context("failed to get history")?;

    match config.format {
        Format::Json => {
            let items: Vec<serde_json::Value> = records
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
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "history": items,
                "count": records.len(),
            }));
            output.print();
        }
        Format::Tree => {
            if records.is_empty() {
                output::warn("No query history found.");
            } else {
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
        }
        Format::Markdown => {
            println!("# Query History\n");
            if records.is_empty() {
                println!("No query history found.");
            } else {
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
        }
        Format::Prompt => {
            if records.is_empty() {
                println!("No query history found.");
            } else {
                println!("Recent queries:");
                for r in &records {
                    let results = r.results_count.unwrap_or(0);
                    println!("  - [{}] {} ({results} results)", r.created_at, r.query);
                }
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
