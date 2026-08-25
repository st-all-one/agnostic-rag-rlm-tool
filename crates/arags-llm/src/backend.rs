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

    #[must_use]
    pub fn completions_url(&self, model: &str) -> String {
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

    #[must_use]
    pub fn auth_headers(&self) -> Vec<(String, String)> {
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
    #[must_use]
    pub fn build_request(self, req: &CompletionRequest) -> Value {
        match self {
            BackendFamily::OpenAi => Self::build_openai(req),
            BackendFamily::Anthropic => Self::build_anthropic(req),
            BackendFamily::Gemini => Self::build_gemini(req),
            BackendFamily::Ollama => Self::build_ollama(req),
        }
    }

    /// Parse a provider response body into a [`CompletionResponse`].
    ///
    /// # Errors
    ///
    /// Returns a family-specific [`LlmError`] when the body cannot be
    /// interpreted (missing fields, unexpected shape).
    pub fn parse_response(self, model: &str, body: &str) -> Result<CompletionResponse, LlmError> {
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
            // Ollama streams NDJSON chunks by default; we parse one body.
            "stream": false,
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
