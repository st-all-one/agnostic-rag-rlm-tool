use std::path::Path;

use anyhow::{Context, Result};

use crate::output::{self, Format};
use crate::util::{data_dir, project_name};

pub struct ContextConfig<'a> {
    pub task: &'a str,
    pub top_k: usize,
    pub all: bool,
    pub tier: &'a str,
    pub max_tokens: Option<u32>,
    pub project: &'a Path,
    pub format: Format,
    pub verbose: bool,
}

fn parse_tier(tier: &str) -> arlm_search::SearchTier {
    match tier {
        "fts" => arlm_search::SearchTier::Fts,
        "entity" => arlm_search::SearchTier::Entity,
        "vector" => arlm_search::SearchTier::Vector,
        "llm_rerank" => arlm_search::SearchTier::LlmRerank,
        _ => arlm_search::SearchTier::Entity, // auto defaults to entity
    }
}

#[allow(clippy::needless_pass_by_value)]
pub async fn execute(config: ContextConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_context");

    let pname = project_name(config.project);

    if config.verbose {
        output::info(&format!("Building context for task: {}...", config.task));
    }

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    storage.ensure_uuids().ok();

    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;

    let tier = parse_tier(config.tier);

    let results = if config.all {
        let hybrid = arlm_search::HybridSearch::new(bm25, None, None);
        hybrid
            .search_all(config.task, config.top_k, &storage)
            .context("cross-project search failed")?
    } else {
        let buffer = storage
            .get_buffer_by_name(&pname)
            .context("failed to check buffer")?
            .context("project not found. Run `arlm index` first.")?;

        if matches!(tier, arlm_search::SearchTier::Fts) {
            let hybrid = arlm_search::HybridSearch::new(bm25, None, None);
            hybrid
                .search_fts(config.task, buffer.id, config.top_k, None)
                .context("FTS search failed")?
        } else {
            let entity = arlm_search::EntitySearch::new(storage.clone()).ok();
            let hybrid = arlm_search::HybridSearch::new(bm25, entity, None);
            let options = arlm_search::SearchOptions {
                tier,
                top_k: config.top_k,
            };
            hybrid
                .search(config.task, None, buffer.id, &options, None, Some(&storage))
                .await
                .context("hybrid search failed")?
        }
    };

    let search_format = match config.format {
        Format::Json => arlm_search::OutputFormat::Json,
        Format::Markdown => arlm_search::OutputFormat::Markdown,
        _ => arlm_search::OutputFormat::Prompt,
    };

    let context = arlm_search::build_context(&storage, &results, search_format, config.max_tokens)
        .context("failed to build context")?;

    match config.format {
        Format::Json => {
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "task": config.task,
                "project": pname,
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

    #[tokio::test]
    async fn test_context_no_project() {
        let tmp = TempDir::new().unwrap();
        // SAFETY: test-only, single-threaded
        unsafe { std::env::set_var("ARLM_DATA_DIR", tmp.path()) };
        let project_path = tmp.path().join("nonexistent");
        let config = ContextConfig {
            task: "fix auth bug",
            top_k: 10,
            all: false,
            tier: "auto",
            max_tokens: None,
            project: project_path.as_path(),
            format: Format::Json,
            verbose: false,
        };
        let result = execute(config).await;
        assert!(result.is_err());
    }
}
