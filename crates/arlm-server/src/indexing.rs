//! Deterministic indexing primitives for `IndexProject`.
//!
//! Kept dependency-light so indexing works offline with no model weights:
//! files are discovered, chunked deterministically, hashed, and stored with
//! their extracted entities. All functions are pure where possible so they can
//! be unit-tested independently of storage.

use std::path::Path;

use sha2::{Digest, Sha256};

/// A chunk produced by the deterministic chunker.
#[derive(Debug, Clone)]
pub struct IndexedChunk {
    pub file_path: String,
    /// 1-based start line (inclusive).
    pub line_start: i32,
    /// 1-based start line (inclusive).
    pub line_end: i32,
    pub content: String,
    /// SHA-256 hex digest of the content.
    pub hash: String,
    /// Language hint inferred from the file extension.
    pub language: Option<String>,
    /// Simple chunk type tag ("code", "markdown", "text").
    pub chunk_type: String,
}

const DEFAULT_MAX_LINES: usize = 80;
const DEFAULT_OVERLAP_LINES: usize = 8;

/// Infer a language hint from a file path extension.
#[must_use]
pub fn infer_language(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "js" | "jsx" | "ts" | "tsx" => Some("javascript"),
        "go" => Some("go"),
        "java" => Some("java"),
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" => Some("cpp"),
        "rb" => Some("ruby"),
        "php" => Some("php"),
        "md" | "markdown" => Some("markdown"),
        _ => None,
    }
}

/// Classify a file as code, markdown or plain text for chunking purposes.
#[must_use]
pub fn classify(path: &Path) -> &'static str {
    match infer_language(path).map(str::to_ascii_lowercase) {
        Some(lang) if lang == "markdown" => "markdown",
        Some(_) => "code",
        None => "text",
    }
}

/// Split text into chunks of at most `max_lines` lines with `overlap` lines of
/// context shared between adjacent chunks.
#[must_use]
pub fn chunk_lines(content: &str, max_lines: usize, overlap: usize) -> Vec<(i32, i32, String)> {
    if content.is_empty() {
        return Vec::new();
    }

    let lines: Vec<&str> = content.lines().collect();
    let max_lines = max_lines.max(1);
    let overlap = overlap.min(max_lines.saturating_sub(1));

    let mut chunks = Vec::with_capacity(lines.len() / max_lines + 1);
    let mut start = 0usize;

    while start < lines.len() {
        let end = (start + max_lines).min(lines.len());
        let text = lines[start..end].join("\n");
        // 1-based line numbers.
        chunks.push((start as i32 + 1, end as i32, text));
        if end >= lines.len() {
            break;
        }
        start = end.saturating_sub(overlap);
    }

    chunks
}

/// Chunk a file's content, returning `IndexedChunk`s with language metadata.
#[must_use]
pub fn index_file(path: &Path, content: &str) -> Vec<IndexedChunk> {
    let language = infer_language(path).map(str::to_string);
    let chunk_type = classify(path);
    let file_path = path.to_string_lossy().into_owned();

    chunk_lines(content, DEFAULT_MAX_LINES, DEFAULT_OVERLAP_LINES)
        .into_iter()
        .map(|(line_start, line_end, text)| {
            let hash = hash_text(&text);
            IndexedChunk {
                file_path: file_path.clone(),
                line_start,
                line_end,
                content: text,
                hash,
                language: language.clone(),
                chunk_type: chunk_type.to_string(),
            }
        })
        .collect()
}

/// SHA-256 hex digest of arbitrary text.
#[must_use]
pub fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}
