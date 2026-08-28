use std::path::Path;
use std::time::Instant;

use tracing::debug;

use crate::chunker::{ChunkingStrategy, RawChunk};

/// Configuration for markdown heading-based chunking.
pub struct MarkdownChunker {
    /// Maximum tokens per chunk. Sections exceeding this limit
    /// are emitted as-is (no sub-splitting).
    pub max_tokens: usize,
}

impl MarkdownChunker {
    #[must_use]
    pub fn new(max_tokens: usize) -> Self {
        Self { max_tokens }
    }
}

impl ChunkingStrategy for MarkdownChunker {
    fn chunk<'a>(&self, content: &'a str, _path: &Path) -> Vec<RawChunk<'a>> {
        let start = Instant::now();

        let mut chunks = Vec::with_capacity(64);
        let mut section_start = 0usize;
        let mut byte_offset = 0usize;

        for line in content.split_inclusive('\n') {
            let is_heading = line.trim_start().starts_with('#');

            if is_heading && byte_offset > section_start {
                // Close previous section
                let slice = &content[section_start..byte_offset];
                chunks.push(RawChunk {
                    offset_start: section_start,
                    offset_end: byte_offset,
                    line_start: 0,
                    line_end: 0,
                    content: std::borrow::Cow::Borrowed(slice),
                    language: None,
                    chunk_type: Some("heading".into()),
                });
                section_start = byte_offset;
            }

            byte_offset += line.len();
        }

        // Emit final section
        if byte_offset > section_start {
            let slice = &content[section_start..];
            if !slice.trim().is_empty() {
                chunks.push(RawChunk {
                    offset_start: section_start,
                    offset_end: content.len(),
                    line_start: 0,
                    line_end: 0,
                    content: std::borrow::Cow::Borrowed(slice),
                    language: None,
                    chunk_type: Some("heading".into()),
                });
            }
        }

        debug!(
            chunk_count = chunks.len(),
            chars = content.len(),
            duration_ms = %start.elapsed().as_millis(),
            "chunked markdown"
        );
        chunks
    }
}
