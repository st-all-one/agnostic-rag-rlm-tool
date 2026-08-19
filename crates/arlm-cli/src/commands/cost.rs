use std::path::Path;

use crate::output::{self, Format};

pub fn execute(run_id: Option<&str>, project: &Path, format: Format) {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_cost");

    let project_name = project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");

    match format {
        Format::Json => {
            let data = if let Some(rid) = run_id {
                serde_json::json!({
                    "run_id": rid,
                    "cost_usd": 0.0,
                    "message": "Cost tracking not yet persisted",
                })
            } else {
                serde_json::json!({
                    "total_cost_usd": 0.0,
                    "projects": [{
                        "name": project_name,
                        "cost_usd": 0.0,
                    }],
                    "message": "Cost tracking not yet implemented",
                })
            };
            let output = crate::output::json::JsonOutput::ok().with_data(data);
            output.print();
        }
        Format::Tree => {
            if let Some(rid) = run_id {
                output::info(&format!("Cost for run {rid}: tracking not yet implemented"));
            } else {
                output::info(&format!(
                    "Cost summary for project '{project_name}': tracking not yet implemented"
                ));
            }
        }
        Format::Markdown => {
            println!("# Cost Summary\n\nCost tracking is not yet implemented.");
        }
        Format::Prompt => {
            println!("Cost tracking is not yet implemented.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_cost_no_run() {
        let tmp = TempDir::new().unwrap();
        execute(None, tmp.path(), Format::Json);
    }
}
