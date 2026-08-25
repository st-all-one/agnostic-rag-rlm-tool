//! Protocol-level tests for the LLM backend families: request building,
//! response parsing, URL composition and auth header schemes. No network.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arags_llm::config::{AuthScheme, BackendConfig, BackendFamily, HealthMethod};
use arags_llm::trait_llm::LlmBackend;
use arags_llm::types::{CompletionRequest, Message, Role, ToolDefinition};
use arags_llm::{GenericBackend, LlmError};

fn msg(role: Role, content: &str) -> Message {
    Message {
        role,
        content: content.to_string(),
    }
}

fn openai_cfg() -> BackendConfig {
    let mut c = BackendConfig::openai(Some("K".to_string()));
    c.base_url = "https://api.openai.com/v1".to_string();
    c
}

fn req(
    model: &str,
    messages: Vec<Message>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
) -> CompletionRequest {
    CompletionRequest {
        model: model.to_string(),
        messages,
        temperature,
        max_tokens,
        stop: None,
        seed: None,
        tools: None,
    }
}

#[test]
fn test_build_openai() {
    let v = BackendFamily::OpenAi.build_request(&req(
        "gpt-4o",
        vec![msg(Role::User, "hi")],
        Some(0.5),
        Some(10),
    ));
    assert_eq!(v["model"], "gpt-4o");
    assert_eq!(v["messages"][0]["role"], "user");
    assert_eq!(v["temperature"], 0.5);
    assert_eq!(v["max_tokens"], 10);
    // seed is None → absent from payload.
    assert!(v.get("seed").is_none() || v["seed"] == serde_json::Value::Null);
}

#[test]
fn test_build_openai_tools() {
    let tools = Some(vec![ToolDefinition {
        name: "f".to_string(),
        description: "d".to_string(),
        parameters: serde_json::json!({"type": "object"}),
    }]);
    let r = CompletionRequest {
        model: "gpt-4o".to_string(),
        messages: vec![],
        temperature: None,
        max_tokens: None,
        stop: None,
        seed: None,
        tools,
    };
    let v = BackendFamily::OpenAi.build_request(&r);
    assert_eq!(v["tools"][0]["type"], "function");
    assert_eq!(v["tools"][0]["function"]["name"], "f");
}

#[test]
fn test_build_anthropic_splits_system() {
    let r = req(
        "claude",
        vec![msg(Role::System, "sys"), msg(Role::User, "hi")],
        None,
        Some(5),
    );
    let v = BackendFamily::Anthropic.build_request(&r);
    assert_eq!(v["system"], "sys");
    assert_eq!(v["messages"][0]["role"], "user");
    assert_eq!(v["messages"][0]["content"][0]["type"], "text");
    assert_eq!(v["max_tokens"], 5);
}

#[test]
fn test_build_gemini() {
    let r = CompletionRequest {
        model: "gemini-1.5-pro".to_string(),
        messages: vec![
            msg(Role::System, "sys"),
            msg(Role::User, "hi"),
            msg(Role::Assistant, "ok"),
        ],
        temperature: Some(0.2),
        max_tokens: Some(7),
        stop: Some(vec!["x".to_string()]),
        seed: None,
        tools: None,
    };
    let v = BackendFamily::Gemini.build_request(&r);
    assert_eq!(v["systemInstruction"]["parts"][0]["text"], "sys");
    assert_eq!(v["contents"][0]["role"], "user");
    assert_eq!(v["contents"][1]["role"], "model");
    assert_eq!(v["generationConfig"]["maxOutputTokens"], 7);
    assert_eq!(v["generationConfig"]["stopSequences"][0], "x");
}

#[test]
fn test_build_ollama_options() {
    let v = BackendFamily::Ollama.build_request(&req(
        "llama3",
        vec![msg(Role::User, "hi")],
        Some(0.5),
        Some(8),
    ));
    assert_eq!(v["options"]["temperature"], 0.5);
    assert_eq!(v["options"]["num_predict"], 8);
}

#[test]
fn test_parse_openai() {
    let body = r#"{"model":"gpt-4o","choices":[{"message":{"content":"hi"}}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
    let r = BackendFamily::OpenAi
        .parse_response("gpt-4o", body)
        .unwrap();
    assert_eq!(r.content, "hi");
    assert_eq!(r.usage.prompt_tokens, 10);
    assert_eq!(r.usage.completion_tokens, 5);
    assert_eq!(r.usage.total_tokens, 15);
}

#[test]
fn test_parse_anthropic() {
    let body = r#"{"content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":3,"output_tokens":2}}"#;
    let r = BackendFamily::Anthropic
        .parse_response("claude", body)
        .unwrap();
    assert_eq!(r.content, "hello");
    assert_eq!(r.usage.prompt_tokens, 3);
    assert_eq!(r.usage.completion_tokens, 2);
    assert_eq!(r.usage.total_tokens, 5);
}

#[test]
fn test_parse_gemini() {
    let body = r#"{"candidates":[{"content":{"parts":[{"text":"gem"}]}}],"usageMetadata":{"promptTokenCount":4,"candidatesTokenCount":1,"totalTokenCount":5}}"#;
    let r = BackendFamily::Gemini
        .parse_response("gemini", body)
        .unwrap();
    assert_eq!(r.content, "gem");
    assert_eq!(r.usage.prompt_tokens, 4);
    assert_eq!(r.usage.completion_tokens, 1);
    assert_eq!(r.usage.total_tokens, 5);
}

#[test]
fn test_parse_ollama() {
    let body =
        r#"{"message":{"role":"assistant","content":"ol"},"prompt_eval_count":2,"eval_count":3}"#;
    let r = BackendFamily::Ollama
        .parse_response("llama3", body)
        .unwrap();
    assert_eq!(r.content, "ol");
    assert_eq!(r.usage.prompt_tokens, 2);
    assert_eq!(r.usage.completion_tokens, 3);
}

#[test]
fn test_completions_url_gemini_query_auth() {
    let mut c = BackendConfig::gemini(Some("KEY".to_string()));
    c.base_url = "https://gen/v1".to_string();
    let b = GenericBackend::from_config(c).unwrap();
    assert_eq!(
        b.completions_url("gemini-1.5-pro"),
        "https://gen/v1/models/gemini-1.5-pro:generateContent?key=KEY"
    );
}

#[test]
fn test_completions_url_openai() {
    let b = GenericBackend::from_config(openai_cfg()).unwrap();
    assert_eq!(
        b.completions_url("gpt-4o"),
        "https://api.openai.com/v1/chat/completions"
    );
}

#[test]
fn test_auth_headers_bearer() {
    let b = GenericBackend::from_config(openai_cfg()).unwrap();
    let headers = b.auth_headers();
    let h: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert!(h.contains(&("Content-Type", "application/json")));
    assert!(h.contains(&("Authorization", "Bearer K")));
}

#[test]
fn test_auth_headers_header_scheme() {
    let mut c = BackendConfig::anthropic(Some("AK".to_string()));
    c.base_url = "https://api.anthropic.com/v1".to_string();
    let b = GenericBackend::from_config(c).unwrap();
    let headers = b.auth_headers();
    let keys: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
    assert!(keys.contains(&"x-api-key"));
    assert!(!keys.contains(&"Authorization"));
}

#[test]
fn test_auth_headers_none() {
    let b = GenericBackend::from_config(BackendConfig::ollama()).unwrap();
    let headers = b.auth_headers();
    let h: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
    assert!(h.contains(&"Content-Type"));
    assert!(!h.contains(&"Authorization"));
}

#[test]
fn test_from_config_requires_key() {
    let c = BackendConfig::openai(None);
    assert!(matches!(
        GenericBackend::from_config(c),
        Err(LlmError::Auth(_))
    ));
}

#[test]
fn test_name_default_and_custom() {
    let b = GenericBackend::from_config(openai_cfg()).unwrap();
    assert_eq!(LlmBackend::name(&b), "openai");
    let mut c = BackendConfig::openai(Some("K".to_string()));
    c.name = Some("custom".to_string());
    let b = GenericBackend::from_config(c).unwrap();
    assert_eq!(LlmBackend::name(&b), "custom");
}

#[test]
fn test_defaults() {
    assert_eq!(HealthMethod::default(), HealthMethod::Get);
    assert_eq!(AuthScheme::default(), AuthScheme::Bearer);
}
