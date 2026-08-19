use std::sync::atomic::{AtomicU32, Ordering};

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
}
