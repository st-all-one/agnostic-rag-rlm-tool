use std::path::Path;

use crate::chunker::{ChunkingStrategy, RawChunk, estimate_tokens, prev_char_boundary};

/// Configuration for text (paragraph/sentence) chunking.
pub struct TextChunker {
    /// Maximum tokens per chunk.
    pub max_tokens: usize,
    /// Overlap in tokens between consecutive chunks.
    pub overlap_tokens: usize,
}

impl TextChunker {
    #[must_use]
    pub fn new(max_tokens: usize, overlap_tokens: usize) -> Self {
        Self {
            max_tokens,
            overlap_tokens,
        }
    }
}

impl ChunkingStrategy for TextChunker {
    fn chunk<'a>(&self, content: &'a str, _path: &Path) -> Vec<RawChunk<'a>> {
        let _timer = crate::Timer::new("text_chunking");

        // Split on double-newlines (paragraph boundaries)
        let paragraphs: Vec<&str> = content.split("\n\n").collect();
        let mut chunks = Vec::with_capacity(paragraphs.len() / 2 + 1);

        let mut chunk_start = 0usize;
        let mut chunk_end = 0usize;
        let mut chunk_token_count = 0usize;

        for para in &paragraphs {
            let para_tokens = estimate_tokens(para);

            // If adding this paragraph would exceed the limit, flush current chunk
            if chunk_token_count + para_tokens > self.max_tokens && chunk_end > chunk_start {
                let slice = &content[chunk_start..prev_char_boundary(content, chunk_end)];
                chunks.push(RawChunk {
                    offset_start: chunk_start,
                    offset_end: prev_char_boundary(content, chunk_end),
                    line_start: 0,
                    line_end: 0,
                    content: std::borrow::Cow::Borrowed(slice),
                    language: None,
                    chunk_type: Some("paragraph".into()),
                });

                // Overlap: back up by overlap_tokens worth of bytes
                let overlap_bytes =
                    estimate_overlap_bytes(content, chunk_start, self.overlap_tokens);
                chunk_start = chunk_start.saturating_sub(overlap_bytes);
                chunk_end = chunk_start;
                chunk_token_count = 0;
            }

            // Advance past the paragraph + separator
            let para_len = para.len() + 2; // +2 for "\n\n"
            chunk_end += para_len;
            chunk_token_count += para_tokens;
        }

        // Emit final chunk
        if chunk_end > chunk_start {
            let slice = &content[chunk_start..];
            if estimate_tokens(slice) > 0 {
                chunks.push(RawChunk {
                    offset_start: chunk_start,
                    offset_end: content.len(),
                    line_start: 0,
                    line_end: 0,
                    content: std::borrow::Cow::Borrowed(slice),
                    language: None,
                    chunk_type: Some("paragraph".into()),
                });
            }
        }

        chunks
    }
}

/// Estimate byte overlap from token count.
fn estimate_overlap_bytes(content: &str, chunk_start: usize, overlap_tokens: usize) -> usize {
    if overlap_tokens == 0 {
        return 0;
    }
    // Walk backwards from chunk_start counting words
    let prefix = &content[..chunk_start];
    let word_count = prefix.split_whitespace().count();
    let target_words = word_count.saturating_sub(overlap_tokens);
    let mut byte_offset = 0usize;
    for (count, word) in prefix.split_whitespace().enumerate() {
        if count >= target_words {
            break;
        }
        // Find the byte position of this word
        if let Some(pos) = prefix[byte_offset..].find(word) {
            byte_offset += pos;
        }
        byte_offset += word.len();
    }
    byte_offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_text_chunker_paragraphs() {
        let chunker = TextChunker::new(20, 0);
        let content = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
        let path = Path::new("test.txt");
        let chunks = chunker.chunk(content, path);
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert!(chunk.chunk_type.as_deref() == Some("paragraph"));
        }
    }

    #[test]
    fn test_text_chunker_single_paragraph() {
        let chunker = TextChunker::new(1000, 0);
        let content = "A single paragraph without any breaks.";
        let path = Path::new("test.txt");
        let chunks = chunker.chunk(content, path);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content.as_ref(), content);
    }

    #[test]
    fn test_text_chunker_empty() {
        let chunker = TextChunker::new(100, 0);
        let content = "";
        let path = Path::new("test.txt");
        let chunks = chunker.chunk(content, path);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_text_chunker_many_paragraphs() {
        let chunker = TextChunker::new(10, 0);
        let content = (0..20)
            .map(|i| format!("Paragraph {i} with some words."))
            .collect::<Vec<_>>()
            .join("\n\n");
        let path = Path::new("test.txt");
        let chunks = chunker.chunk(&content, path);
        assert!(chunks.len() > 1);
    }
}
