use std::path::Path;

use anyhow::{Context, Result};

use crate::output::Format;
use crate::util::data_dir;

pub fn execute(action: &str, project: &Path, format: Format) -> Result<()> {
    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    storage.ensure_uuids().ok();

    let pname = crate::util::project_name(project);

    match action {
        "init" => {
            // Initialize wiki directory with git
            let wiki_dir = data_dir().join("wiki").join(&pname);
            std::fs::create_dir_all(&wiki_dir)
                .with_context(|| format!("failed to create wiki dir at {}", wiki_dir.display()))?;

            // Initialize git repo
            std::process::Command::new("git")
                .args(["init"])
                .current_dir(&wiki_dir)
                .output()
                .context("failed to init git repo")?;

            // Create .gitignore
            std::fs::write(wiki_dir.join(".gitignore"), "*.tmp\n")
                .context("failed to create .gitignore")?;

            match format {
                Format::Json => {
                    let output =
                        crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                            "action": "init",
                            "wiki_dir": wiki_dir,
                        }));
                    output.print();
                }
                _ => {
                    crate::output::success(&format!("Wiki initialized at {}", wiki_dir.display()));
                }
            }
        }
        "commit" => {
            // Commit all changes in wiki directory
            let wiki_dir = data_dir().join("wiki").join(&pname);
            if !wiki_dir.exists() {
                crate::output::error("Wiki not initialized, run 'arlm wiki init' first");
                return Ok(());
            }

            std::process::Command::new("git")
                .args(["add", "."])
                .current_dir(&wiki_dir)
                .output()
                .context("failed to git add")?;

            let output = std::process::Command::new("git")
                .args(["commit", "-m", "Update wiki"])
                .current_dir(&wiki_dir)
                .output()
                .context("failed to git commit")?;

            match format {
                Format::Json => {
                    let output =
                        crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                            "action": "commit",
                            "output": String::from_utf8_lossy(&output.stdout),
                        }));
                    output.print();
                }
                _ => {
                    crate::output::success("Wiki changes committed");
                }
            }
        }
        "log" => {
            // Show git log
            let wiki_dir = data_dir().join("wiki").join(&pname);
            if !wiki_dir.exists() {
                crate::output::error("Wiki not initialized, run 'arlm wiki init' first");
                return Ok(());
            }

            let output = std::process::Command::new("git")
                .args(["log", "--oneline", "-10"])
                .current_dir(&wiki_dir)
                .output()
                .context("failed to git log")?;

            match format {
                Format::Json => {
                    let output =
                        crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                            "action": "log",
                            "output": String::from_utf8_lossy(&output.stdout),
                        }));
                    output.print();
                }
                _ => {
                    println!("{}", String::from_utf8_lossy(&output.stdout));
                }
            }
        }
        _ => {
            crate::output::error(&format!("Unknown wiki action: {action}"));
        }
    }

    Ok(())
}
