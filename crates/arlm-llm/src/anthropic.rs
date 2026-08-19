use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::retry::{RetryConfig, retry_with_backoff};
use crate::trait_llm::LlmBackend;
use crate::types::{CompletionRequest, CompletionResponse, LlmError, UsageSummary};

#[derive(Debug, Clone)]
pub struct AnthropicBackend {
    client: Client,
    api_key: String,
    base_url: String,
    retry_config: RetryConfig,
}

#[derive(Serialize, Clone)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Clone)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
    model: String,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Deserialize)]
struct AnthropicErrorResponse {
    error: AnthropicError,
}

#[derive(Deserialize)]
struct AnthropicError {
    message: String,
}

impl AnthropicBackend {
    #[must_use]
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://api.anthropic.com/v1".to_string(),
            retry_config: RetryConfig::default(),
        }
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }

    #[must_use]
    pub fn with_retry_config(mut self, retry_config: RetryConfig) -> Self {
        self.retry_config = retry_config;
        self
    }

    fn convert_request(request: &CompletionRequest) -> (Option<String>, Vec<AnthropicMessage>) {
        let mut system = None;
        let mut messages = Vec::new();

        for msg in &request.messages {
            match msg.role {
                crate::types::Role::System => {
                    system = Some(msg.content.clone());
                }
                crate::types::Role::User => {
                    messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: msg.content.clone(),
                    });
                }
                crate::types::Role::Assistant => {
                    messages.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: msg.content.clone(),
                    });
                }
            }
        }

        (system, messages)
    }
}

#[async_trait]
impl LlmBackend for AnthropicBackend {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = request.model.clone();
        let (system, messages) = Self::convert_request(&request);

        let anthropic_request = AnthropicRequest {
            model: model.clone(),
            messages,
            max_tokens: request.max_tokens.unwrap_or(4096),
            system,
            temperature: request.temperature,
            stop_sequences: request.stop,
        };

        let url = format!("{}/messages", self.base_url);
        let api_key = self.api_key.clone();
        let client = self.client.clone();

        let response = retry_with_backoff(&self.retry_config, || {
            let url = url.clone();
            let api_key = api_key.clone();
            let client = client.clone();
            let body = anthropic_request.clone();
            async move {
                let resp = client
                    .post(&url)
                    .header("x-api-key", &api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| LlmError::Connection(e.to_string()))?;

                let status = resp.status();
                let body_text = resp
                    .text()
                    .await
                    .map_err(|e| LlmError::Connection(e.to_string()))?;

                if status == 429 {
                    return Err(LlmError::RateLimited {
                        retry_after_ms: 1000,
                    });
                }

                if !status.is_success() {
                    let error_msg = serde_json::from_str::<AnthropicErrorResponse>(&body_text)
                        .map_or_else(|_| body_text.clone(), |e| e.error.message);
                    return Err(LlmError::Http {
                        status: status.as_u16(),
                        body: error_msg,
                    });
                }

                let anthropic_response: AnthropicResponse = serde_json::from_str(&body_text)
                    .map_err(|e| LlmError::Serialization(e.to_string()))?;

                let content = anthropic_response
                    .content
                    .first()
                    .map_or_else(String::new, |c| c.text.clone());

                let usage = anthropic_response
                    .usage
                    .map_or_else(UsageSummary::default, |u| UsageSummary {
                        prompt_tokens: u.input_tokens,
                        completion_tokens: u.output_tokens,
                        total_tokens: u.input_tokens + u.output_tokens,
                    });

                Ok(CompletionResponse {
                    content,
                    model: anthropic_response.model,
                    usage,
                })
            }
        })
        .await?;

        info!(model = %model, "anthropic completion finished");
        Ok(response)
    }

    fn name(&self) -> &'static str {
        "anthropic"
    }

    async fn health_check(&self) -> Result<(), LlmError> {
        let test_request = AnthropicRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: "Hi".to_string(),
            }],
            max_tokens: 10,
            system: None,
            temperature: None,
            stop_sequences: None,
        };

        let url = format!("{}/messages", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&test_request)
            .send()
            .await
            .map_err(|e| LlmError::Connection(e.to_string()))?;

        if resp.status().is_success() || resp.status().as_u16() == 400 {
            Ok(())
        } else {
            Err(LlmError::Auth("API key invalid".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};

    #[test]
    fn test_anthropic_backend_new() {
        let backend = AnthropicBackend::new("test-key".to_string());
        assert_eq!(backend.name(), "anthropic");
    }

    #[test]
    fn test_anthropic_backend_with_base_url() {
        let backend = AnthropicBackend::new("test-key".to_string())
            .with_base_url("http://localhost:8080/v1".to_string());
        assert_eq!(backend.base_url, "http://localhost:8080/v1");
    }

    #[test]
    fn test_convert_request_with_system() {
        let request = CompletionRequest {
            model: "claude-3".to_string(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: "You are helpful".to_string(),
                },
                Message {
                    role: Role::User,
                    content: "Hello".to_string(),
                },
            ],
            temperature: None,
            max_tokens: None,
            stop: None,
        };
        let (system, messages) = AnthropicBackend::convert_request(&request);
        assert_eq!(system, Some("You are helpful".to_string()));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
    }

    #[test]
    fn test_convert_request_without_system() {
        let request = CompletionRequest {
            model: "claude-3".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            temperature: None,
            max_tokens: None,
            stop: None,
        };
        let (system, messages) = AnthropicBackend::convert_request(&request);
        assert!(system.is_none());
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_anthropic_request_serialization() {
        let req = AnthropicRequest {
            model: "claude-3".to_string(),
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: "test".to_string(),
            }],
            max_tokens: 1024,
            system: Some("system prompt".to_string()),
            temperature: Some(0.7),
            stop_sequences: None,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        assert!(json.contains("claude-3"));
        assert!(json.contains("system prompt"));
    }
}
