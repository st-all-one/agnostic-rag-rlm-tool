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
                })
        })
        .collect();

    Ok(search_results)
}

fn load_chunks(storage: &Storage, results: &[HybridResult]) -> Result<Vec<ChunkWithText>> {
    let mut chunks = Vec::with_capacity(results.len());

    for hr in results {
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
            });
        }
    }

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
                    let truncated_tokens = estimate_tokens(&truncated);
                    used_tokens += truncated_tokens;
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
    let max_words = (max_tokens as f64 / 1.3).floor() as usize;

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

#[cfg(test)]
mod tests {
    use super::*;
    use arlm_storage::sqlite::buffers::NewBuffer;
    use arlm_storage::sqlite::chunks::NewChunk;

    fn setup() -> (Storage, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = Storage::open(tmp.path()).unwrap();
        (storage, tmp)
    }

    fn create_test_data(storage: &Storage) -> (i64, i64) {
        let buf_id = storage
            .insert_buffer(&NewBuffer {
                name: "test".to_string(),
                path: "/test".to_string(),
            })
            .unwrap();

        let chunk_id = storage
            .insert_chunk(&NewChunk {
                buffer_id: buf_id,
                file_path: "src/main.rs".to_string(),
                offset_start: 0,
                offset_end: 100,
                line_start: 1,
                line_end: 10,
                hash: vec![0u8],
                language: Some("rust".to_string()),
                chunk_type: None,
                token_count: Some(50),
            })
            .unwrap();

        storage
            .insert_chunk_content(chunk_id, "fn main() { println!(\"hello\"); }")
            .unwrap();

        (buf_id, chunk_id)
    }

    #[test]
    fn test_build_context_prompt() {
        let (storage, _tmp) = setup();
        let (_, chunk_id) = create_test_data(&storage);

        let results = vec![HybridResult {
            chunk_id,
            score: 0.85,
        }];

        let ctx = build_context(&storage, &results, OutputFormat::Prompt, None).unwrap();
        assert!(ctx.contains("## Project Context"));
        assert!(ctx.contains("src/main.rs"));
        assert!(ctx.contains("fn main()"));
        assert!(ctx.contains("0.85"));
    }

    #[test]
    fn test_build_context_markdown() {
        let (storage, _tmp) = setup();
        let (_, chunk_id) = create_test_data(&storage);

        let results = vec![HybridResult {
            chunk_id,
            score: 0.90,
        }];

        let ctx = build_context(&storage, &results, OutputFormat::Markdown, None).unwrap();
        assert!(ctx.contains("# Search Results"));
        assert!(ctx.contains("src/main.rs"));
    }

    #[test]
    fn test_build_context_json() {
        let (storage, _tmp) = setup();
        let (_, chunk_id) = create_test_data(&storage);

        let results = vec![HybridResult {
            chunk_id,
            score: 0.75,
        }];

        let ctx = build_context(&storage, &results, OutputFormat::Json, None).unwrap();
        assert!(ctx.contains("chunk_id"));
        assert!(ctx.contains("src/main.rs"));
    }

    #[test]
    fn test_build_search_results() {
        let (storage, _tmp) = setup();
        let (_, chunk_id) = create_test_data(&storage);

        let results = vec![HybridResult {
            chunk_id,
            score: 0.95,
        }];

        let search_results = build_search_results(&storage, &results, None).unwrap();
        assert_eq!(search_results.len(), 1);
        assert_eq!(search_results[0].chunk_id, chunk_id);
        assert_eq!(search_results[0].file_path, "src/main.rs");
        assert_eq!(search_results[0].language, Some("rust".to_string()));
    }

    #[test]
    fn test_build_context_empty() {
        let (storage, _tmp) = setup();
        let ctx = build_context(&storage, &[], OutputFormat::Prompt, None).unwrap();
        assert!(ctx.contains("## Project Context"));
    }

    #[test]
    fn test_load_chunks_missing() {
        let (storage, _tmp) = setup();
        let results = vec![HybridResult {
            chunk_id: 999,
            score: 0.5,
        }];

        let chunks = load_chunks(&storage, &results).unwrap();
        assert!(chunks.is_empty());
    }
}
