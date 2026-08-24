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

use arags_llm::types::{
    CompletionRequest, CompletionResponse, LlmError, Message, Role, UsageSummary,
};

#[test]
fn test_message_creation() {
    let msg = Message {
        role: Role::User,
        content: "Hello".to_string(),
    };
    assert_eq!(msg.role, Role::User);
    assert_eq!(msg.content, "Hello");
}

#[test]
fn test_role_display() {
    assert_eq!(Role::System.to_string(), "system");
    assert_eq!(Role::User.to_string(), "user");
    assert_eq!(Role::Assistant.to_string(), "assistant");
}

#[test]
fn test_usage_summary_default() {
    let usage = UsageSummary::default();
    assert_eq!(usage.prompt_tokens, 0);
    assert_eq!(usage.completion_tokens, 0);
    assert_eq!(usage.total_tokens, 0);
    assert_eq!(usage.cost_usd, 0.0);
}

#[test]
fn test_completion_request_serialization() {
    let req = CompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![Message {
            role: Role::User,
            content: "test".to_string(),
        }],
        temperature: Some(0.7),
        max_tokens: Some(100),
        stop: None,
        seed: None,
        tools: None,
    };
    let json = serde_json::to_string(&req).expect("serialization should succeed");
    assert!(json.contains("gpt-4"));
    assert!(json.contains("0.7"));
    assert!(!json.contains("seed"));
}

#[test]
fn test_completion_request_with_seed_and_tools() {
    let req = CompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![Message {
            role: Role::User,
            content: "test".to_string(),
        }],
        temperature: None,
        max_tokens: None,
        stop: None,
        seed: Some(42),
        tools: Some(vec![arags_llm::ToolDefinition {
            name: "search".to_string(),
            description: "web search".to_string(),
            parameters: serde_json::json!({ "type": "object" }),
        }]),
    };
    let json = serde_json::to_string(&req).expect("serialization should succeed");
    assert!(json.contains("\"seed\":42"));
    assert!(json.contains("search"));
}

#[test]
fn test_completion_response_deserialization() {
    let json = r#"{
        "content": "Hello!",
        "model": "gpt-4",
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    }"#;
    let resp: CompletionResponse =
        serde_json::from_str(json).expect("deserialization should succeed");
    assert_eq!(resp.content, "Hello!");
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 5);
    assert_eq!(resp.usage.cost_usd, 0.0);
}

#[test]
fn test_llm_error_display() {
    let err = LlmError::Http {
        status: 429,
        body: "rate limited".to_string(),
    };
    assert!(err.to_string().contains("429"));
}
