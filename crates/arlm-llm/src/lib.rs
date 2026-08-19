pub mod anthropic;
pub mod factory;
pub mod gemini;
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
