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
#[must_use]
pub fn split_into_chunks(context: &str) -> Vec<String> {
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
