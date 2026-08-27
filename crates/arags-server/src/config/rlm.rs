//! RLM recursive-summary pipeline knobs.

use serde::Deserialize;

/// RLM recursive-summary pipeline knobs.
#[derive(Debug, Clone, Deserialize)]
pub struct RlmConfig {
    /// Master switch for enqueueing summary work after indexing. `false`
    /// disables the whole RLM pipeline (nodes already stored stay readable).
    #[serde(default = "default_rlm_enabled")]
    pub enabled: bool,

    /// L2 tolerance: fraction of a theme's file hashes allowed to change
    /// before the theme summary is re-enqueued. Lower = stricter.
    #[serde(default = "default_l2_tolerance")]
    pub l2_tolerance: f64,

    /// L3 tolerance: fraction of module summaries allowed to change before
    /// the project overview is re-enqueued. Progressively higher than L2 so
    /// trivial edits never rebuild the global summary.
    #[serde(default = "default_l3_tolerance")]
    pub l3_tolerance: f64,
}

fn default_rlm_enabled() -> bool {
    true
}

fn default_l2_tolerance() -> f64 {
    0.3
}

fn default_l3_tolerance() -> f64 {
    0.5
}

impl Default for RlmConfig {
    fn default() -> Self {
        Self {
            enabled: default_rlm_enabled(),
            l2_tolerance: default_l2_tolerance(),
            l3_tolerance: default_l3_tolerance(),
        }
    }
}
