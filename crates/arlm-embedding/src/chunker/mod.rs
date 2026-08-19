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

/// Approximate token count (whitespace-delimited words).
///
/// This is a fast heuristic for sizing chunks. Production systems
/// may use a proper tokenizer for more accurate counts.
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    text.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language_rust() {
        let path = Path::new("src/main.rs");
        assert_eq!(detect_language(path).as_deref(), Some("rust"));
    }

    #[test]
    fn test_detect_language_python() {
        let path = Path::new("script.py");
        assert_eq!(detect_language(path).as_deref(), Some("python"));
    }

    #[test]
    fn test_detect_language_unknown() {
        let path = Path::new("file.xyz");
        assert_eq!(detect_language(path), None);
    }

    #[test]
    fn test_prev_char_boundary_ascii() {
        let s = "hello";
        assert_eq!(prev_char_boundary(s, 5), 5);
        assert_eq!(prev_char_boundary(s, 3), 3);
    }

    #[test]
    fn test_prev_char_boundary_unicode() {
        let s = "héllo"; // é is 2 bytes: [0xC3, 0xA9]
        // byte 0: 'h', byte 1-2: 'é', byte 3: 'l', byte 4: 'l', byte 5: 'o'
        assert_eq!(prev_char_boundary(s, 5), 5); // 'o' start
        assert_eq!(prev_char_boundary(s, 4), 4); // second 'l' start
        assert_eq!(prev_char_boundary(s, 3), 3); // first 'l' start
        assert_eq!(prev_char_boundary(s, 2), 1); // middle of 'é', recede to byte 1
    }

    #[test]
    fn test_nth_line_byte_offset() {
        let s = "line1\nline2\nline3";
        assert_eq!(nth_line_byte_offset(s, 0), 0);
        assert_eq!(nth_line_byte_offset(s, 1), 6);
        assert_eq!(nth_line_byte_offset(s, 2), 12);
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens("hello world"), 2);
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("  spaced  out  "), 2);
    }
}
