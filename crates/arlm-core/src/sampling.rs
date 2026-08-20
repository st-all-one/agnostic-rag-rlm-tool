use serde::{Deserialize, Serialize};

use crate::types::Action;

/// Sampling parameters for LLM requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingArgs {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: Option<u32>,
    /// Optional deterministic seed for reproducible sampling.
    ///
    /// `arlm-core` carries this value so callers/backends that support seeding can read
    /// it via [`SamplingArgs::seed`]. The underlying `arlm-llm::CompletionRequest` does not
    /// yet expose a `seed` field, so it is preserved here rather than pushed onto the wire.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl SamplingArgs {
    /// Create sampling args tailored to the node type in the RLM tree.
    #[must_use]
    pub fn for_node_type(action: Action) -> Self {
        match action {
            Action::Solve => Self {
                temperature: 0.3,
                top_p: 0.9,
                top_k: None,
                seed: None,
            },
            Action::Decompose => Self {
                temperature: 0.1,
                top_p: 0.85,
                top_k: None,
                seed: None,
            },
        }
    }

    /// Set a deterministic seed (builder-style).
    #[must_use]
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Get the configured seed, if any.
    #[must_use]
    pub fn seed(&self) -> Option<u64> {
        self.seed
    }

    /// Apply these sampling args to a `CompletionRequest` by setting the
    /// temperature field. Returns the request unchanged if temperature is
    /// already set. The `seed` (if present) is carried on `SamplingArgs` for
    /// backends that support deterministic sampling.
    #[must_use]
    pub fn apply_to_request(
        self,
        mut req: arlm_llm::CompletionRequest,
    ) -> arlm_llm::CompletionRequest {
        if req.temperature.is_none() {
            req.temperature = Some(self.temperature);
        }
        req
    }
}
