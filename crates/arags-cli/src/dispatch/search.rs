//! `search`/`query` RPCs and their multi-format rendering.

use anyhow::Result;
use tokio::runtime::Runtime;
use tonic::Request;

use arags_proto::proto::{SearchRequest, SearchResult};

use crate::auth_client::AragsClient;
use crate::output::Format;

use super::map_search_tier;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_search(
    rt: &Runtime,
    client: &mut AragsClient,
    project: &str,
    query: &str,
    top_k: usize,
    tier: &str,
    min_score: Option<f32>,
    file_pattern: Option<&str>,
    format: Format,
) -> Result<()> {
    let project_str = project.to_string();
    let request = Request::new(SearchRequest {
        project: project_str,
        query: query.to_string(),
        max_results: top_k as i32,
        tier: map_search_tier(tier) as i32,
    });
    let response = rt.block_on(client.search(request))?;
    let inner = response.into_inner();
    let mut results = inner.results;

    if let Some(min) = min_score {
        results.retain(|r| r.score >= min);
    }
    if let Some(pat) = file_pattern {
        results.retain(|r| r.file_path.contains(pat));
    }

    let rendered = render_search(
        &results,
        &inner.summaries,
        &inner.explorations,
        query,
        format,
    );
    print!("{rendered}");
    Ok(())
}

fn render_search(
    results: &[SearchResult],
    summaries: &[arags_proto::proto::SummaryHit],
    explorations: &[arags_proto::proto::ExplorationRef],
    query: &str,
    format: Format,
) -> String {
    match format {
        Format::FullJson => {
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "chunk_id": r.chunk_id,
                        "file": r.file_path,
                        "score": r.score,
                        "text": r.text,
                    })
                })
                .collect();
            crate::output::json::JsonOutput::ok()
                .with_data(serde_json::json!({
                    "query": query,
                    "results": items,
                    "count": results.len(),
                    "summaries": summaries.iter().map(|s| serde_json::json!({
                        "node_id": s.node_id,
                        "level": s.level,
                        "subject": s.subject,
                        "score": s.score,
                        "text": s.summary_text,
                    })).collect::<Vec<_>>(),
                    "explorations": explorations.iter().map(|e| serde_json::json!({
                        "exploration_id": e.exploration_id,
                        "goal": e.goal,
                        "summary": e.summary,
                        "confidence": e.confidence,
                    })).collect::<Vec<_>>(),
                }))
                .to_json_string()
        }
        Format::Jsonl => {
            let mut pairs: Vec<(String, String)> = results
                .iter()
                .map(|r| (r.file_path.clone(), r.text.clone()))
                .collect();
            for s in summaries {
                pairs.push((
                    format!("[summary:{}] {}", s.level, s.subject),
                    s.summary_text.clone(),
                ));
            }
            for e in explorations {
                pairs.push((format!("[map] {}", e.exploration_id), e.summary.clone()));
            }
            crate::output::jsonl::render_content_jsonl("query", query, &pairs)
        }
        Format::Path => {
            let items: Vec<crate::output::tree::SearchResultItem> = results
                .iter()
                .map(|r| crate::output::tree::SearchResultItem {
                    file_path: r.file_path.clone(),
                    line_start: i64::from(r.start_line),
                    line_end: i64::from(r.end_line),
                    score: r.score,
                })
                .collect();
            crate::output::tree::render_search_results(&items)
        }
        Format::Markdown => {
            let mut out = render_markdown_results(results);
            if !summaries.is_empty() {
                out.push_str("## RLM Summaries\n\n");
                let items: Vec<crate::output::markdown::SuperItem> = summaries
                    .iter()
                    .map(|s| crate::output::markdown::SuperItem {
                        file_path: format!("[summary:{}] {}", s.level, s.subject),
                        score: s.score,
                        content: s.summary_text.clone(),
                        language: None,
                    })
                    .collect();
                out.push_str(&crate::output::markdown::render_search_results(&items));
            }
            if !explorations.is_empty() {
                out.push_str("## Exploration Maps\n\n");
                let items: Vec<crate::output::markdown::SuperItem> = explorations
                    .iter()
                    .map(|e| crate::output::markdown::SuperItem {
                        file_path: format!(
                            "[map:{id}] {goal}",
                            id = e.exploration_id,
                            goal = e.goal
                        ),
                        score: e.confidence,
                        content: e.summary.clone(),
                        language: None,
                    })
                    .collect();
                out.push_str(&crate::output::markdown::render_search_results(&items));
            }
            out
        }
        Format::Text => {
            let items: Vec<crate::output::prompt::PromptItem> = results
                .iter()
                .map(|r| crate::output::prompt::PromptItem {
                    file_path: r.file_path.clone(),
                    score: r.score,
                    content: r.text.clone(),
                    language: None,
                })
                .collect();
            crate::output::prompt::render_search_context(&items)
        }
    }
}

fn render_markdown_results(results: &[SearchResult]) -> String {
    let items: Vec<crate::output::markdown::SuperItem> = results
        .iter()
        .map(|r| crate::output::markdown::SuperItem {
            file_path: r.file_path.clone(),
            score: r.score,
            content: r.text.clone(),
            language: None,
        })
        .collect();
    crate::output::markdown::render_search_results(&items)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_query(
    rt: &Runtime,
    client: &mut AragsClient,
    project: &str,
    question: &str,
    cache_id: Option<String>,
    qa: bool,
    backend: Option<&str>,
    model: Option<&str>,
    format: Format,
) -> Result<()> {
    let project_str = project.to_string();

    if let Some(id) = cache_id {
        return crate::commands::qa_cache::run_get(rt, client, &id, &project_str, format);
    }
    if qa {
        return crate::commands::qa_cache::run_ask(
            rt,
            client,
            question,
            backend,
            model,
            &project_str,
            format,
        );
    }

    // Default: server-side context (no client LLM), deterministic. Mirrors the
    // removed `context` command.
    let request = Request::new(arags_proto::proto::ContextRequest {
        project: project_str.clone(),
        task: question.to_string(),
        ..Default::default()
    });
    let response = rt.block_on(client.build_context(request))?;
    let ctx = response.into_inner().context;
    let rendered = match format {
        Format::FullJson => crate::output::json::JsonOutput::ok()
            .with_data(serde_json::json!({ "question": question, "context": ctx }))
            .to_json_string(),
        Format::Jsonl => {
            let pairs: Vec<(String, String)> = vec![(project_str.clone(), ctx.clone())];
            crate::output::jsonl::render_content_jsonl("question", question, &pairs)
        }
        _ => ctx,
    };
    print!("{rendered}");
    Ok(())
}
