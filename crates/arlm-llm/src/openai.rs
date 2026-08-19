use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::retry::{RetryConfig, retry_with_backoff};
use crate::trait_llm::LlmBackend;
use crate::types::{CompletionRequest, CompletionResponse, LlmError, UsageSummary};

#[derive(Debug, Clone)]
pub struct OpenAiBackend {
    client: Client,
    api_key: String,
    base_url: String,
    retry_config: RetryConfig,
}

#[derive(Serialize, Clone)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Clone)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    model: String,
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Deserialize)]
#[allow(clippy::struct_field_names)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Deserialize)]
struct OpenAiErrorResponse {
    error: OpenAiError,
}

#[derive(Deserialize)]
struct OpenAiError {
    message: String,
}

impl OpenAiBackend {
    #[must_use]
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
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

    fn convert_messages(request: &CompletionRequest) -> Vec<OpenAiMessage> {
        request
            .messages
            .iter()
            .map(|m| OpenAiMessage {
                role: m.role.to_string(),
                content: m.content.clone(),
            })
            .collect()
    }
}

#[async_trait]
impl LlmBackend for OpenAiBackend {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = request.model.clone();
        let messages = Self::convert_messages(&request);

        let openai_request = OpenAiRequest {
            model: model.clone(),
            messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stop: request.stop,
        };

        let url = format!("{}/chat/completions", self.base_url);
        let api_key = self.api_key.clone();
        let client = self.client.clone();

        let response = retry_with_backoff(&self.retry_config, || {
            let url = url.clone();
            let api_key = api_key.clone();
            let client = client.clone();
            let body = openai_request.clone();
            async move {
                let resp = client
                    .post(&url)
                    .header("Authorization", format!("Bearer {api_key}"))
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
                    let error_msg = serde_json::from_str::<OpenAiErrorResponse>(&body_text)
                        .map_or_else(|_| body_text.clone(), |e| e.error.message);
                    return Err(LlmError::Http {
                        status: status.as_u16(),
                        body: error_msg,
                    });
                }

                let openai_response: OpenAiResponse = serde_json::from_str(&body_text)
                    .map_err(|e| LlmError::Serialization(e.to_string()))?;

                let content = openai_response
                    .choices
                    .first()
                    .map_or_else(String::new, |c| c.message.content.clone());

                let usage = openai_response
                    .usage
                    .map_or_else(UsageSummary::default, |u| UsageSummary {
                        prompt_tokens: u.prompt_tokens,
                        completion_tokens: u.completion_tokens,
                        total_tokens: u.total_tokens,
                    });

                Ok(CompletionResponse {
                    content,
                    model: openai_response.model,
                    usage,
                })
            }
        })
        .await?;

        info!(model = %model, "openai completion finished");
        Ok(response)
    }

    fn name(&self) -> &'static str {
        "openai"
    }

    async fn health_check(&self) -> Result<(), LlmError> {
        let url = format!("{}/models", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| LlmError::Connection(e.to_string()))?;

        if resp.status().is_success() {
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
    fn test_openai_backend_new() {
        let backend = OpenAiBackend::new("test-key".to_string());
        assert_eq!(backend.name(), "openai");
    }

    #[test]
    fn test_openai_backend_with_base_url() {
        let backend = OpenAiBackend::new("test-key".to_string())
            .with_base_url("http://localhost:8080/v1".to_string());
        assert_eq!(backend.base_url, "http://localhost:8080/v1");
    }

    #[test]
    fn test_convert_messages() {
        let request = CompletionRequest {
            model: "gpt-4".to_string(),
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
        let messages = OpenAiBackend::convert_messages(&request);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
    }

    #[test]
    fn test_openai_request_serialization() {
        let req = OpenAiRequest {
            model: "gpt-4".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: "test".to_string(),
            }],
            temperature: Some(0.7),
            max_tokens: Some(100),
            stop: None,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        assert!(json.contains("gpt-4"));
        assert!(json.contains("0.7"));
    }
}
