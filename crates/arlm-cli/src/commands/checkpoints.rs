use anyhow::{Context, Result};

use crate::output::Format;
use crate::util::data_dir;

pub fn execute(run_id: Option<&str>, format: Format) -> Result<()> {
    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    storage.ensure_uuids().ok();

    match run_id {
        Some(id) => {
            // Get checkpoints for a specific run
            let trajectories = storage
                .get_trajectories_by_task_hash(id)
                .context("failed to get trajectories")?;

            match format {
                Format::Json => {
                    let output =
                        crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                            "run_id": id,
                            "checkpoints": trajectories.len(),
                        }));
                    output.print();
                }
                _ => {
                    if trajectories.is_empty() {
                        crate::output::info(&format!("No checkpoints for run {id}"));
                    } else {
                        crate::output::success(&format!(
                            "Run {id} has {} checkpoints",
                            trajectories.len()
                        ));
                    }
                }
            }
        }
        None => {
            // List all runs with their checkpoint counts
            let runs = storage.list_runs(20).context("failed to list runs")?;

            match format {
                Format::Json => {
                    let runs_data: Vec<_> = runs
                        .iter()
                        .map(|r| {
                            let count = storage
                                .get_trajectories_by_task_hash(&r.id)
                                .map(|t| t.len())
                                .unwrap_or(0);
                            serde_json::json!({
                                "run_id": r.id,
                                "task": r.task,
                                "status": r.status,
                                "checkpoints": count,
                            })
                        })
                        .collect();
                    let output = crate::output::json::JsonOutput::ok()
                        .with_data(serde_json::json!({ "runs": runs_data }));
                    output.print();
                }
                _ => {
                    if runs.is_empty() {
                        crate::output::info("No runs found");
                    } else {
                        for run in &runs {
                            let count = storage
                                .get_trajectories_by_task_hash(&run.id)
                                .map(|t| t.len())
                                .unwrap_or(0);
                            println!(
                                "{} | {} | {} checkpoints",
                                run.id,
                                run.status.as_deref().unwrap_or("unknown"),
                                count
                            );
                        }
                    }
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
    fn test_checkpoints_empty() {
        let tmp = TempDir::new().unwrap();
        let result = execute(None, Format::Json);
        assert!(result.is_ok());
    }
}
