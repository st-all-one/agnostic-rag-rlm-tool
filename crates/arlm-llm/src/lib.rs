#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::needless_borrow,
        clippy::unnecessary_literal_bound,
        clippy::float_cmp,
        clippy::duration_suboptimal_units,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )
)]
#![allow(clippy::doc_markdown)]

pub mod backend;
pub mod config;
pub mod factory;
pub mod fallback;
pub mod pricing;
pub mod retry;
pub mod token_counter;
pub mod trait_llm;
pub mod transport;
pub mod types;

pub use backend::GenericBackend;
pub use config::{AuthScheme, BackendConfig, BackendFamily, HealthMethod, LlmConfig};
pub use factory::{BackendKind, get_backend, get_backend_from_config};
pub use fallback::ModelFallback;
pub use pricing::{ModelPricing, PricingTable};
pub use retry::RetryConfig;
pub use token_counter::{ModelContextLimits, TokenCounter};
pub use trait_llm::LlmBackend;
pub use types::{
    CompletionRequest, CompletionResponse, LlmError, Message, Role, ToolDefinition, UsageSummary,
};

#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Scoped timing helper.
///
/// Logs the elapsed time of a code region on drop, with a structured
/// `timer` field so it can be filtered/aggregated in logs.
pub(crate) struct Timer {
    name: &'static str,
    start: std::time::Instant,
}

impl Timer {
    #[must_use]
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            name,
            start: std::time::Instant::now(),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let elapsed_ms = self.start.elapsed().as_millis();
        tracing::info!(timer = %self.name, elapsed_ms = elapsed_ms, "timing");
    }
}
