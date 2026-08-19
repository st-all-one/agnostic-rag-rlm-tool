use std::path::Path;

use anyhow::{Context, Result};

use crate::output::{self, Format};
use crate::util::project_dirs;

pub struct SearchConfig<'a> {
    pub query: &'a str,
    pub top_k: usize,
    pub file_pattern: Option<&'a str>,
    pub min_score: Option<f32>,
    pub project: &'a Path,
    pub format: Format,
    pub verbose: bool,
}

#[allow(clippy::needless_pass_by_value)]
pub fn execute(config: SearchConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_search");

    let project_name = config
        .project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");

    let data_dir = project_dirs().join(project_name);

    if config.verbose {
        output::info(&format!("Searching '{}'...", config.query));
    }

    let storage = arlm_storage::Storage::open(&data_dir).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(project_name)
        .context("failed to check buffer")?
        .context("project not found. Run `arlm index` first.")?;

    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
    let hybrid = arlm_search::HybridSearch::new(bm25, None, None);

    let results = hybrid
        .search_fts(config.query, buffer.id, config.top_k, None)
        .context("FTS search failed")?;

    let search_results =
        arlm_search::build_search_results(&storage, &results).context("failed to build results")?;

    match config.format {
        Format::Json => {
            let items: Vec<serde_json::Value> = search_results
                .iter()
                .filter(|r| config.min_score.is_none_or(|min| r.score >= min))
                .filter(|r| {
                    #[allow(clippy::unnecessary_map_or)]
                    config
                        .file_pattern
                        .as_ref()
                        .map_or(true, |pat| r.file_path.contains(&**pat))
                })
                .map(|r| {
                    serde_json::json!({
                        "chunk_id": r.chunk_id,
                        "file": r.file_path,
                        "line_start": r.line_start,
                        "line_end": r.line_end,
                        "score": r.score,
                        "content": r.content,
                        "language": r.language,
                    })
                })
                .collect();

            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "query": config.query,
                "results": items,
                "count": search_results.len(),
            }));
            output.print();
        }
        Format::Tree => {
            let items: Vec<crate::output::tree::SearchResultItem> = search_results
                .iter()
                .map(|r| crate::output::tree::SearchResultItem {
                    file_path: r.file_path.clone(),
                    line_start: r.line_start,
                    line_end: r.line_end,
                    score: r.score,
                })
                .collect();
            print!("{}", crate::output::tree::render_search_results(&items));
        }
        Format::Markdown => {
            let items: Vec<crate::output::markdown::SuperItem> = search_results
                .iter()
                .map(|r| crate::output::markdown::SuperItem {
                    file_path: r.file_path.clone(),
                    score: r.score,
                    content: r.content.clone(),
                    language: r.language.clone(),
                })
                .collect();
            print!("{}", crate::output::markdown::render_search_results(&items));
        }
        Format::Prompt => {
            let items: Vec<crate::output::prompt::PromptItem> = search_results
                .iter()
                .map(|r| crate::output::prompt::PromptItem {
                    file_path: r.file_path.clone(),
                    score: r.score,
                    content: r.content.clone(),
                    language: r.language.clone(),
                })
                .collect();
            print!("{}", crate::output::prompt::render_search_context(&items));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_search_no_project() {
        let tmp = TempDir::new().unwrap();
        let project_path = tmp.path().join("nonexistent");
        let config = SearchConfig {
            query: "test query",
            top_k: 10,
            file_pattern: None,
            min_score: None,
            project: project_path.as_path(),
            format: Format::Json,
            verbose: false,
        };
        let result = execute(config);
        assert!(result.is_err());
    }
}
