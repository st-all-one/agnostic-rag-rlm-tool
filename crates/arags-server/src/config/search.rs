//! Search serving defaults (plan 020).

use serde::Deserialize;

/// Search serving defaults (plan 020). Applied by the handlers when a request
/// omits the corresponding field.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchConfig {
    /// Default tier when a request does not specify one: `hybrid` (default),
    /// `fts`, `entity` or `vector`.
    #[serde(default = "default_search_tier")]
    pub tier: String,
    /// Default `top_k` for requests without an explicit limit.
    #[serde(default = "default_search_top_k")]
    pub top_k: usize,
    /// Default token budget for rendered context.
    #[serde(default = "default_search_max_tokens")]
    pub max_tokens: u32,
    /// Serving-path salience decay rate (`score * e^(-lambda * age_hours)`).
    /// Applied after RRF fusion when > 0; `0` disables decay at query time.
    #[serde(default)]
    pub decay_lambda: f64,
    /// Unified query (plan 023): share of the result budget given to approved
    /// RLM summaries when they are available (`0.0..=1.0`, default 0.6).
    /// `0` disables summary fusion entirely.
    #[serde(default = "default_summary_ratio")]
    pub summary_ratio: f64,
    /// Unified query: minimum normalized score for an RLM summary to be
    /// considered for fusion.
    #[serde(default = "default_summary_min_score")]
    pub summary_min_score: f64,
    /// Unified query: surface relevant exploration maps in search responses.
    #[serde(default = "default_exploration_in_query")]
    pub exploration_enabled: bool,
    /// Unified query: maximum number of exploration hits attached per query.
    #[serde(default = "default_exploration_limit")]
    pub exploration_limit: usize,
}

fn default_search_tier() -> String {
    "hybrid".to_string()
}

fn default_search_top_k() -> usize {
    10
}

fn default_search_max_tokens() -> u32 {
    8000
}

fn default_summary_ratio() -> f64 {
    0.6
}

fn default_summary_min_score() -> f64 {
    0.35
}

fn default_exploration_in_query() -> bool {
    true
}

fn default_exploration_limit() -> usize {
    2
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            tier: default_search_tier(),
            top_k: default_search_top_k(),
            max_tokens: default_search_max_tokens(),
            decay_lambda: 0.0,
            summary_ratio: default_summary_ratio(),
            summary_min_score: default_summary_min_score(),
            exploration_enabled: default_exploration_in_query(),
            exploration_limit: default_exploration_limit(),
        }
    }
}
