//! CLI for the semantic query-answer cache (plan 017).
//!
//! - `arags query --qa ...`: `QueryWithCache` + client-side digest-once. On a
//!   hit the server-served answer is printed with zero LLM calls; on a miss the
//!   client digests the top-K chunks with the user's LLM and fires the answer
//!   back to `StoreAnswer` (background, non-blocking for the user).
//! - `arags query --cache-id <id>`: direct `GetAnswerById` lookup (anti-drift).
//! - `arags cache get|invalidate`: direct cache inspection / admin invalidation.

use anyhow::{Context, Result};
use std::fmt::Write as _;

use arags_proto::proto::{
    GetAnswerByIdRequest, InvalidateCacheRequest, InvalidateMode, QueryWithCacheRequest,
    SearchResult, StoreAnswerRequest,
};

use crate::auth_client::AragsClient;
use crate::output::Format;

/// Run a query through the semantic cache (hit → zero LLM; miss → digest-once).
///
/// `client` is the authenticated gRPC client; `rt` drives the async calls.
pub fn run_ask(
    rt: &tokio::runtime::Runtime,
    client: &mut AragsClient,
    question: &str,
    backend: Option<&str>,
    model: Option<&str>,
    project: &str,
    format: Format,
) -> Result<()> {
    let req = QueryWithCacheRequest {
        project: project.to_string(),
        question: question.to_string(),
        buffer_id: 0,
    };
    let resp = rt.block_on(client.query_with_cache(req))?.into_inner();

    if resp.hit {
        print_answer(
            &resp.answer_text,
            &resp.provenance,
            &resp.cache_id,
            format,
            false,
        );
        return Ok(());
    }

    // MISS: synthesize the answer client-side with the user's LLM.
    let cfg = crate::user_config::load().context("failed to load user config")?;
    let llm = crate::backend::resolve_backend(cfg.llm_config(), backend, model)
        .context("failed to build LLM backend for digest")?;

    let mut context = String::new();
    for c in &resp.candidates {
        let _ = write!(context, "# {}\n```\n{}\n```\n", c.file_path, c.text);
    }
    let prompt = format!(
        "Based on the following project context, answer this question concisely and with provenance:\n\nQuestion: {question}\n\nContext:\n{context}"
    );

    let resolved_model = model.map(str::to_string).or_else(|| llm.default_model());
    let answer = rt
        .block_on(llm.complete(arags_llm::CompletionRequest {
            model: resolved_model.unwrap_or_else(|| "llama3".to_string()),
            messages: vec![arags_llm::Message {
                role: arags_llm::Role::User,
                content: prompt,
            }],
            temperature: Some(0.3),
            max_tokens: Some(2048),
            stop: None,
            seed: None,
            tools: None,
        }))
        .context("LLM digest failed")?
        .content;

    // Print immediately (UX), then fire-and-forget the store.
    print_answer(&answer, &resp.candidates, &resp.cache_id, format, true);

    let source_chunk_ids: Vec<String> = resp
        .candidates
        .iter()
        .map(|c| c.chunk_id.to_string())
        .collect();
    let source_hashes: Vec<String> = resp
        .candidates
        .iter()
        .map(|c| arags_core::qa_cache::chunk_content_hash(&c.text))
        .collect();

    let store_req = StoreAnswerRequest {
        project: project.to_string(),
        question: question.to_string(),
        answer,
        source_chunk_ids,
        source_hashes,
        model: model.map(str::to_string).unwrap_or_default(),
        token_count: 0,
        buffer_id: 0,
        cache_id: resp.cache_id,
    };
    if let Err(e) = rt.block_on(client.store_answer(store_req)) {
        tracing::warn!(error = %e, "StoreAnswer failed (answer already shown to user)");
    }
    Ok(())
}

/// Direct, deterministic lookup of a served answer by stable id (anti-drift).
pub fn run_get(
    rt: &tokio::runtime::Runtime,
    client: &mut AragsClient,
    cache_id: &str,
    project: &str,
    format: Format,
) -> Result<()> {
    let req = GetAnswerByIdRequest {
        cache_id: cache_id.to_string(),
        project: project.to_string(),
    };
    let resp = rt.block_on(client.get_answer_by_id(req))?.into_inner();

    if !resp.found {
        eprintln!("answer {cache_id} not found for project {project}");
        return Ok(());
    }
    print_answer(&resp.answer_text, &[], &resp.cache_id, format, false);
    Ok(())
}

/// Admin-gated invalidation of a cached answer.
pub fn run_invalidate(
    rt: &tokio::runtime::Runtime,
    client: &mut AragsClient,
    cache_id: Option<&str>,
    project: Option<&str>,
    delete: bool,
    radius: Option<f32>,
    reason: Option<&str>,
) -> Result<()> {
    let req = InvalidateCacheRequest {
        project: project.unwrap_or_default().to_string(),
        cache_id: cache_id.unwrap_or_default().to_string(),
        mode: if delete {
            InvalidateMode::Delete as i32
        } else {
            InvalidateMode::Stale as i32
        },
        similarity_radius: radius.unwrap_or(0.0),
    };
    let resp = rt.block_on(client.invalidate_cache(req))?.into_inner();
    println!(
        "invalidated {} cache entr(y/ies) by {}",
        resp.invalidated, resp.invalidated_by
    );
    let _ = reason;
    Ok(())
}

/// Render an answer (and its provenance) to the chosen output format.
fn print_answer(
    answer: &str,
    provenance: &[SearchResult],
    cache_id: &str,
    format: Format,
    miss: bool,
) {
    let prov: Vec<(String, String)> = provenance
        .iter()
        .map(|c| (c.file_path.clone(), c.text.clone()))
        .collect();
    match format {
        Format::FullJson => {
            let json = serde_json::json!({
                "cache_id": cache_id,
                "hit": !miss,
                "answer": answer,
                "provenance": prov,
            });
            println!("{}", serde_json::to_string(&json).unwrap_or_default());
        }
        Format::Jsonl => {
            println!(
                "{}",
                crate::output::jsonl::render_content_jsonl("cache_id", cache_id, &prov)
            );
        }
        Format::Markdown => {
            println!("## {cache_id}\n\n{answer}");
        }
        _ => {
            println!("{answer}");
        }
    }
}
