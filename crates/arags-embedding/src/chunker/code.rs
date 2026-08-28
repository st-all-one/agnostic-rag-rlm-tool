pub mod util;

pub use util::{is_structure_start, merge_small_chunks};

use std::path::Path;
use std::time::Instant;

use tracing::debug;

use crate::chunker::{
    ChunkingStrategy, RawChunk, detect_language, estimate_tokens, nth_line_byte_offset,
    prev_char_boundary,
};
use util::byte_start_line;

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
        let start = Instant::now();
        let language = detect_language(path);

        let chunks = match language.as_deref() {
            Some(
                "rust" | "python" | "javascript" | "typescript" | "go" | "java" | "cpp" | "c"
                | "ruby" | "php",
            ) => self.chunk_by_structures(content, language.as_deref()),
            _ => self.chunk_by_lines(content, language.as_deref()),
        };

        debug!(
            chunk_count = chunks.len(),
            chars = content.len(),
            language = language.as_deref().unwrap_or("unknown"),
            duration_ms = %start.elapsed().as_millis(),
            "chunked code"
        );
        chunks
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
                block_end += line.len();
            }
        }

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

        if chunks.is_empty() {
            return self.chunk_by_lines(content, language);
        }

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

                let overlap_lines = self.overlap_tokens.min(current_lines - 1);
                line_start = i + 1 - overlap_lines;
                byte_start = nth_line_byte_offset(content, line_start);
                current_lines = overlap_lines;
            }
        }

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
