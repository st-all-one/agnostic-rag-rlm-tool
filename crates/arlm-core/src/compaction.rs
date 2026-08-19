use crate::token_counter::TokenCounter;

/// A search result chunk with score and content for compaction.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub score: f32,
    pub content: String,
    pub file_path: String,
}

/// Context compaction to keep accumulated context within token limits.
///
/// When the RLM engine accumulates too much context from search results
/// and node outputs, compaction identifies the most important parts,
/// summarizes or truncates less important parts, and keeps context
/// within token limits.
#[derive(Debug, Clone)]
pub struct Compaction {
    max_tokens: u32,
    /// Number of most recent chunks to always keep (recency bias).
    recency_keep: usize,
}

impl Compaction {
    /// Create a new compaction policy with a token budget.
    #[must_use]
    pub fn new(max_tokens: u32) -> Self {
        Self {
            max_tokens,
            recency_keep: 3,
        }
    }

    /// Create a compaction policy with custom recency bias.
    #[must_use]
    pub fn with_recency_keep(max_tokens: u32, recency_keep: usize) -> Self {
        Self {
            max_tokens,
            recency_keep,
        }
    }

    /// Compact context by keeping highest-scored chunks within token budget.
    ///
    /// Splits context into chunks separated by `## ` headers, scores them
    /// using provided `SearchResult` scores, applies recency bias to keep
    /// the most recent chunks, and reassembles within the token budget.
    #[must_use]
    pub fn compact(&self, context: &str, results: &[SearchResult]) -> String {
        if context.is_empty() || results.is_empty() {
            return context.to_string();
        }

        let total_tokens = TokenCounter::estimate(context);
        if total_tokens <= self.max_tokens {
            return context.to_string();
        }

        let chunks = split_into_chunks(context);
        if chunks.is_empty() {
            return context.to_string();
        }

        let mut scored: Vec<(usize, f32, &str)> = chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| {
                let score = results.get(i).map_or(0.0, |r| r.score);
                (i, score, chunk.as_str())
            })
            .collect();

        let last_n = scored.len().saturating_sub(self.recency_keep);
        let recent: Vec<(usize, f32, &str)> = scored.split_off(last_n);

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut selected: Vec<&str> = Vec::new();
        let mut used_tokens: u32 = 0;

        for chunk in &recent {
            let chunk_tokens = TokenCounter::estimate(chunk.2);
            if used_tokens + chunk_tokens <= self.max_tokens {
                selected.push(chunk.2);
                used_tokens += chunk_tokens;
            }
        }

        for chunk in &scored {
            let chunk_tokens = TokenCounter::estimate(chunk.2);
            if used_tokens + chunk_tokens <= self.max_tokens {
                selected.push(chunk.2);
                used_tokens += chunk_tokens;
            }
        }

        let mut output = String::new();
        for (i, chunk) in selected.iter().enumerate() {
            if i > 0 {
                output.push_str("\n\n");
            }
            output.push_str(chunk);
        }

        output
    }

    /// Maximum token budget.
    #[must_use]
    pub fn max_tokens(&self) -> u32 {
        self.max_tokens
    }
}

/// Split context into chunks by `## ` headers (markdown section boundaries).
fn split_into_chunks(context: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for line in context.lines() {
        if line.starts_with("## ") && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        current.push_str(line);
        current.push('\n');
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::single_char_pattern
)]
mod tests {
    use super::*;

    fn make_result(score: f32, file_path: &str) -> SearchResult {
        SearchResult {
            score,
            content: format!("content of {file_path}"),
            file_path: file_path.to_string(),
        }
    }

    #[test]
    fn test_new() {
        let c = Compaction::new(1000);
        assert_eq!(c.max_tokens(), 1000);
    }

    #[test]
    fn test_with_recency_keep() {
        let c = Compaction::with_recency_keep(1000, 5);
        assert_eq!(c.max_tokens(), 1000);
    }

    #[test]
    fn test_compact_empty_context() {
        let c = Compaction::new(1000);
        let result = c.compact("", &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_compact_empty_results() {
        let c = Compaction::new(1000);
        let ctx = "## Section 1\nHello world";
        let result = c.compact(ctx, &[]);
        assert_eq!(result, ctx);
    }

    #[test]
    fn test_compact_within_budget() {
        let c = Compaction::new(10_000);
        let ctx = "## Section 1\nHello world";
        let results = vec![make_result(0.9, "a.rs")];
        let result = c.compact(ctx, &results);
        assert_eq!(result, ctx);
    }

    #[test]
    fn test_compact_splits_by_headers() {
        let c = Compaction::new(50);
        let ctx = "## Section 1\nSome content here\n## Section 2\nMore content here\n## Section 3\nEven more content\n";
        let results = vec![
            make_result(0.5, "a.rs"),
            make_result(0.9, "b.rs"),
            make_result(0.3, "c.rs"),
        ];
        let result = c.compact(ctx, &results);
        assert!(result.contains("Section 2"));
        assert!(result.contains("Section 1"));
    }

    #[test]
    fn test_compact_keeps_highest_scored() {
        let c = Compaction::with_recency_keep(5, 0);
        let ctx = "## Low\nshort\n## High\nshort\n## Mid\nshort\n";
        let results = vec![
            make_result(0.2, "low.rs"),
            make_result(0.9, "high.rs"),
            make_result(0.5, "mid.rs"),
        ];
        let result = c.compact(ctx, &results);
        assert!(result.contains("High"));
        assert!(!result.contains("Low"));
    }

    #[test]
    fn test_compact_recency_bias() {
        let c = Compaction::with_recency_keep(80, 2);
        let ctx = "## Old1\nshort\n## Old2\nshort\n## Recent1\nshort\n## Recent2\nshort\n";
        let results = vec![
            make_result(0.1, "old1.rs"),
            make_result(0.1, "old2.rs"),
            make_result(0.1, "recent1.rs"),
            make_result(0.1, "recent2.rs"),
        ];
        let result = c.compact(ctx, &results);
        assert!(result.contains("Recent1"));
        assert!(result.contains("Recent2"));
    }

    #[test]
    fn test_compact_falls_back_to_context() {
        let c = Compaction::with_recency_keep(10, 0);
        let ctx = "## A\nxx\n## B\nyy\n## C\nzz\n";
        let results = vec![
            make_result(0.5, "a.rs"),
            make_result(0.5, "b.rs"),
            make_result(0.5, "c.rs"),
        ];
        let result = c.compact(ctx, &results);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_split_into_chunks() {
        let ctx = "## A\nfoo\n## B\nbar\n";
        let chunks = split_into_chunks(ctx);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("A"));
        assert!(chunks[1].contains("B"));
    }

    #[test]
    fn test_split_no_headers() {
        let ctx = "just some text\nno headers here\n";
        let chunks = split_into_chunks(ctx);
        assert_eq!(chunks.len(), 1);
    }
}
