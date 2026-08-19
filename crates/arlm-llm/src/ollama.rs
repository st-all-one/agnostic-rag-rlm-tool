use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::retry::{RetryConfig, retry_with_backoff};
use crate::trait_llm::LlmBackend;
use crate::types::{CompletionRequest, CompletionResponse, LlmError, UsageSummary};

#[derive(Debug, Clone)]
pub struct OllamaBackend {
    client: Client,
    base_url: String,
    retry_config: RetryConfig,
}

#[derive(Serialize, Clone)]
struct OllamaRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Serialize, Deserialize, Clone)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Serialize, Clone)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct OllamaResponse {
    message: OllamaMessage,
    model: String,
    #[serde(default)]
    prompt_eval_count: u32,
    #[serde(default)]
    eval_count: u32,
}

impl OllamaBackend {
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: "http://localhost:11434".to_string(),
            retry_config: RetryConfig::new(2, 500, 5000),
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

    fn convert_messages(request: &CompletionRequest) -> Vec<OllamaMessage> {
        request
            .messages
            .iter()
            .map(|m| OllamaMessage {
                role: m.role.to_string(),
                content: m.content.clone(),
            })
            .collect()
    }
}

impl Default for OllamaBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmBackend for OllamaBackend {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = request.model.clone();
        let messages = Self::convert_messages(&request);

        let options = OllamaOptions {
            temperature: request.temperature,
            num_predict: request.max_tokens,
            stop: request.stop,
        };

        let ollama_request = OllamaRequest {
            model: model.clone(),
            messages,
            stream: Some(false),
            options: Some(options),
        };

        let url = format!("{}/api/chat", self.base_url);
        let client = self.client.clone();

        let response = retry_with_backoff(&self.retry_config, || {
            let url = url.clone();
            let client = client.clone();
            let body = ollama_request.clone();
            async move {
                let resp = client
                    .post(&url)
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

                if !status.is_success() {
                    return Err(LlmError::Http {
                        status: status.as_u16(),
                        body: body_text,
                    });
                }

                let ollama_response: OllamaResponse = serde_json::from_str(&body_text)
                    .map_err(|e| LlmError::Serialization(e.to_string()))?;

                let total = ollama_response.prompt_eval_count + ollama_response.eval_count;

                Ok(CompletionResponse {
                    content: ollama_response.message.content,
                    model: ollama_response.model,
                    usage: UsageSummary {
                        prompt_tokens: ollama_response.prompt_eval_count,
                        completion_tokens: ollama_response.eval_count,
                        total_tokens: total,
                    },
                })
            }
        })
        .await?;

        info!(model = %model, "ollama completion finished");
        Ok(response)
    }

    fn name(&self) -> &'static str {
        "ollama"
    }

    async fn health_check(&self) -> Result<(), LlmError> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| LlmError::Connection(e.to_string()))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(LlmError::Connection(
                "Ollama server not responding".to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};

    #[test]
    fn test_ollama_backend_new() {
        let backend = OllamaBackend::new();
        assert_eq!(backend.name(), "ollama");
        assert_eq!(backend.base_url, "http://localhost:11434");
    }

    #[test]
    fn test_ollama_backend_with_base_url() {
        let backend = OllamaBackend::new().with_base_url("http://custom:11434".to_string());
        assert_eq!(backend.base_url, "http://custom:11434");
    }

    #[test]
    fn test_ollama_convert_messages() {
        let request = CompletionRequest {
            model: "llama3".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            temperature: None,
            max_tokens: None,
            stop: None,
        };
        let messages = OllamaBackend::convert_messages(&request);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Hello");
    }

    #[test]
    fn test_ollama_request_serialization() {
        let req = OllamaRequest {
            model: "llama3".to_string(),
            messages: vec![OllamaMessage {
                role: "user".to_string(),
                content: "test".to_string(),
            }],
            stream: Some(false),
            options: Some(OllamaOptions {
                temperature: Some(0.7),
                num_predict: Some(100),
                stop: None,
            }),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        assert!(json.contains("llama3"));
        assert!(json.contains("0.7"));
    }

    #[test]
    fn test_ollama_response_deserialization() {
        let json = r#"{
            "message": {"role": "assistant", "content": "Hello!"},
            "model": "llama3",
            "prompt_eval_count": 10,
            "eval_count": 5
        }"#;
        let resp: OllamaResponse =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(resp.message.content, "Hello!");
        assert_eq!(resp.prompt_eval_count, 10);
        assert_eq!(resp.eval_count, 5);
    }
}
