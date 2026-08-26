use std::sync::Arc;

use async_trait::async_trait;
use tracing::warn;

use crate::trait_llm::LlmBackend;
use crate::types::{CompletionRequest, CompletionResponse, LlmError};

/// Chain two LLM backends: a primary and an optional fallback.
///
/// On a primary failure (or, when health checking is enabled, on an unhealthy
/// primary) the request is forwarded to the fallback. Implements [`LlmBackend`]
/// so it is a drop-in replacement for any single backend — enabling model
/// fallback (e.g. `OpenAI` → `Ollama` local).
#[derive(Clone)]
pub struct ModelFallback {
    primary: Arc<dyn LlmBackend>,
    fallback: Option<Arc<dyn LlmBackend>>,
    check_health: bool,
}

impl ModelFallback {
    #[must_use]
    pub fn new(primary: Arc<dyn LlmBackend>, fallback: Option<Arc<dyn LlmBackend>>) -> Self {
        Self {
            primary,
            fallback,
            check_health: false,
        }
    }

    /// Enable a health check of the primary before each completion; on failure
    /// the request is routed straight to the fallback.
    #[must_use]
    pub fn with_health_check(mut self, enabled: bool) -> Self {
        self.check_health = enabled;
        self
    }
}

#[async_trait]
impl LlmBackend for ModelFallback {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let _timer = crate::Timer::new("fallback_complete");

        if self.check_health {
            if let Err(e) = self.primary.health_check().await {
                warn!(error = %e, "primary backend unhealthy; routing to fallback");
                if let Some(fb) = &self.fallback {
                    return fb.complete(request).await;
                }
                return Err(e);
            }
        }

        match self.primary.complete(request.clone()).await {
            Ok(response) => Ok(response),
            Err(e) => {
                warn!(error = %e, "primary backend failed; trying fallback");
                match &self.fallback {
                    Some(fb) => fb.complete(request).await,
                    None => Err(e),
                }
            }
        }
    }

    fn name(&self) -> &str {
        self.primary.name()
    }

    fn default_model(&self) -> Option<String> {
        self.primary.default_model()
    }

    async fn health_check(&self) -> Result<(), LlmError> {
        self.primary.health_check().await
    }
}
