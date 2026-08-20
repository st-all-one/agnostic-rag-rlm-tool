use std::path::Path;

use anyhow::{Context, Result};

use crate::output::Format;
use crate::util::data_dir;

pub fn execute(run_id: &str, _project: &Path, format: Format) -> Result<()> {
    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    storage.ensure_uuids().ok();

    let run = storage.get_run(run_id).context("failed to get run")?;

    match run {
        Some(_run) => {
            // Mark run as cancelled in DB
            storage.cancel_run(run_id).context("failed to cancel run")?;

            match format {
                Format::Json => {
                    let output =
                        crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                            "run_id": run_id,
                            "status": "cancelled",
                        }));
                    output.print();
                }
                _ => {
                    crate::output::success(&format!("Run {run_id} cancelled"));
                }
            }
        }
        None => {
            crate::output::error(&format!("Run {run_id} not found"));
        }
    }

    Ok(())
}
