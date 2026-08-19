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
pub mod anthropic;
pub mod deepseek;
pub mod factory;
pub mod gemini;
pub mod mimo;
pub mod ollama;
pub mod openai;
pub mod pricing;
pub mod retry;
pub mod trait_llm;
pub mod types;

pub use factory::{BackendKind, get_backend};
pub use pricing::{ModelPricing, PricingTable};
pub use retry::RetryConfig;
pub use trait_llm::LlmBackend;
pub use types::{CompletionRequest, CompletionResponse, LlmError, Message, Role, UsageSummary};

#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
