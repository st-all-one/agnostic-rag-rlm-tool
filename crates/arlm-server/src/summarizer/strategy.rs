use anyhow::Result;

/// Summarization strategy for different scopes.
pub enum SummaryStrategy {
    /// Per-file: summarize all chunks from the same file.
    File,
    /// Per-module: summarize file summaries from the same directory.
    Module,
    /// Per-project: summarize module summaries for the entire project.
    Project,
}

/// A chunk of raw content to be summarized.
#[derive(Debug, Clone)]
pub struct RawChunk {
    pub id: i64,
    pub content: String,
    pub file_path: String,
}

/// A summary produced from raw chunks.
#[derive(Debug, Clone)]
pub struct Summary {
    pub content: String,
    pub scope: String,
    pub source_chunk_ids: Vec<i64>,
    pub source_hash: String,
    pub confidence: f64,
}

/// Build a summarization prompt for the given chunks.
///
/// # Errors
///
/// Returns an error if the chunks are empty.
pub fn build_summary_prompt(chunks: &[RawChunk], scope: &str) -> Result<String> {
    if chunks.is_empty() {
        return Err(anyhow::anyhow!("no chunks to summarize"));
    }

    let file_info = if scope == "file" {
        format!("File: {}", chunks[0].file_path)
    } else {
        let files: Vec<&str> = chunks.iter().map(|c| c.file_path.as_str()).collect();
        let unique_files = files.iter().copied().collect::<std::collections::HashSet<_>>();
        format!("Files: {}", unique_files.iter().cloned().collect::<Vec<_>>().join(", "))
    };

    let chunks_text = chunks
        .iter()
        .map(|c| format!("--- Chunk {} ---\n{}", c.id, c.content))
        .collect::<Vec<_>>()
        .join("\n\n");

    Ok(format!(
        r#"You are a code summarizer. Summarize the following {scope} content.

{file_info}

The summary should:
1. Describe the purpose and functionality of the code
2. List key functions, structs, and their relationships
3. Note any important patterns or conventions
4. Be concise but comprehensive (200-500 tokens)
5. Be optimized for LLM consumption (clear, structured)

Content to summarize:
{chunks_text}

Provide a structured summary:"#
    ))
}

/// Parse a summary response from the LLM.
///
/// # Errors
///
/// Returns an error if the response is empty.
pub fn parse_summary_response(response: &str) -> Result<String> {
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("empty summary response"));
    }
    Ok(trimmed.to_string())
}
