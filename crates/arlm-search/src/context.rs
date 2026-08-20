use std::fmt::Write as _;
use std::time::Instant;

use anyhow::{Context, Result};
use arlm_storage::Storage;

use crate::types::{ChunkWithText, HybridResult, OutputFormat, SearchResult};

/// Assemble context from search results for LLM consumption.
///
/// # Arguments
///
/// * `storage` - Storage to load chunk content from.
/// * `results` - Search results to include.
/// * `format` - Output format (Prompt, Json, Markdown).
/// * `max_tokens` - Optional token budget. Chunks are truncated/skipped to fit.
///
/// # Errors
///
/// Returns an error if chunk metadata or content cannot be loaded from storage.
pub fn build_context(
    storage: &Storage,
    results: &[HybridResult],
    format: OutputFormat,
    max_tokens: Option<u32>,
) -> Result<String> {
    let start = Instant::now();

    let chunks = load_chunks(storage, results)?;

    let chunks = if let Some(budget) = max_tokens {
        apply_token_budget(results, &chunks, budget)
    } else {
        chunks
    };

    let output = match format {
        OutputFormat::Prompt => format_prompt(results, &chunks),
        OutputFormat::Json => format_json(results, &chunks)?,
        OutputFormat::Markdown => format_markdown(results, &chunks),
    };

    tracing::info!(
        format = %format,
        chunks_loaded = chunks.len(),
        max_tokens = ?max_tokens,
        elapsed_ms = start.elapsed().as_millis(),
        "context assembled"
    );

    Ok(output)
}

/// Build rich search results with full metadata.
///
/// # Arguments
///
/// * `storage` - Storage to load chunk content from.
/// * `results` - Search results to include.
/// * `max_tokens` - Optional token budget. Results are truncated to fit.
///
/// # Errors
///
/// Returns an error if chunk metadata or content cannot be loaded from storage.
pub fn build_search_results(
    storage: &Storage,
    results: &[HybridResult],
    max_tokens: Option<u32>,
) -> Result<Vec<SearchResult>> {
    let start = Instant::now();

    let chunks = load_chunks(storage, results)?;

    let chunks = if let Some(budget) = max_tokens {
        apply_token_budget(results, &chunks, budget)
    } else {
        chunks
    };

    let search_results: Vec<SearchResult> = results
        .iter()
        .filter_map(|hr| {
            chunks
                .iter()
                .find(|c| c.id == hr.chunk_id)
                .map(|c| SearchResult {
                    chunk_id: hr.chunk_id,
                    score: hr.score,
                    file_path: c.file_path.clone(),
                    line_start: c.line_start,
                    line_end: c.line_end,
                    content: c.content.clone(),
                    language: c.language.clone(),
                    is_summary: c.is_summary,
                    summary_scope: c.summary_scope.clone(),
                })
        })
        .collect();

    tracing::debug!(
        results_count = search_results.len(),
        elapsed_ms = start.elapsed().as_millis(),
        "search results built"
    );

    Ok(search_results)
}

/// Load full chunk metadata + content for the given hybrid results.
///
/// Chunks whose id is not present in storage are skipped (e.g. deleted).
///
/// # Errors
///
/// Returns an error if chunk metadata or content cannot be read from storage.
pub fn load_chunks(storage: &Storage, results: &[HybridResult]) -> Result<Vec<ChunkWithText>> {
    let start = Instant::now();
    let mut chunks = Vec::with_capacity(results.len());

    for hr in results {
        if hr.is_summary {
            // Dual-layer: resolve from the `summaries` table instead of `chunks`.
            if let Some(summary) = storage.get_summary(hr.chunk_id)? {
                chunks.push(ChunkWithText {
                    id: summary.id,
                    buffer_id: summary.buffer_id,
                    file_path: format!("summary/{}", summary.scope),
                    line_start: 0,
                    line_end: 0,
                    content: summary.content,
                    language: None,
                    is_summary: true,
                    summary_scope: Some(summary.scope),
                });
            }
            continue;
        }

        let chunk = storage
            .get_chunk(hr.chunk_id)
            .context("failed to get chunk")?;

        let content = storage
            .get_chunk_content(hr.chunk_id)
            .context("failed to get chunk content")?;

        if let Some(c) = chunk {
            chunks.push(ChunkWithText {
                id: c.id,
                buffer_id: c.buffer_id,
                file_path: c.file_path,
                line_start: c.line_start,
                line_end: c.line_end,
                content: content.unwrap_or_default(),
                language: c.language,
                is_summary: false,
                summary_scope: None,
            });
        }
    }

    tracing::debug!(
        requested = results.len(),
        loaded = chunks.len(),
        elapsed_ms = start.elapsed().as_millis(),
        "chunks loaded"
    );

    Ok(chunks)
}

/// Apply a token budget by keeping the highest-scoring chunks that fit.
///
/// Estimates tokens using word-count heuristic (words × 1.3).
/// Truncates the last chunk's content to fit the remaining budget.
fn apply_token_budget(
    results: &[HybridResult],
    chunks: &[ChunkWithText],
    max_tokens: u32,
) -> Vec<ChunkWithText> {
    let mut used_tokens: u32 = 0;
    let mut selected = Vec::with_capacity(chunks.len());

    for hr in results {
        if let Some(chunk) = chunks.iter().find(|c| c.id == hr.chunk_id) {
            let chunk_tokens = estimate_tokens(&chunk.content);

            if used_tokens + chunk_tokens <= max_tokens {
                used_tokens += chunk_tokens;
                selected.push(chunk.clone());
            } else {
                // Try to fit a truncated version
                let remaining = max_tokens.saturating_sub(used_tokens);
                if remaining > 100 {
                    let truncated = truncate_to_tokens(&chunk.content, remaining);
                    let mut truncated_chunk = chunk.clone();
                    truncated_chunk.content = truncated;
                    selected.push(truncated_chunk);
                }
                break;
            }
        }
    }

    selected
}

/// Estimate token count using word-count heuristic.
fn estimate_tokens(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    let words = text.split_whitespace().count();
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let tokens = (words as f64 * 1.3).ceil() as u32;
    tokens
}

/// Truncate text to approximately fit within the given token budget.
fn truncate_to_tokens(text: &str, max_tokens: u32) -> String {
    // Approximate words from tokens: words ≈ tokens / 1.3
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let max_words = (f64::from(max_tokens) / 1.3).floor() as usize;

    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= max_words {
        return text.to_string();
    }

    words[..max_words].join(" ")
}

fn format_prompt(results: &[HybridResult], chunks: &[ChunkWithText]) -> String {
    let mut ctx = String::from("## Project Context\n\n");

    for (i, hr) in results.iter().enumerate() {
        if let Some(chunk) = chunks.iter().find(|c| c.id == hr.chunk_id) {
            let lang = chunk.language.as_deref().unwrap_or("");
            let _ = write!(
                ctx,
                "### File {} (score: {:.2})\n{}\n```{}\n{}\n```\n\n",
                i + 1,
                hr.score,
                chunk.file_path,
                lang,
                chunk.content,
            );
        }
    }

    ctx
}

fn format_json(results: &[HybridResult], chunks: &[ChunkWithText]) -> Result<String> {
    let items: Vec<serde_json::Value> = results
        .iter()
        .filter_map(|hr| {
            chunks.iter().find(|c| c.id == hr.chunk_id).map(|c| {
                let preview_len = 200.min(c.content.len());
                serde_json::json!({
                    "chunk_id": hr.chunk_id,
                    "score": hr.score,
                    "file_path": c.file_path,
                    "line_start": c.line_start,
                    "line_end": c.line_end,
                    "content_preview": &c.content[..preview_len],
                    "language": c.language,
                })
            })
        })
        .collect();

    serde_json::to_string_pretty(&items).context("failed to serialize results to JSON")
}

fn format_markdown(results: &[HybridResult], chunks: &[ChunkWithText]) -> String {
    let mut md = String::from("# Search Results\n\n");

    for (i, hr) in results.iter().enumerate() {
        if let Some(chunk) = chunks.iter().find(|c| c.id == hr.chunk_id) {
            let lang = chunk.language.as_deref().unwrap_or("");
            let _ = write!(
                md,
                "## {} {} (score: {:.2})\n\n```{}\n{}\n```\n\n",
                i + 1,
                chunk.file_path,
                hr.score,
                lang,
                chunk.content,
            );
        }
    }

    md
}
