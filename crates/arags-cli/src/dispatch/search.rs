//! `search`/`query` RPCs and their multi-format rendering.

use anyhow::Result;
use std::fmt::Write as _;
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
    as_of_epoch: i64,
    format: Format,
) -> Result<()> {
    let project_str = project.to_string();
    let request = Request::new(SearchRequest {
        project: project_str,
        query: query.to_string(),
        max_results: top_k as i32,
        tier: map_search_tier(tier) as i32,
        as_of_epoch,
        ..Default::default()
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
        as_of_epoch,
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
    as_of_epoch: i64,
    format: Format,
) -> String {
    match format {
        Format::FullJson => {
            let as_of = if as_of_epoch > 0 {
                Some(as_of_epoch)
            } else {
                None
            };
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "chunk_id": r.chunk_id,
                        "file": r.file_path,
                        "score": r.score,
                        "text": r.text,
                        "epoch": r.epoch,
                        "created_by": r.created_by,
                        "model": r.model,
                        "version": r.version,
                    })
                })
                .collect();
            crate::output::json::JsonOutput::ok()
                .with_data(serde_json::json!({
                    "query": query,
                    "as_of_epoch": as_of,
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
            let mut obj = serde_json::Map::new();
            obj.insert("query".into(), serde_json::Value::String(query.to_string()));
            if as_of_epoch > 0 {
                obj.insert("as_of_epoch".into(), serde_json::json!(as_of_epoch));
            }
            let items: Vec<serde_json::Value> = pairs
                .iter()
                .map(|(file, text)| serde_json::json!({ "file": file, "text": text }))
                .collect();
            // Attach per-chunk temporal metadata (plan 021) keyed by result
            // index so agents can read username/model/epoch/version alongside
            // `text`.
            let meta: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "chunk_id": r.chunk_id,
                        "epoch": r.epoch,
                        "created_by": r.created_by,
                        "model": r.model,
                        "version": r.version,
                    })
                })
                .collect();
            obj.insert("result_meta".into(), serde_json::Value::Array(meta));
            obj.insert("results".into(), serde_json::Value::Array(items));
            serde_json::to_string(&obj).unwrap_or_default()
        }
        Format::Path => {
            let mut out = String::new();
            if as_of_epoch > 0 {
                let _ = writeln!(out, "// time-travel snapshot @ epoch {as_of_epoch}");
            }
            let items: Vec<crate::output::tree::SearchResultItem> = results
                .iter()
                .map(|r| crate::output::tree::SearchResultItem {
                    file_path: r.file_path.clone(),
                    line_start: i64::from(r.start_line),
                    line_end: i64::from(r.end_line),
                    score: r.score,
                })
                .collect();
            out.push_str(&crate::output::tree::render_search_results(&items));
            out
        }
        Format::Markdown => {
            let mut out = String::new();
            if as_of_epoch > 0 {
                let _ = write!(
                    out,
                    "> **Time-travel snapshot** at epoch `{as_of_epoch}`\n\n"
                );
            }
            out.push_str(&render_markdown_results(results));
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
            let mut out = String::new();
            if as_of_epoch > 0 {
                let _ = writeln!(out, "// TIME-TRAVEL SNAPSHOT @ epoch {as_of_epoch}");
            }
            for r in results {
                let _ = write!(
                    out,
                    "// {} (epoch {}, by {}, via {}, v{}) score={:.2}\n{}\n",
                    r.file_path, r.epoch, r.created_by, r.model, r.version, r.score, r.text
                );
            }
            out
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
    as_of_epoch: i64,
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
            as_of_epoch,
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
