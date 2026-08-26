use async_trait::async_trait;

use crate::types::{CompletionRequest, CompletionResponse, LlmError};

#[async_trait]
pub trait LlmBackend: Send + Sync {
    /// Send a completion request to the LLM backend.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError>;

    /// Return the backend name for logging/metrics.
    fn name(&self) -> &str;

    /// Model configured for this backend, used as the default when a completion
    /// request does not specify one. `None` if the backend has no configured
    /// default (callers then fall back to a family-specific literal).
    fn default_model(&self) -> Option<String>;

    /// Check if the backend is available (e.g., API key set, server running).
    async fn health_check(&self) -> Result<(), LlmError>;
}
