//! Property-based tests for text chunking (proptest, plan 021 §7.4).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use arags_embedding::chunker::text::TextChunker;
use arags_embedding::chunker::{ChunkingStrategy, estimate_tokens};
use proptest::prelude::*;

fn strategy() -> impl Strategy<Value = String> {
    // Paragraphs of printable ASCII words; avoids multi-byte boundary noise
    // while still stressing paragraph/flush/overlap logic.
    proptest::collection::vec("[a-z]{1,12}( [a-z]{1,12}){0,30}", 1..=40)
        .prop_map(|paras| paras.join("\n\n"))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn chunks_respect_max_tokens(max in 8usize..=64, overlap in 0usize..=8, content in strategy()) {
        let chunker = TextChunker::new(max, overlap.min(max.saturating_sub(1)));
        let chunks = chunker.chunk(&content, std::path::Path::new("f.txt"));
        for c in &chunks {
            let tokens = estimate_tokens(&c.content);
            let single_word = !c.content.trim().contains(char::is_whitespace);
            prop_assert!(
                tokens <= max.max(1) || single_word,
                "multi-word chunk with {tokens} tokens exceeds max {max} (unsplittable single words may)"
            );
            prop_assert!(c.offset_start <= c.offset_end);
            prop_assert_eq!(c.offset_end - c.offset_start, c.content.len());
        }
    }

    #[test]
    fn chunks_preserve_all_content(max in 8usize..=48, content in strategy()) {
        let chunker = TextChunker::new(max, 0);
        let chunks = chunker.chunk(&content, std::path::Path::new("f.txt"));
        if content.trim().is_empty() {
            return Ok(());
        }
        // Every input word must appear in some chunk (no content loss).
        for word in content.split_whitespace() {
            let found = chunks.iter().any(|c| c.content.split_whitespace().any(|w| w == word));
            prop_assert!(found, "word {word:?} lost during chunking");
        }
        prop_assert!(!chunks.is_empty());
    }

    #[test]
    fn offsets_are_valid_slices(max in 8usize..=64, content in strategy()) {
        let chunker = TextChunker::new(max, 2);
        let chunks = chunker.chunk(&content, std::path::Path::new("f.txt"));
        for c in &chunks {
            prop_assert!(c.offset_end <= content.len());
            prop_assert_eq!(&content[c.offset_start..c.offset_start + c.content.len()], &*c.content);
        }
    }
}
