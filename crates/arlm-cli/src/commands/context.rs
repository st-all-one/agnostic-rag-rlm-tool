use std::path::Path;

use anyhow::{Context, Result};
use tracing::{debug, warn};

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
    pub persist: bool,
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
    debug!(tier = config.tier, resolved = ?tier, "resolved context tier");

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

    let rendered = match config.format {
        Format::Json => {
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "task": config.task,
                "project": pname,
                "context": context,
                "results_count": results.len(),
            }));
            output.to_json_string()
        }
        Format::Tree => {
            format!("Context for: {}\n\n{context}", config.task)
        }
        Format::Markdown | Format::Prompt => context,
    };

    print!("{rendered}");

    if config.persist {
        if let Err(e) = crate::commands::persist::save_page(
            config.task,
            &rendered,
            config.project,
            config.format,
        ) {
            warn!(error = %e, "failed to persist context output");
        }
    }

    Ok(())
}
