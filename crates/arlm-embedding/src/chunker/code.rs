use std::path::Path;

use crate::chunker::{
    ChunkingStrategy, RawChunk, detect_language, estimate_tokens, nth_line_byte_offset,
    prev_char_boundary,
};

/// Configuration for code-aware chunking.
pub struct CodeChunker {
    /// Maximum tokens per chunk.
    pub max_tokens: usize,
    /// Overlap in tokens between consecutive chunks.
    pub overlap_tokens: usize,
}

impl CodeChunker {
    #[must_use]
    pub fn new(max_tokens: usize, overlap_tokens: usize) -> Self {
        Self {
            max_tokens,
            overlap_tokens,
        }
    }
}

impl ChunkingStrategy for CodeChunker {
    fn chunk<'a>(&self, content: &'a str, path: &Path) -> Vec<RawChunk<'a>> {
        let _timer = crate::Timer::new("code_chunking");
        let language = detect_language(path);

        match language.as_deref() {
            Some(
                "rust" | "python" | "javascript" | "typescript" | "go" | "java" | "cpp" | "c"
                | "ruby" | "php",
            ) => self.chunk_by_structures(content, language.as_deref()),
            _ => self.chunk_by_lines(content, language.as_deref()),
        }
    }
}

impl CodeChunker {
    /// Chunk by logical structures (functions, classes, etc.).
    ///
    /// Uses a heuristic: split on blank lines or dedents to identify
    /// top-level blocks. Falls back to line-based chunking for complex cases.
    fn chunk_by_structures<'a>(
        &self,
        content: &'a str,
        language: Option<&str>,
    ) -> Vec<RawChunk<'a>> {
        let mut chunks = Vec::with_capacity(64);
        let mut block_start = 0usize;
        let mut block_end = 0usize;
        let mut in_block = false;

        for (i, line) in content.split_inclusive('\n').enumerate() {
            let trimmed = line.trim();
            let is_blank = trimmed.is_empty();
            let is_block_start = is_structure_start(trimmed, language);

            if is_block_start && in_block && block_end > block_start {
                // Close previous block
                let chunk_content = &content[block_start..prev_char_boundary(content, block_end)];
                if estimate_tokens(chunk_content) > 0 {
                    chunks.push(RawChunk {
                        offset_start: block_start,
                        offset_end: prev_char_boundary(content, block_end),
                        line_start: byte_start_line(content, block_start),
                        line_end: i,
                        content: std::borrow::Cow::Borrowed(chunk_content),
                        language: language.map(String::from),
                        chunk_type: Some("structure".into()),
                    });
                }
                block_start = nth_line_byte_offset(content, i);
                block_end = block_start;
            }

            if is_block_start || in_block {
                in_block = true;
                block_end += line.len();
            } else if is_blank && in_block {
                // Blank line may end a block
                block_end += line.len();
            }
        }

        // Final block
        if in_block && block_end > block_start {
            let chunk_content = &content[block_start..prev_char_boundary(content, block_end)];
            if estimate_tokens(chunk_content) > 0 {
                chunks.push(RawChunk {
                    offset_start: block_start,
                    offset_end: prev_char_boundary(content, block_end),
                    line_start: byte_start_line(content, block_start),
                    line_end: content.split('\n').count(),
                    content: std::borrow::Cow::Borrowed(chunk_content),
                    language: language.map(String::from),
                    chunk_type: Some("structure".into()),
                });
            }
        }

        // If no structures were found, fall back to line-based
        if chunks.is_empty() {
            return self.chunk_by_lines(content, language);
        }

        // Merge small chunks that fit within max_tokens
        merge_small_chunks(chunks, self.max_tokens)
    }

    /// Chunk by lines with overlap.
    fn chunk_by_lines<'a>(&self, content: &'a str, language: Option<&str>) -> Vec<RawChunk<'a>> {
        let mut chunks = Vec::with_capacity(64);
        let mut line_start = 0usize;
        let mut byte_start = 0usize;
        let mut current_lines = 0usize;

        for (i, line) in content.split_inclusive('\n').enumerate() {
            current_lines += 1;
            let line_tokens = estimate_tokens(line);

            if current_lines + line_tokens > self.max_tokens && current_lines > 1 {
                // Emit chunk
                let chunk_content =
                    &content[byte_start..prev_char_boundary(content, byte_start + line.len())];
                chunks.push(RawChunk {
                    offset_start: byte_start,
                    offset_end: prev_char_boundary(content, byte_start + line.len()),
                    line_start: line_start + 1,
                    line_end: i + 1,
                    content: std::borrow::Cow::Borrowed(chunk_content),
                    language: language.map(String::from),
                    chunk_type: Some("lines".into()),
                });

                // Overlap: go back overlap_tokens lines
                let overlap_lines = self.overlap_tokens.min(current_lines - 1);
                line_start = i + 1 - overlap_lines;
                byte_start = nth_line_byte_offset(content, line_start);
                current_lines = overlap_lines;
            }
        }

        // Emit final chunk
        if byte_start < content.len() {
            let chunk_content = &content[byte_start..];
            if estimate_tokens(chunk_content) > 0 {
                chunks.push(RawChunk {
                    offset_start: byte_start,
                    offset_end: content.len(),
                    line_start: line_start + 1,
                    line_end: content.split('\n').count(),
                    content: std::borrow::Cow::Borrowed(chunk_content),
                    language: language.map(String::from),
                    chunk_type: Some("lines".into()),
                });
            }
        }

        chunks
    }
}

/// Merge consecutive small chunks that fit within `max_tokens`.
fn merge_small_chunks<'a>(chunks: Vec<RawChunk<'a>>, max_tokens: usize) -> Vec<RawChunk<'a>> {
    if chunks.len() <= 1 {
        return chunks;
    }

    let mut merged = Vec::with_capacity(chunks.len());
    let mut pending: Option<RawChunk<'a>> = None;

    for chunk in chunks {
        match pending.take() {
            Some(prev) => {
                let combined_tokens =
                    estimate_tokens(&prev.content) + estimate_tokens(&chunk.content);
                if combined_tokens <= max_tokens
                    && prev.chunk_type == chunk.chunk_type
                    && prev.language == chunk.language
                {
                    // Merge: allocate a new owned string with combined content
                    let mut combined =
                        String::with_capacity(prev.content.len() + chunk.content.len());
                    combined.push_str(&prev.content);
                    combined.push_str(&chunk.content);
                    pending = Some(RawChunk {
                        offset_start: prev.offset_start,
                        offset_end: chunk.offset_end,
                        line_start: prev.line_start,
                        line_end: chunk.line_end,
                        content: std::borrow::Cow::Owned(combined),
                        language: chunk.language,
                        chunk_type: chunk.chunk_type,
                    });
                } else {
                    merged.push(prev);
                    pending = Some(chunk);
                }
            }
            None => {
                pending = Some(chunk);
            }
        }
    }

    if let Some(last) = pending {
        merged.push(last);
    }

    merged
}

/// Check if a trimmed line looks like the start of a code structure.
fn is_structure_start(line: &str, language: Option<&str>) -> bool {
    match language {
        Some("rust") => {
            line.starts_with("fn ")
                || line.starts_with("pub fn ")
                || line.starts_with("pub(crate) fn ")
                || line.starts_with("async fn ")
                || line.starts_with("pub async fn ")
                || line.starts_with("struct ")
                || line.starts_with("pub struct ")
                || line.starts_with("enum ")
                || line.starts_with("pub enum ")
                || line.starts_with("impl ")
                || line.starts_with("trait ")
                || line.starts_with("pub trait ")
                || line.starts_with("mod ")
                || line.starts_with("pub mod ")
                || line.starts_with("type ")
        }
        Some("python") => {
            line.starts_with("def ") || line.starts_with("class ") || line.starts_with("async def ")
        }
        Some("javascript" | "typescript") => {
            line.starts_with("function ")
                || line.starts_with("export function ")
                || line.starts_with("export default function ")
                || line.starts_with("class ")
                || line.starts_with("export class ")
                || line.starts_with("async function ")
                || line.starts_with("export async function ")
                || line.starts_with("const ")
                || line.starts_with("let ")
        }
        Some("go") => {
            line.starts_with("func ") || line.starts_with("type ") || line.starts_with("package ")
        }
        Some("java") => {
            line.starts_with("public class ")
                || line.starts_with("class ")
                || line.starts_with("public interface ")
                || line.starts_with("interface ")
                || line.starts_with("public enum ")
                || line.starts_with("enum ")
                || (line.contains("public static void main") && line.contains('('))
        }
        Some("cpp" | "c") => {
            line.starts_with("void ")
                || line.starts_with("int ")
                || line.starts_with("static ")
                || line.starts_with("extern ")
                || line.starts_with("class ")
                || line.starts_with("struct ")
                || line.starts_with("namespace ")
                || line.starts_with("template")
        }
        _ => false,
    }
}

/// Find the 1-based line number for a byte offset.
fn byte_start_line(content: &str, byte_offset: usize) -> usize {
    content[..byte_offset]
        .chars()
        .filter(|&c| c == '\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_code_chunker_rust_functions() {
        let chunker = CodeChunker::new(512, 64);
        let content = r#"fn main() {
    println!("hello");
}

fn helper() -> i32 {
    42
}

fn another() {
    let x = 1;
}
"#;
        let path = Path::new("test.rs");
        let chunks = chunker.chunk(content, path);
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert!(chunk.language.as_deref() == Some("rust"));
        }
    }

    #[test]
    fn test_code_chunker_python() {
        let chunker = CodeChunker::new(512, 64);
        let content = r#"def hello():
    print("hello")

def world():
    print("world")
"#;
        let path = Path::new("test.py");
        let chunks = chunker.chunk(content, path);
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert!(chunk.language.as_deref() == Some("python"));
        }
    }

    #[test]
    fn test_code_chunker_line_fallback() {
        let chunker = CodeChunker::new(10, 2);
        let content = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\nline11\nline12\n";
        let path = Path::new("unknown.xyz");
        let chunks = chunker.chunk(content, path);
        assert!(chunks.len() > 1);
    }

    #[test]
    fn test_is_structure_start_rust() {
        assert!(is_structure_start("fn main() {", Some("rust")));
        assert!(is_structure_start("pub fn helper() -> i32 {", Some("rust")));
        assert!(is_structure_start("struct Foo {", Some("rust")));
        assert!(is_structure_start("impl Bar {", Some("rust")));
        assert!(!is_structure_start("let x = 1;", Some("rust")));
    }

    #[test]
    fn test_is_structure_start_python() {
        assert!(is_structure_start("def hello():", Some("python")));
        assert!(is_structure_start("class Foo:", Some("python")));
        assert!(!is_structure_start("x = 1", Some("python")));
    }

    #[test]
    fn test_merge_small_chunks() {
        let chunker = CodeChunker::new(100, 0);
        let content = "fn a() {}\n\nfn b() {}\n";
        let path = Path::new("test.rs");
        let chunks = chunker.chunk(content, path);
        // Both functions are small, may be merged
        assert!(!chunks.is_empty());
    }
}
