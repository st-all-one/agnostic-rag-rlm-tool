use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::time::Instant;

use anyhow::{Context, Result};
use arlm_llm::{CompletionRequest, LlmBackend, Message, Role};
use arlm_storage::Storage;

use crate::context::build_search_results;
use crate::types::{HybridResult, SearchResult};

use super::{
    HybridSearch, RERANK_MAX_TOKENS, RERANK_MODEL, RERANK_SNIPPET_LEN, RERANK_SYSTEM_PROMPT,
};

/// Build the rerank prompt: query + each candidate's id, file path, and a snippet.
fn build_rerank_prompt(query: &str, candidates: &[SearchResult]) -> String {
    let mut prompt = format!("Query: {query}\n\nCandidates:\n");
    for c in candidates {
        let snippet: String = c.content.chars().take(RERANK_SNIPPET_LEN).collect();
        let _ = writeln!(prompt, "ID {} [{}]: {snippet}", c.chunk_id, c.file_path);
    }
    prompt.push_str("\nRank the IDs by relevance (most relevant first), one ID per line.");
    prompt
}

/// Parse the LLM's ranked id ordering from a free-form response.
///
/// Tolerates varied formats ("1. 42", "ID 42", "42", "rank 1: id=42", "42 (score)"),
/// keeps only known candidate ids, and drops duplicates while preserving order.
fn parse_rerank_order(response: &str, candidates: &[SearchResult]) -> Vec<i64> {
    let known: HashSet<i64> = candidates.iter().map(|c| c.chunk_id).collect();
    let mut ordered = Vec::new();
    for line in response.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        for token in
            line.split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ')' || ch == '(')
        {
            let token = token.trim_matches(|ch: char| !ch.is_ascii_digit());
            if token.is_empty() {
                continue;
            }
            if let Ok(id) = token.parse::<i64>() {
                if known.contains(&id) && !ordered.contains(&id) {
                    ordered.push(id);
                }
            }
        }
    }
    ordered
}

/// Reorder candidates by the parsed id ranking; unknown/unranked ids are appended
/// at the end in ascending id order.
fn reorder_candidates(candidates: Vec<SearchResult>, ranked_ids: &[i64]) -> Vec<SearchResult> {
    let mut by_id: HashMap<i64, SearchResult> =
        candidates.into_iter().map(|c| (c.chunk_id, c)).collect();
    let mut reordered: Vec<SearchResult> = ranked_ids
        .iter()
        .filter_map(|id| by_id.remove(id))
        .collect();
    // Append remaining candidates (not in ranked_ids) at the end in their original order
    let mut remaining: Vec<SearchResult> = by_id.into_values().collect();
    remaining.sort_by_key(|c| c.chunk_id);
    reordered.append(&mut remaining);
    reordered
}

impl HybridSearch {
    /// Reorder fused results by asking the LLM backend to rank candidate chunks.
    ///
    /// Builds a prompt containing the query and each candidate's chunk id plus a
    /// text snippet, calls the backend, parses the returned id ordering, and
    /// reorders the candidates accordingly. Candidate ids not mentioned in the
    /// response are appended at the end in their original order.
    ///
    /// # Errors
    ///
    /// Returns an error if the LLM completion fails.
    pub async fn llm_rerank(
        candidates: Vec<SearchResult>,
        query: &str,
        backend: &dyn LlmBackend,
    ) -> Result<Vec<SearchResult>> {
        if candidates.is_empty() {
            return Ok(candidates);
        }

        let prompt = build_rerank_prompt(query, &candidates);

        let request = CompletionRequest {
            model: RERANK_MODEL.to_string(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: RERANK_SYSTEM_PROMPT.to_string(),
                },
                Message {
                    role: Role::User,
                    content: prompt,
                },
            ],
            temperature: Some(0.0),
            max_tokens: Some(RERANK_MAX_TOKENS),
            stop: None,
        };

        let response = backend
            .complete(request)
            .await
            .context("LLM rerank completion failed")?;

        let ranked_ids = parse_rerank_order(&response.content, &candidates);
        let reranked = reorder_candidates(candidates, &ranked_ids);

        tracing::info!(
            backend = %backend.name(),
            candidates = reranked.len(),
            "llm rerank completed"
        );

        Ok(reranked)
    }

    pub(crate) async fn rerank_with_llm(
        &self,
        storage: &Storage,
        query: &str,
        results: Vec<HybridResult>,
    ) -> Result<Vec<HybridResult>> {
        let start = Instant::now();
        let Some(backend) = &self.llm_backend else {
            return Ok(results);
        };

        let search_results = build_search_results(storage, &results, None)?;

        let reranked = Self::llm_rerank(search_results, query, backend.as_ref()).await?;

        let mapped: Vec<HybridResult> = reranked
            .into_iter()
            .map(|s| HybridResult {
                chunk_id: s.chunk_id,
                score: s.score,
                is_summary: s.is_summary,
            })
            .collect();

        tracing::debug!(
            reranked = mapped.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "llm rerank applied"
        );

        Ok(mapped)
    }
}
