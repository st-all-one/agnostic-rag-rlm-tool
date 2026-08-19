use std::path::Path;

use anyhow::{Context, Result};

use crate::output::{self, Format};
use crate::util::project_dirs;

pub struct ConsolidateConfig<'a> {
    pub project: &'a Path,
    pub format: Format,
    pub verbose: bool,
}

#[allow(clippy::needless_pass_by_value)]
pub fn execute(config: ConsolidateConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_consolidate");

    let project_name = config
        .project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");

    let data_dir = project_dirs().join(project_name);

    let storage = arlm_storage::Storage::open(&data_dir).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(project_name)
        .context("failed to check buffer")?
        .context("project not found. Run `arlm index` first.")?;

    if config.verbose {
        output::info(&format!("Consolidating project: {project_name}..."));
    }

    let engine = arlm_memory::ConsolidationEngine::new(storage);
    let opts = arlm_memory::ConsolidateOptions::default();
    let result = engine
        .consolidate(buffer.id, &opts)
        .context("consolidation failed")?;

    match config.format {
        Format::Json => {
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "project": project_name,
                "duplicate_chunks_removed": result.duplicate_chunks_removed,
                "low_confidence_patterns_removed": result.low_confidence_patterns_removed,
            }));
            output.print();
        }
        Format::Tree => {
            output::success(&format!("Consolidation complete for '{project_name}'"));
            println!("  Duplicates removed: {}", result.duplicate_chunks_removed);
            println!(
                "  Low-confidence patterns removed: {}",
                result.low_confidence_patterns_removed
            );
        }
        Format::Markdown => {
            println!(
                "# Consolidation Complete\n\n- **Project:** {project_name}\n- **Duplicates removed:** {}\n- **Low-confidence patterns removed:** {}\n",
                result.duplicate_chunks_removed,
                result.low_confidence_patterns_removed,
            );
        }
        Format::Prompt => {
            println!(
                "Consolidation complete for {project_name}: {} duplicates, {} patterns removed.",
                result.duplicate_chunks_removed,
                result.low_confidence_patterns_removed,
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_consolidate_no_project() {
        let tmp = TempDir::new().unwrap();
        let project_path = tmp.path().join("nonexistent");
        let config = ConsolidateConfig {
            project: project_path.as_path(),
            format: Format::Json,
            verbose: false,
        };
        let result = execute(config);
        assert!(result.is_err());
    }
}
