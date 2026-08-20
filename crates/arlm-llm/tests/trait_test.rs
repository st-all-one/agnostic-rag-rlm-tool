#![allow(
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
)]

use arlm_llm::trait_llm::LlmBackend;
use arlm_llm::types::{
    CompletionRequest, CompletionResponse, LlmError, Message, Role, UsageSummary,
};

struct MockBackend;

#[async_trait::async_trait]
impl LlmBackend for MockBackend {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: format!("Mock response to: {}", request.messages[0].content),
            model: request.model,
            usage: UsageSummary {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
                cost_usd: 0.0,
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
        seed: None,
        tools: None,
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
