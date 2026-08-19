use std::path::Path;

use anyhow::{Context, Result};

use crate::output::{self, Format};
use crate::util::project_dirs;

pub struct ContextConfig<'a> {
    pub task: &'a str,
    pub top_k: usize,
    pub project: &'a Path,
    pub format: Format,
    pub verbose: bool,
}

#[allow(clippy::needless_pass_by_value)]
pub fn execute(config: ContextConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_context");

    let project_name = config
        .project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");

    let data_dir = project_dirs().join(project_name);

    if config.verbose {
        output::info(&format!("Building context for task: {}...", config.task));
    }

    let storage = arlm_storage::Storage::open(&data_dir).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(project_name)
        .context("failed to check buffer")?
        .context("project not found. Run `arlm index` first.")?;

    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
    let hybrid = arlm_search::HybridSearch::new(bm25, None, None);

    let results = hybrid
        .search_fts(config.task, buffer.id, config.top_k, None)
        .context("FTS search failed")?;

    let search_format = match config.format {
        Format::Json => arlm_search::OutputFormat::Json,
        Format::Markdown => arlm_search::OutputFormat::Markdown,
        _ => arlm_search::OutputFormat::Prompt,
    };

    let context = arlm_search::build_context(&storage, &results, search_format)
        .context("failed to build context")?;

    match config.format {
        Format::Json => {
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "task": config.task,
                "project": project_name,
                "context": context,
                "results_count": results.len(),
            }));
            output.print();
        }
        Format::Tree => {
            output::success(&format!("Context for: {}", config.task));
            println!("\n{context}");
        }
        Format::Markdown | Format::Prompt => {
            print!("{context}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_context_no_project() {
        let tmp = TempDir::new().unwrap();
        let project_path = tmp.path().join("nonexistent");
        let config = ContextConfig {
            task: "fix auth bug",
            top_k: 10,
            project: project_path.as_path(),
            format: Format::Json,
            verbose: false,
        };
        let result = execute(config);
        assert!(result.is_err());
    }
}
