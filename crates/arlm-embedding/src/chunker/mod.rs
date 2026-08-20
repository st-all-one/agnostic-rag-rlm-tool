use std::borrow::Cow;
use std::path::Path;

pub mod code;
pub mod markdown;
pub mod recursive;
pub mod text;

/// A raw chunk produced by a chunking strategy.
///
/// `Cow<'a, str>` allows zero-copy borrowing from the source text
/// when no modification is needed, and owned allocation only when
/// the chunk content must be transformed (e.g., joining lines).
pub struct RawChunk<'a> {
    pub offset_start: usize,
    pub offset_end: usize,
    pub line_start: usize,
    pub line_end: usize,
    pub content: Cow<'a, str>,
    pub language: Option<String>,
    pub chunk_type: Option<String>,
}

/// Strategy for splitting text into chunks.
///
/// Implementations produce `RawChunk` values that borrow from the input
/// text when possible (zero-copy via `Cow::Borrowed`).
pub trait ChunkingStrategy: Send + Sync {
    /// Split `content` into chunks.
    ///
    /// The returned chunks borrow from `content` when no modification is needed.
    fn chunk<'a>(&self, content: &'a str, path: &Path) -> Vec<RawChunk<'a>>;
}

/// Detect the programming language from a file path extension.
#[must_use]
pub fn detect_language(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;

    let lang = match ext {
        "rs" => "rust",
        "py" => "python",
        "js" | "jsx" => "javascript",
        "ts" | "tsx" => "typescript",
        "go" => "go",
        "java" => "java",
        "cpp" | "cc" | "cxx" => "cpp",
        "c" | "h" => "c",
        "rb" => "ruby",
        "php" => "php",
        "md" | "markdown" => "markdown",
        "txt" | "log" => "text",
        _ => return None,
    };
    Some(lang.to_string())
}

/// Recede a byte index to the nearest valid UTF-8 character boundary.
///
/// This prevents panics when slicing strings at arbitrary byte positions.
#[must_use]
pub fn prev_char_boundary(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Compute the byte offset of the start of the n-th line (0-based).
///
/// Returns `s.len()` if `n` exceeds the number of lines.
#[must_use]
pub fn nth_line_byte_offset(s: &str, n: usize) -> usize {
    let mut offset = 0usize;
    for (i, line) in s.split('\n').enumerate() {
        if i >= n {
            break;
        }
        offset += line.len() + 1; // +1 for '\n'
    }
    offset.min(s.len())
}

/// Approximate token count using tiktoken (`cl100k_base` encoding).
///
/// Falls back to whitespace counting if tiktoken fails.
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    let enc = tiktoken_rs::cl100k_base_singleton();
    enc.encode_with_special_tokens(text).len()
}
