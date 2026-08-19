use std::path::Path;

use anyhow::{Context, Result};

use crate::output::{self, Format};
use crate::util::data_dir;

pub fn execute(run_id: Option<&str>, _project: &Path, format: Format) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_status");

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    storage.ensure_uuids().ok();

    let buffers = storage.list_buffers().context("failed to list buffers")?;

    if let Some(rid) = run_id {
        match format {
            Format::Json => {
                let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                    "run_id": rid,
                    "status": "unknown",
                    "message": "Run tracking not yet persisted",
                }));
                output.print();
            }
            _ => {
                output::info(&format!(
                    "Run {rid}: status not available (run tracking not yet persisted)"
                ));
            }
        }
        return Ok(());
    }

    match format {
        Format::Json => {
            let items: Vec<serde_json::Value> = buffers
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
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "projects": items,
                "count": buffers.len(),
            }));
            output.print();
        }
        Format::Tree => {
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
        }
        Format::Prompt => {
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
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_status_empty() {
        let tmp = TempDir::new().unwrap();
        // SAFETY: test-only, single-threaded
        unsafe { std::env::set_var("ARLM_DATA_DIR", tmp.path()) };
        let project_path = tmp.path().join("nonexistent");
        let result = execute(None, project_path.as_path(), Format::Json);
        assert!(result.is_ok());
    }
}
