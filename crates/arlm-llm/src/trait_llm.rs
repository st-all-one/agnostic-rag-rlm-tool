use async_trait::async_trait;

use crate::types::{CompletionRequest, CompletionResponse, LlmError};

#[async_trait]
pub trait LlmBackend: Send + Sync {
    /// Send a completion request to the LLM backend.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError>;

    /// Return the backend name for logging/metrics.
    fn name(&self) -> &str;

    /// Check if the backend is available (e.g., API key set, server running).
    async fn health_check(&self) -> Result<(), LlmError>;
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::unnecessary_literal_bound
)]
mod tests {
    use super::*;
    use crate::types::{Message, Role, UsageSummary};

    struct MockBackend;

    #[async_trait]
    impl LlmBackend for MockBackend {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: format!("Mock response to: {}", request.messages[0].content),
                model: request.model,
                usage: UsageSummary {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                },
            })
        }

        fn name(&self) -> &str {
            "mock"
        }

        async fn health_check(&self) -> Result<(), LlmError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_mock_backend_complete() {
        let backend = MockBackend;
        let req = CompletionRequest {
            model: "test".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: "hello".to_string(),
            }],
            temperature: None,
            max_tokens: None,
            stop: None,
        };
        let resp = backend
            .complete(req)
            .await
            .expect("complete should succeed");
        assert!(resp.content.contains("hello"));
        assert_eq!(resp.usage.total_tokens, 15);
    }

    #[tokio::test]
    async fn test_mock_backend_health_check() {
        let backend = MockBackend;
        backend
            .health_check()
            .await
            .expect("health check should succeed");
    }

    #[tokio::test]
    async fn test_mock_backend_name() {
        let backend = MockBackend;
        assert_eq!(backend.name(), "mock");
    }
}
