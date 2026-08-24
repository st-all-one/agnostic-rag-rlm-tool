use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

/// Model context window limits (in tokens).
///
/// Keyed by substring — the longest matching key wins. Fallback: 128K tokens.
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

/// Convenience wrapper over [`get_context_limit`].
#[derive(Debug, Clone, Copy)]
pub struct ModelContextLimits;

impl ModelContextLimits {
    /// Resolve the context window (in tokens) for `model`.
    #[must_use]
    pub fn limit_for(model: &str) -> u32 {
        get_context_limit(model)
    }
}

/// Token counter for tracking usage across the LLM engine.
///
/// Uses a character/punctuation heuristic for cheap estimation and is
/// thread-safe via `AtomicU32` (no mutex needed).
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

    /// Estimate token count from text using a character/punctuation heuristic.
    ///
    /// Assumes roughly 4 characters per token, plus a small surcharge for
    /// ASCII punctuation (each punctuation char adds ~0.25 token). The result
    /// is `ceil(chars/4 + punctuation/4)`, staying within ~15% of most English
    /// tokenizers for code and prose.
    #[must_use]
    pub fn estimate(text: &str) -> u32 {
        if text.is_empty() {
            return 0;
        }
        let chars = text.chars().count();
        let punctuation = text.chars().filter(char::is_ascii_punctuation).count();
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let tokens = ((chars as f64 / 4.0) + (punctuation as f64 / 4.0)).ceil() as u32;
        tokens.max(1)
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
