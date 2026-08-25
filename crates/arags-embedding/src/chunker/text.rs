use std::path::Path;

use crate::chunker::{ChunkingStrategy, RawChunk, estimate_tokens, prev_char_boundary};

/// Configuration for text (paragraph/sentence) chunking.
pub struct TextChunker {
    /// Maximum tokens per chunk.
    pub max_tokens: usize,
    /// Overlap in tokens between consecutive chunks.
    pub overlap_tokens: usize,
}

/// Token cost of the `\n\n` paragraph separator included in each slice.
const SEPARATOR_TOKENS: usize = 1;

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

        // Emits `[start..end)` minus one trailing paragraph separator, so
        // chunks never carry dangling "\n\n" bytes past their counted budget.
        // Token estimates are not additive across boundaries, so if the slice
        // still exceeds the budget it is hard-split by words (guarantees the
        // "never exceed max_tokens" property tested in tests/chunker_proptest.rs).
        let push = |chunks: &mut Vec<RawChunk<'a>>, s: usize, e: usize, src: &'a str| {
            let mut end = e.min(src.len());
            if end >= 2 && &src[end - 2..end] == "\n\n" {
                end -= 2;
            }
            let cut = prev_char_boundary(src, end);
            if cut <= s {
                return;
            }
            if estimate_tokens(&src[s..cut]) > self.max_tokens {
                Self::push_word_groups(src, s, cut, self.max_tokens, chunks);
                return;
            }
            if estimate_tokens(&src[s..cut]) == 0 {
                return;
            }
            chunks.push(RawChunk {
                offset_start: s,
                offset_end: cut,
                line_start: 0,
                line_end: 0,
                content: std::borrow::Cow::Borrowed(&src[s..cut]),
                language: None,
                chunk_type: Some("paragraph".into()),
            });
        };

        // Byte offset of each paragraph inside `content` (split loses them).
        let mut cursor = 0usize;

        for para in &paragraphs {
            let para_start = cursor.min(content.len());
            let para_end = (para_start + para.len()).min(content.len());
            cursor = (para_end + 2).min(content.len()); // +2 for "\n\n"
            let para_tokens = estimate_tokens(para);

            // A single paragraph bigger than the budget is hard-split by
            // words so no emitted chunk ever exceeds `max_tokens`
            // (property-tested in tests/chunker_proptest.rs).
            if para_tokens > self.max_tokens {
                if chunk_end > chunk_start && chunk_token_count > 0 {
                    push(&mut chunks, chunk_start, chunk_end, content);
                }
                Self::push_word_groups(content, para_start, para_end, self.max_tokens, &mut chunks);
                chunk_start = cursor;
                chunk_end = cursor;
                chunk_token_count = 0;
                continue;
            }

            // If adding this paragraph would exceed the limit, flush current chunk
            if chunk_token_count + para_tokens > self.max_tokens && chunk_end > chunk_start {
                push(&mut chunks, chunk_start, chunk_end, content);

                // Overlap: the next chunk starts `overlap_tokens` worth of
                // bytes BEFORE the flush point (zero overlap ⇒ starts there).
                let overlap_bytes = estimate_overlap_bytes(content, chunk_end, self.overlap_tokens);
                chunk_start = chunk_end.saturating_sub(overlap_bytes);
                chunk_end = chunk_start;
                // The rewound overlap prefix is part of the next slice, so
                // charge it against the budget immediately.
                chunk_token_count = self.overlap_tokens + SEPARATOR_TOKENS.min(self.max_tokens + 1);
            }

            // Advance past the paragraph + separator
            let para_len = para.len() + 2; // +2 for "\n\n"
            chunk_end += para_len;
            // The separator itself costs tokens inside the emitted slice.
            chunk_token_count += para_tokens + SEPARATOR_TOKENS;
        }

        // Emit final chunk
        if chunk_end > chunk_start {
            push(&mut chunks, chunk_start, content.len(), content);
        }

        chunks
    }
}

impl TextChunker {
    /// Hard-split `content[start..end]` into consecutive word groups whose
    /// estimated token count stays within `max_tokens`.
    fn push_word_groups<'a>(
        content: &'a str,
        start: usize,
        end: usize,
        max_tokens: usize,
        chunks: &mut Vec<RawChunk<'a>>,
    ) {
        let segment = &content[start..end];
        let mut group_start = start;
        let mut group_end = start;
        for word_span in WordSpans::new(segment) {
            let candidate_end = start + word_span.1;
            let candidate = &content[group_start..candidate_end];
            if estimate_tokens(candidate) > max_tokens && candidate_end > group_start + 1 {
                let cut = prev_char_boundary(content, group_end);
                if cut > group_start {
                    chunks.push(RawChunk {
                        offset_start: group_start,
                        offset_end: cut,
                        line_start: 0,
                        line_end: 0,
                        content: std::borrow::Cow::Borrowed(&content[group_start..cut]),
                        language: None,
                        chunk_type: Some("paragraph".into()),
                    });
                }
                group_start = group_end;
            }
            group_end = candidate_end;
        }
        // Trailing remainder: shrink word-by-word until it fits the budget
        // (the leftover beyond the last fitting word is emitted on its own,
        // possibly a single unsplittable word).
        let tail_end = prev_char_boundary(content, end);
        if tail_end > group_start && estimate_tokens(&content[group_start..tail_end]) > 0 {
            let mut fit = tail_end;
            while fit > group_start && estimate_tokens(&content[group_start..fit]) > max_tokens {
                let seg = &content[group_start..fit];
                let trimmed = match seg.trim_end().rfind(char::is_whitespace) {
                    Some(i) => i + 1,
                    None => 0,
                };
                if trimmed == 0 {
                    break;
                }
                fit = group_start + trimmed;
            }
            chunks.push(RawChunk {
                offset_start: group_start,
                offset_end: fit,
                line_start: 0,
                line_end: 0,
                content: std::borrow::Cow::Borrowed(&content[group_start..fit]),
                language: None,
                chunk_type: Some("paragraph".into()),
            });
            if fit < tail_end && estimate_tokens(&content[fit..tail_end]) > 0 {
                chunks.push(RawChunk {
                    offset_start: fit,
                    offset_end: tail_end,
                    line_start: 0,
                    line_end: 0,
                    content: std::borrow::Cow::Borrowed(&content[fit..tail_end]),
                    language: None,
                    chunk_type: Some("paragraph".into()),
                });
            }
        }
    }
}

/// Byte spans of whitespace-separated words within a string slice.
struct WordSpans<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> WordSpans<'a> {
    fn new(s: &'a str) -> Self {
        Self { s, pos: 0 }
    }
}

impl Iterator for WordSpans<'_> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let bytes = self.s.as_bytes();
        while self.pos < self.s.len() && bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        if self.pos >= self.s.len() {
            return None;
        }
        let start = self.pos;
        while self.pos < self.s.len() && !bytes[self.pos].is_ascii_whitespace() {
            // Multi-byte UTF-8 continuation bytes are not ASCII whitespace,
            // so this always lands on a char boundary.
            self.pos += 1;
        }
        Some((start, self.pos))
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
