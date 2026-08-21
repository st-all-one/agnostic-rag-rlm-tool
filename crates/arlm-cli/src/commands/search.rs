use std::path::Path;

use anyhow::{Context, Result};
use tracing::{debug, warn};

use crate::output::{self, Format};
use crate::util::{data_dir, project_name};

pub struct SearchConfig<'a> {
    pub query: &'a str,
    pub top_k: usize,
    pub file_pattern: Option<&'a str>,
    pub min_score: Option<f32>,
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
pub async fn execute(config: SearchConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_search");

    let pname = project_name(config.project);

    if config.verbose {
        output::info(&format!("Searching '{}'...", config.query));
    }

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    storage.ensure_uuids().ok();

    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;

    let tier = parse_tier(config.tier);
    debug!(tier = config.tier, resolved = ?tier, "resolved search tier");

    let results = if config.all {
        let hybrid = arlm_search::HybridSearch::new(bm25, None, None);
        hybrid
            .search_all(config.query, config.top_k, &storage)
            .context("cross-project search failed")?
    } else {
        let buffer = storage
            .get_buffer_by_name(&pname)
            .context("failed to check buffer")?
            .context("project not found. Run `arlm index` first.")?;

        if matches!(tier, arlm_search::SearchTier::Fts) {
            let hybrid = arlm_search::HybridSearch::new(bm25, None, None);
            hybrid
                .search_fts(config.query, buffer.id, config.top_k, None)
                .context("FTS search failed")?
        } else {
            let entity = arlm_search::EntitySearch::new(storage.clone()).ok();
            let hybrid = arlm_search::HybridSearch::new(bm25, entity, None);
            let options = arlm_search::SearchOptions {
                tier,
                top_k: config.top_k,
            };
            hybrid
                .search(
                    config.query,
                    None,
                    buffer.id,
                    &options,
                    None,
                    Some(&storage),
                )
                .await
                .context("hybrid search failed")?
        }
    };

    let search_results = arlm_search::build_search_results(&storage, &results, config.max_tokens)
        .context("failed to build results")?;

    let rendered = match config.format {
        Format::FullJson => {
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
            output.to_json_string()
        }
        Format::Jsonl => {
            let pairs: Vec<(String, String)> = search_results
                .iter()
                .filter(|r| config.min_score.is_none_or(|min| r.score >= min))
                .filter(|r| {
                    config
                        .file_pattern
                        .as_ref()
                        .is_none_or(|pat| r.file_path.contains(pat))
                })
                .map(|r| (r.file_path.clone(), r.content.clone()))
                .collect();
            crate::output::jsonl::render_content_jsonl("query", config.query, &pairs)
        }
        Format::Path => {
            let items: Vec<crate::output::tree::SearchResultItem> = search_results
                .iter()
                .map(|r| crate::output::tree::SearchResultItem {
                    file_path: r.file_path.clone(),
                    line_start: r.line_start,
                    line_end: r.line_end,
                    score: r.score,
                })
                .collect();
            crate::output::tree::render_search_results(&items)
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
            crate::output::markdown::render_search_results(&items)
        }
        Format::Text => {
            let items: Vec<crate::output::prompt::PromptItem> = search_results
                .iter()
                .map(|r| crate::output::prompt::PromptItem {
                    file_path: r.file_path.clone(),
                    score: r.score,
                    content: r.content.clone(),
                    language: r.language.clone(),
                })
                .collect();
            crate::output::prompt::render_search_context(&items)
        }
    };

    print!("{rendered}");

    if config.persist {
        if let Err(e) = crate::commands::persist::save_page(
            config.query,
            &rendered,
            config.project,
            config.format,
        ) {
            warn!(error = %e, "failed to persist search output");
        }
    }

    Ok(())
}
