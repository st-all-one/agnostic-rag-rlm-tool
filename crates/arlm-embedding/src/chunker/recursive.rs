use std::path::Path;

use crate::chunker::{ChunkingStrategy, RawChunk, estimate_tokens, prev_char_boundary};

/// A separator used to split text recursively.
struct Separator {
    pattern: &'static str,
}

/// Configuration for recursive size-based chunking.
///
/// Attempts to split on a hierarchy of separators, recursing into smaller
/// pieces when a chunk exceeds `max_tokens`. This is the most general-purpose
/// chunker, suitable for arbitrary text formats.
pub struct RecursiveChunker {
    /// Maximum tokens per chunk.
    pub max_tokens: usize,
    /// Overlap in tokens between consecutive chunks.
    pub overlap_tokens: usize,
    /// Separators in priority order (first tried, then fallback).
    separators: Vec<Separator>,
}

impl RecursiveChunker {
    /// Create a new recursive chunker with default separators:
    /// `\n\n`, `\n`, `. `, ` `, `""`.
    #[must_use]
    pub fn new(max_tokens: usize, overlap_tokens: usize) -> Self {
        Self {
            max_tokens,
            overlap_tokens,
            separators: vec![
                Separator { pattern: "\n\n" },
                Separator { pattern: "\n" },
                Separator { pattern: ". " },
                Separator { pattern: " " },
            ],
        }
    }

    /// Create with custom separator list.
    #[must_use]
    pub fn with_separators(
        max_tokens: usize,
        overlap_tokens: usize,
        separators: Vec<&'static str>,
    ) -> Self {
        Self {
            max_tokens,
            overlap_tokens,
            separators: separators
                .into_iter()
                .map(|p| Separator { pattern: p })
                .collect(),
        }
    }

    fn chunk_recursive<'a>(
        &self,
        content: &'a str,
        separator_idx: usize,
        chunk_start: usize,
    ) -> Vec<RawChunk<'a>> {
        let tokens = estimate_tokens(content);

        if tokens <= self.max_tokens {
            if tokens == 0 {
                return Vec::new();
            }
            return vec![RawChunk {
                offset_start: chunk_start,
                offset_end: chunk_start + content.len(),
                line_start: 0,
                line_end: 0,
                content: std::borrow::Cow::Borrowed(content),
                language: None,
                chunk_type: Some("recursive".into()),
            }];
        }

        // Try current separator
        if separator_idx < self.separators.len() {
            let sep = &self.separators[separator_idx];
            let pieces: Vec<&str> = content.split(sep.pattern).collect();
            let mut chunks = Vec::with_capacity(pieces.len());
            let mut offset = chunk_start;

            for (i, piece) in pieces.iter().enumerate() {
                // Re-add the separator (except for the last piece)
                let piece_content = if i < pieces.len() - 1 {
                    // Borrow from original content for zero-copy
                    let piece_start = piece.as_ptr() as usize - content.as_ptr() as usize;
                    let piece_end = piece_start + piece.len();
                    &content[piece_start..piece_end + sep.pattern.len()]
                } else {
                    let piece_start = piece.as_ptr() as usize - content.as_ptr() as usize;
                    &content[piece_start..piece_start + piece.len()]
                };

                let sub_chunks = self.chunk_recursive(piece_content, separator_idx + 1, offset);
                chunks.extend(sub_chunks);
                offset += piece_content.len();
            }

            return chunks;
        }

        // Last resort: hard split at max_tokens boundary
        let mut chunks = Vec::with_capacity(4);
        let mut start = 0usize;

        while start < content.len() {
            let end = prev_char_boundary(content, (start + self.max_tokens * 4).min(content.len()));
            let slice = &content[start..end];
            if estimate_tokens(slice) > 0 {
                chunks.push(RawChunk {
                    offset_start: chunk_start + start,
                    offset_end: chunk_start + end,
                    line_start: 0,
                    line_end: 0,
                    content: std::borrow::Cow::Borrowed(slice),
                    language: None,
                    chunk_type: Some("recursive".into()),
                });
            }
            start = end;
        }

        chunks
    }
}

impl ChunkingStrategy for RecursiveChunker {
    fn chunk<'a>(&self, content: &'a str, _path: &Path) -> Vec<RawChunk<'a>> {
        let _timer = crate::Timer::new("recursive_chunking");
        self.chunk_recursive(content, 0, 0)
    }
}
