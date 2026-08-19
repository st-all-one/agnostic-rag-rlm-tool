use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

/// Model context window limits (in tokens).
///
/// Keyed by substring — the longest matching key wins.
/// Fallback: 128K tokens.
static MODEL_CONTEXT_LIMITS: OnceLock<HashMap<&'static str, u32>> = OnceLock::new();

fn context_limits() -> &'static HashMap<&'static str, u32> {
    MODEL_CONTEXT_LIMITS.get_or_init(|| {
        let mut m = HashMap::new();
        // OpenAI
        m.insert("gpt-4o", 128_000);
        m.insert("gpt-4o-mini", 128_000);
        m.insert("gpt-4-turbo", 128_000);
        m.insert("gpt-4", 8_192);
        m.insert("gpt-3.5-turbo", 16_385);
        m.insert("o1", 200_000);
        m.insert("o3", 200_000);
        // Anthropic
        m.insert("claude-4", 1_000_000);
        m.insert("claude-3.5-sonnet", 200_000);
        m.insert("claude-3-opus", 200_000);
        m.insert("claude-3-sonnet", 200_000);
        m.insert("claude-3-haiku", 200_000);
        m.insert("claude", 200_000);
        // Google
        m.insert("gemini-2.5", 1_000_000);
        m.insert("gemini-2.0", 1_000_000);
        m.insert("gemini-1.5-pro", 2_000_000);
        m.insert("gemini-1.5-flash", 1_000_000);
        m.insert("gemini", 1_000_000);
        // DeepSeek
        m.insert("deepseek-v4", 1_000_000);
        m.insert("deepseek-v3", 131_072);
        m.insert("deepseek-r1", 131_072);
        m.insert("deepseek", 131_072);
        // MiMo
        m.insert("mimo", 131_072);
        // Qwen
        m.insert("qwen3-max", 256_000);
        m.insert("qwen3", 131_072);
        // Meta
        m.insert("llama-4", 1_000_000);
        m.insert("llama-3.3", 131_072);
        m.insert("llama-3.1", 131_072);
        m.insert("llama-3", 8_192);
        m
    })
}

/// Look up the context window limit for a model by substring match.
///
/// Returns the limit for the longest matching key, or 128K as fallback.
#[must_use]
pub fn get_context_limit(model_name: &str) -> u32 {
    let limits = context_limits();
    let lower = model_name.to_lowercase();

    // Find longest matching key
    let mut best_len = 0;
    let mut best_limit = 128_000;
    for (key, &limit) in limits {
        if lower.contains(key) && key.len() > best_len {
            best_len = key.len();
            best_limit = limit;
        }
    }
    best_limit
}

/// Token counter for tracking usage across the RLM engine.
///
/// Uses a word-count heuristic (words × 1.3 ≈ tokens) for cheap estimation.
/// Thread-safe via `AtomicU32` — no mutex needed.
#[derive(Debug)]
pub struct TokenCounter {
    prompt: AtomicU32,
    completion: AtomicU32,
    budget: u32,
}

impl TokenCounter {
    /// Create a new token counter with a budget limit.
    #[must_use]
    pub fn new(budget: u32) -> Self {
        Self {
            prompt: AtomicU32::new(0),
            completion: AtomicU32::new(0),
            budget,
        }
    }

    /// Estimate token count from text using word-count heuristic.
    ///
    /// Splits on whitespace and multiplies by 1.3 as a rough approximation.
    /// Cheap — no real tokenizer needed.
    #[must_use]
    pub fn estimate(text: &str) -> u32 {
        if text.is_empty() {
            return 0;
        }
        let words = text.split_whitespace().count();
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let tokens = (words as f64 * 1.3).ceil() as u32;
        tokens
    }

    /// Record tokens from an LLM call (prompt + completion).
    pub fn record(&self, prompt: u32, completion: u32) {
        self.prompt.fetch_add(prompt, Ordering::Relaxed);
        self.completion.fetch_add(completion, Ordering::Relaxed);
    }

    /// Total tokens used so far.
    #[must_use]
    pub fn total_used(&self) -> u32 {
        self.prompt.load(Ordering::Relaxed) + self.completion.load(Ordering::Relaxed)
    }

    /// Remaining tokens in budget.
    #[must_use]
    pub fn remaining(&self) -> u32 {
        self.budget.saturating_sub(self.total_used())
    }

    /// Check if we are still within budget.
    #[must_use]
    pub fn is_within_budget(&self) -> bool {
        self.remaining() > 0
    }

    /// Check budget, returning an error if exhausted.
    ///
    /// # Errors
    ///
    /// Returns an error if the token budget is exhausted.
    pub fn check_budget(&self) -> anyhow::Result<()> {
        if !self.is_within_budget() {
            anyhow::bail!("token budget exhausted");
        }
        Ok(())
    }

    /// Prompt tokens used.
    #[must_use]
    pub fn prompt_tokens(&self) -> u32 {
        self.prompt.load(Ordering::Relaxed)
    }

    /// Completion tokens used.
    #[must_use]
    pub fn completion_tokens(&self) -> u32 {
        self.completion.load(Ordering::Relaxed)
    }

    /// Maximum token budget.
    #[must_use]
    pub fn max_tokens(&self) -> u32 {
        self.budget
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.prompt.store(0, Ordering::Relaxed);
        self.completion.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::manual_range_contains
)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_empty() {
        assert_eq!(TokenCounter::estimate(""), 0);
    }

    #[test]
    fn test_estimate_single_word() {
        let est = TokenCounter::estimate("hello");
        assert!(est >= 1 && est <= 2);
    }

    #[test]
    fn test_estimate_multiple_words() {
        let est = TokenCounter::estimate("this is a test sentence");
        assert!(est >= 5 && est <= 10);
    }

    #[test]
    fn test_record_and_total() {
        let tc = TokenCounter::new(1000);
        assert_eq!(tc.total_used(), 0);
        tc.record(100, 50);
        assert_eq!(tc.total_used(), 150);
        assert_eq!(tc.prompt_tokens(), 100);
        assert_eq!(tc.completion_tokens(), 50);
    }

    #[test]
    fn test_remaining() {
        let tc = TokenCounter::new(1000);
        assert_eq!(tc.remaining(), 1000);
        tc.record(300, 200);
        assert_eq!(tc.remaining(), 500);
    }

    #[test]
    fn test_is_within_budget() {
        let tc = TokenCounter::new(100);
        assert!(tc.is_within_budget());
        tc.record(50, 50);
        assert!(!tc.is_within_budget());
    }

    #[test]
    fn test_check_budget_ok() {
        let tc = TokenCounter::new(100);
        tc.record(30, 20);
        assert!(tc.check_budget().is_ok());
    }

    #[test]
    fn test_check_budget_exhausted() {
        let tc = TokenCounter::new(100);
        tc.record(50, 50);
        assert!(tc.check_budget().is_err());
    }

    #[test]
    fn test_reset() {
        let tc = TokenCounter::new(1000);
        tc.record(100, 200);
        assert_eq!(tc.total_used(), 300);
        tc.reset();
        assert_eq!(tc.total_used(), 0);
        assert_eq!(tc.remaining(), 1000);
    }

    #[test]
    fn test_multiple_records() {
        let tc = TokenCounter::new(1000);
        tc.record(100, 50);
        tc.record(200, 100);
        tc.record(50, 25);
        assert_eq!(tc.prompt_tokens(), 350);
        assert_eq!(tc.completion_tokens(), 175);
        assert_eq!(tc.total_used(), 525);
        assert_eq!(tc.remaining(), 475);
    }

    #[test]
    fn test_saturating_sub() {
        let tc = TokenCounter::new(100);
        tc.record(80, 50);
        assert_eq!(tc.remaining(), 0);
    }

    #[test]
    fn test_max_tokens_accessor() {
        let tc = TokenCounter::new(500);
        assert_eq!(tc.max_tokens(), 500);
    }

    #[test]
    fn test_estimate_long_text() {
        let text = "The quick brown fox jumps over the lazy dog. \
                     This is a longer sentence with many words to estimate tokens.";
        let est = TokenCounter::estimate(text);
        assert!(est > 20);
    }

    #[test]
    fn test_get_context_limit_gpt4o() {
        assert_eq!(get_context_limit("gpt-4o"), 128_000);
        assert_eq!(get_context_limit("gpt-4o-mini"), 128_000);
    }

    #[test]
    fn test_get_context_limit_claude() {
        assert_eq!(get_context_limit("claude-4-sonnet"), 1_000_000);
        assert_eq!(get_context_limit("claude-3.5-sonnet"), 200_000);
        assert_eq!(get_context_limit("claude-3-opus"), 200_000);
    }

    #[test]
    fn test_get_context_limit_gemini() {
        assert_eq!(get_context_limit("gemini-2.5-pro"), 1_000_000);
        assert_eq!(get_context_limit("gemini-1.5-pro"), 2_000_000);
    }

    #[test]
    fn test_get_context_limit_deepseek() {
        assert_eq!(get_context_limit("deepseek-v3"), 131_072);
        assert_eq!(get_context_limit("deepseek-r1"), 131_072);
    }

    #[test]
    fn test_get_context_limit_mimo() {
        assert_eq!(get_context_limit("mimo"), 131_072);
    }

    #[test]
    fn test_get_context_limit_unknown_fallback() {
        assert_eq!(get_context_limit("some-unknown-model"), 128_000);
    }

    #[test]
    fn test_get_context_limit_longest_match() {
        // "gpt-4" matches "gpt-4" (8192) but "gpt-4o" matches "gpt-4o" (128000)
        assert_eq!(get_context_limit("gpt-4o-2024-08-06"), 128_000);
    }
}
