//! Generic, config-driven LLM backend.
//!
//! [`GenericBackend`] implements [`LlmBackend`] for *any* provider described by
//! a [`BackendConfig`]. Request building and response parsing are dispatched on
//! [`BackendFamily`]. This replaces the previous per-provider backend structs
//! (OpenAiBackend, AnthropicBackend, GeminiBackend, OllamaBackend, DeepSeekBackend,
//! MiMoBackend).

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};

use crate::config::{AuthScheme, BackendConfig, BackendFamily, HealthMethod};
use crate::retry::RetryConfig;
use crate::trait_llm::LlmBackend;
use crate::transport::{extract_json_error_message, request_completion};
use crate::types::{CompletionRequest, CompletionResponse, LlmError, Message, Role, UsageSummary};

/// LLM backend driven entirely by a [`BackendConfig`].
pub struct GenericBackend {
    config: BackendConfig,
    name: String,
    client: Client,
    retry_config: RetryConfig,
}

impl GenericBackend {
    /// Build a backend from a configuration.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::Auth`] if the configured [`AuthScheme`] requires an
    /// API key but `api_key` is absent.
    pub fn from_config(config: BackendConfig) -> Result<Self, LlmError> {
        if config.auth != AuthScheme::None && config.api_key.is_none() {
            return Err(LlmError::Auth(format!(
                "API key required for {} backend (auth = {:?})",
                config.family, config.auth
            )));
        }
        let name = config
            .name
            .clone()
            .unwrap_or_else(|| config.family.as_str().to_string());
        Ok(Self {
            config,
            name,
            client: Client::new(),
            retry_config: RetryConfig::default(),
        })
    }

    #[must_use]
    pub fn with_retry_config(mut self, retry_config: RetryConfig) -> Self {
        self.retry_config = retry_config;
        self
    }

    fn completions_url(&self, model: &str) -> String {
        let path = self.config.completions_path.replace("{model}", model);
        let mut url = format!(
            "{}/{}",
            self.config.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        if self.config.auth == AuthScheme::Query {
            let sep = if url.contains('?') { '&' } else { '?' };
            let key = self.config.api_key.as_deref().unwrap_or("");
            url = format!("{url}{sep}{}={}", self.config.auth_query_param, key);
        }
        url
    }

    fn auth_headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        match self.config.auth {
            AuthScheme::Bearer => {
                if let Some(key) = &self.config.api_key {
                    headers.push((
                        self.config.auth_header.clone(),
                        format!("{} {}", self.config.auth_prefix, key),
                    ));
                }
            }
            AuthScheme::Header => {
                if let Some(key) = &self.config.api_key {
                    headers.push((self.config.auth_header.clone(), key.clone()));
                }
            }
            AuthScheme::Query | AuthScheme::None => {}
        }
        headers.extend(self.config.extra_headers.iter().cloned());
        headers
    }

    fn health_url(&self) -> String {
        format!(
            "{}/{}",
            self.config.base_url.trim_end_matches('/'),
            self.config.health_path.trim_start_matches('/')
        )
    }
}

#[async_trait]
impl LlmBackend for GenericBackend {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = request.model.clone();
        let payload = self.config.family.build_request(&request);
        let url = self.completions_url(&model);
        let headers = self.auth_headers();
        let family = self.config.family;
        request_completion(
            &self.client,
            &url,
            &headers,
            &payload,
            &self.retry_config,
            move |_, body| family.error_message(body),
            move |body| family.parse_response(&model, body),
        )
        .await
    }

    fn name(&self) -> &str {
        &self.name
    }

    async fn health_check(&self) -> Result<(), LlmError> {
        let mut url = self.health_url();
        if self.config.auth == AuthScheme::Query {
            let sep = if url.contains('?') { '&' } else { '?' };
            let key = self.config.api_key.as_deref().unwrap_or("");
            url = format!("{url}{sep}{}={}", self.config.auth_query_param, key);
        }
        let mut builder = match self.config.health_method {
            HealthMethod::Get => self.client.get(&url),
            HealthMethod::Post => self.client.post(&url),
        };
        for (k, v) in self.auth_headers() {
            builder = builder.header(k, v);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| LlmError::Connection(e.to_string()))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(LlmError::Backend(format!(
                "health check failed: status {}",
                resp.status()
            )))
        }
    }
}

impl BackendFamily {
    pub(crate) fn build_request(self, req: &CompletionRequest) -> Value {
        match self {
            BackendFamily::OpenAi => Self::build_openai(req),
            BackendFamily::Anthropic => Self::build_anthropic(req),
            BackendFamily::Gemini => Self::build_gemini(req),
            BackendFamily::Ollama => Self::build_ollama(req),
        }
    }

    pub(crate) fn parse_response(
        self,
        model: &str,
        body: &str,
    ) -> Result<CompletionResponse, LlmError> {
        match self {
            BackendFamily::OpenAi => Self::parse_openai(model, body),
            BackendFamily::Anthropic => Self::parse_anthropic(model, body),
            BackendFamily::Gemini => Self::parse_gemini(model, body),
            BackendFamily::Ollama => Self::parse_ollama(model, body),
        }
    }

    pub(crate) fn error_message(self, body: &str) -> String {
        match self {
            BackendFamily::Ollama => body.to_string(),
            _ => extract_json_error_message(body),
        }
    }

    fn build_openai(req: &CompletionRequest) -> Value {
        let mut v = json!({
            "model": req.model,
            "messages": messages_simple(&req.messages),
        });
        if let Some(t) = req.temperature {
            v["temperature"] = json!(t);
        }
        if let Some(m) = req.max_tokens {
            v["max_tokens"] = json!(m);
        }
        if let Some(s) = &req.stop {
            v["stop"] = json!(s);
        }
        if let Some(s) = req.seed {
            v["seed"] = json!(s);
        }
        if let Some(tools) = &req.tools {
            let tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }
                    })
                })
                .collect();
            v["tools"] = json!(tools);
        }
        v
    }

    fn build_anthropic(req: &CompletionRequest) -> Value {
        let (system, rest) = split_system(&req.messages);
        let mut v = json!({
            "model": req.model,
            "messages": anthropic_messages(&rest),
        });
        if let Some(sys) = system {
            v["system"] = json!(sys);
        }
        if let Some(m) = req.max_tokens {
            v["max_tokens"] = json!(m);
        }
        if let Some(t) = req.temperature {
            v["temperature"] = json!(t);
        }
        if let Some(s) = &req.stop {
            v["stop_sequences"] = json!(s);
        }
        if let Some(tools) = &req.tools {
            let tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.parameters,
                    })
                })
                .collect();
            v["tools"] = json!(tools);
        }
        v
    }

    fn build_gemini(req: &CompletionRequest) -> Value {
        let (system, rest) = split_system(&req.messages);
        let mut v = json!({
            "contents": gemini_contents(&rest),
        });
        if let Some(sys) = system {
            v["systemInstruction"] = json!({ "parts": [{ "text": sys }] });
        }
        let mut gen_cfg = json!({});
        if let Some(t) = req.temperature {
            gen_cfg["temperature"] = json!(t);
        }
        if let Some(m) = req.max_tokens {
            gen_cfg["maxOutputTokens"] = json!(m);
        }
        if let Some(s) = &req.stop {
            gen_cfg["stopSequences"] = json!(s);
        }
        if gen_cfg.as_object().is_some_and(|o| !o.is_empty()) {
            v["generationConfig"] = gen_cfg;
        }
        if let Some(tools) = &req.tools {
            let decls: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    })
                })
                .collect();
            v["tools"] = json!({ "functionDeclarations": decls });
        }
        v
    }

    fn build_ollama(req: &CompletionRequest) -> Value {
        let mut v = json!({
            "model": req.model,
            "messages": messages_simple(&req.messages),
        });
        let mut opts = json!({});
        if let Some(t) = req.temperature {
            opts["temperature"] = json!(t);
        }
        if let Some(m) = req.max_tokens {
            opts["num_predict"] = json!(m);
        }
        if opts.as_object().is_some_and(|o| !o.is_empty()) {
            v["options"] = opts;
        }
        v
    }

    fn parse_openai(model: &str, body: &str) -> Result<CompletionResponse, LlmError> {
        let v: Value =
            serde_json::from_str(body).map_err(|e| LlmError::Serialization(e.to_string()))?;
        let content = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Ok(CompletionResponse {
            content,
            model: model.to_string(),
            usage: usage_from(&v["usage"]),
        })
    }

    fn parse_anthropic(model: &str, body: &str) -> Result<CompletionResponse, LlmError> {
        let v: Value =
            serde_json::from_str(body).map_err(|e| LlmError::Serialization(e.to_string()))?;
        let mut content = String::new();
        if let Some(arr) = v["content"].as_array() {
            for block in arr {
                if block["type"] == "text" {
                    if let Some(t) = block["text"].as_str() {
                        content.push_str(t);
                    }
                }
            }
        }
        let prompt = v["usage"]["input_tokens"].as_u64().unwrap_or(0);
        let completion = v["usage"]["output_tokens"].as_u64().unwrap_or(0);
        Ok(CompletionResponse {
            content,
            model: model.to_string(),
            usage: UsageSummary {
                prompt_tokens: u32::try_from(prompt).unwrap_or(u32::MAX),
                completion_tokens: u32::try_from(completion).unwrap_or(u32::MAX),
                total_tokens: u32::try_from(prompt + completion).unwrap_or(u32::MAX),
                cost_usd: 0.0,
            },
        })
    }

    fn parse_gemini(model: &str, body: &str) -> Result<CompletionResponse, LlmError> {
        let v: Value =
            serde_json::from_str(body).map_err(|e| LlmError::Serialization(e.to_string()))?;
        let mut content = String::new();
        if let Some(cands) = v["candidates"].as_array() {
            for c in cands {
                if let Some(parts) = c["content"]["parts"].as_array() {
                    for p in parts {
                        if let Some(t) = p["text"].as_str() {
                            content.push_str(t);
                        }
                    }
                }
            }
        }
        let prompt = v["usageMetadata"]["promptTokenCount"].as_u64().unwrap_or(0);
        let completion = v["usageMetadata"]["candidatesTokenCount"]
            .as_u64()
            .unwrap_or(0);
        let total = v["usageMetadata"]["totalTokenCount"]
            .as_u64()
            .unwrap_or(prompt + completion);
        Ok(CompletionResponse {
            content,
            model: model.to_string(),
            usage: UsageSummary {
                prompt_tokens: u32::try_from(prompt).unwrap_or(u32::MAX),
                completion_tokens: u32::try_from(completion).unwrap_or(u32::MAX),
                total_tokens: u32::try_from(total).unwrap_or(u32::MAX),
                cost_usd: 0.0,
            },
        })
    }

    fn parse_ollama(model: &str, body: &str) -> Result<CompletionResponse, LlmError> {
        let v: Value =
            serde_json::from_str(body).map_err(|e| LlmError::Serialization(e.to_string()))?;
        let content = v["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let prompt = v["prompt_eval_count"].as_u64().unwrap_or(0);
        let completion = v["eval_count"].as_u64().unwrap_or(0);
        Ok(CompletionResponse {
            content,
            model: if model.is_empty() {
                "ollama".to_string()
            } else {
                model.to_string()
            },
            usage: UsageSummary {
                prompt_tokens: u32::try_from(prompt).unwrap_or(u32::MAX),
                completion_tokens: u32::try_from(completion).unwrap_or(u32::MAX),
                total_tokens: u32::try_from(prompt + completion).unwrap_or(u32::MAX),
                cost_usd: 0.0,
            },
        })
    }
}

fn messages_simple(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            json!({
                "role": m.role.to_string(),
                "content": m.content,
            })
        })
        .collect()
}

fn anthropic_messages(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            json!({
                "role": m.role.to_string(),
                "content": [{ "type": "text", "text": m.content }],
            })
        })
        .collect()
}

fn gemini_contents(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::User | Role::System => "user",
                Role::Assistant => "model",
            };
            json!({ "role": role, "parts": [{ "text": m.content }] })
        })
        .collect()
}

fn split_system(messages: &[Message]) -> (Option<String>, Vec<Message>) {
    let mut system: Option<String> = None;
    let mut rest = Vec::new();
    for m in messages {
        if m.role == Role::System {
            system = Some(m.content.clone());
        } else {
            rest.push(m.clone());
        }
    }
    (system, rest)
}

fn usage_from(v: &Value) -> UsageSummary {
    let prompt = v["prompt_tokens"].as_u64().unwrap_or(0);
    let completion = v["completion_tokens"].as_u64().unwrap_or(0);
    let total = v["total_tokens"].as_u64().unwrap_or(prompt + completion);
    UsageSummary {
        prompt_tokens: u32::try_from(prompt).unwrap_or(u32::MAX),
        completion_tokens: u32::try_from(completion).unwrap_or(u32::MAX),
        total_tokens: u32::try_from(total).unwrap_or(u32::MAX),
        cost_usd: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuthScheme, BackendConfig, BackendFamily, HealthMethod};

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

    #[test]
    fn test_build_openai() {
        let req = CompletionRequest {
            model: "gpt-4o".to_string(),
            messages: vec![msg(Role::User, "hi")],
            temperature: Some(0.5),
            max_tokens: Some(10),
            stop: None,
            seed: Some(1),
            tools: None,
        };
        let v = BackendFamily::OpenAi.build_request(&req);
        assert_eq!(v["model"], "gpt-4o");
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["temperature"], 0.5);
        assert_eq!(v["max_tokens"], 10);
        assert_eq!(v["seed"], 1);
    }

    #[test]
    fn test_build_openai_tools() {
        let tools = Some(vec![crate::types::ToolDefinition {
            name: "f".to_string(),
            description: "d".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }]);
        let req = CompletionRequest {
            model: "gpt-4o".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            stop: None,
            seed: None,
            tools,
        };
        let v = BackendFamily::OpenAi.build_request(&req);
        assert_eq!(v["tools"][0]["type"], "function");
        assert_eq!(v["tools"][0]["function"]["name"], "f");
    }

    #[test]
    fn test_build_anthropic_splits_system() {
        let req = CompletionRequest {
            model: "claude".to_string(),
            messages: vec![msg(Role::System, "sys"), msg(Role::User, "hi")],
            temperature: None,
            max_tokens: Some(5),
            stop: None,
            seed: None,
            tools: None,
        };
        let v = BackendFamily::Anthropic.build_request(&req);
        assert_eq!(v["system"], "sys");
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"][0]["type"], "text");
        assert_eq!(v["max_tokens"], 5);
    }

    #[test]
    fn test_build_gemini() {
        let req = CompletionRequest {
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
        let v = BackendFamily::Gemini.build_request(&req);
        assert_eq!(v["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(v["contents"][0]["role"], "user");
        assert_eq!(v["contents"][1]["role"], "model");
        assert_eq!(v["generationConfig"]["maxOutputTokens"], 7);
        assert_eq!(v["generationConfig"]["stopSequences"][0], "x");
    }

    #[test]
    fn test_build_ollama_options() {
        let req = CompletionRequest {
            model: "llama3".to_string(),
            messages: vec![msg(Role::User, "hi")],
            temperature: Some(0.5),
            max_tokens: Some(8),
            stop: None,
            seed: None,
            tools: None,
        };
        let v = BackendFamily::Ollama.build_request(&req);
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
        let body = r#"{"message":{"role":"assistant","content":"ol"},"prompt_eval_count":2,"eval_count":3}"#;
        let r = BackendFamily::Ollama
            .parse_response("llama3", body)
            .unwrap();
        assert_eq!(r.content, "ol");
        assert_eq!(r.usage.prompt_tokens, 2);
        assert_eq!(r.usage.completion_tokens, 3);
    }

    #[test]
    fn test_completions_url_gemini_query() {
        let mut c = BackendConfig::gemini(Some("KEY".to_string()));
        c.base_url = "https://gen/v1".to_string();
        let b = GenericBackend::from_config(c).unwrap();
        let url = b.completions_url("gemini-1.5-pro");
        assert_eq!(
            url,
            "https://gen/v1/models/gemini-1.5-pro:generateContent?key=KEY"
        );
    }

    #[test]
    fn test_completions_url_openai() {
        let b = GenericBackend::from_config(openai_cfg()).unwrap();
        let url = b.completions_url("gpt-4o");
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
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
        assert!(GenericBackend::from_config(c).is_err());
    }

    #[test]
    fn test_name_default() {
        let b = GenericBackend::from_config(openai_cfg()).unwrap();
        assert_eq!(b.name(), "openai");
        let mut c = BackendConfig::openai(Some("K".to_string()));
        c.name = Some("custom".to_string());
        let b = GenericBackend::from_config(c).unwrap();
        assert_eq!(b.name(), "custom");
    }

    #[test]
    fn test_defaults() {
        assert_eq!(HealthMethod::default(), HealthMethod::Get);
        assert_eq!(AuthScheme::default(), AuthScheme::Bearer);
    }
}
