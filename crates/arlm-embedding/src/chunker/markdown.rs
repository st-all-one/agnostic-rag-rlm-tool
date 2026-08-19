use std::path::Path;

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
        let _timer = crate::Timer::new("markdown_chunking");

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

        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_markdown_chunker_headings() {
        let chunker = MarkdownChunker::new(512);
        let content = "# Title\n\nSome intro text.\n\n## Section 1\n\nContent of section 1.\n\n## Section 2\n\nContent of section 2.\n";
        let path = Path::new("test.md");
        let chunks = chunker.chunk(content, path);
        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].content.as_ref().contains("# Title"));
        assert!(chunks[1].content.as_ref().contains("## Section 1"));
        assert!(chunks[2].content.as_ref().contains("## Section 2"));
    }

    #[test]
    fn test_markdown_chunker_no_headings() {
        let chunker = MarkdownChunker::new(512);
        let content = "Just some text without any headings.\n";
        let path = Path::new("test.md");
        let chunks = chunker.chunk(content, path);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content.as_ref(), content);
    }

    #[test]
    fn test_markdown_chunker_consecutive_headings() {
        let chunker = MarkdownChunker::new(512);
        let content = "# A\n## B\n### C\n";
        let path = Path::new("test.md");
        let chunks = chunker.chunk(content, path);
        // Each heading starts a new section
        assert_eq!(chunks.len(), 3);
    }

    #[test]
    fn test_markdown_chunker_empty() {
        let chunker = MarkdownChunker::new(512);
        let content = "";
        let path = Path::new("test.md");
        let chunks = chunker.chunk(content, path);
        assert!(chunks.is_empty());
    }
}
