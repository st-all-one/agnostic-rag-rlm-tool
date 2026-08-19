use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::retry::{RetryConfig, retry_with_backoff};
use crate::trait_llm::LlmBackend;
use crate::types::{CompletionRequest, CompletionResponse, LlmError, UsageSummary};

#[derive(Debug, Clone)]
pub struct GeminiBackend {
    client: Client,
    api_key: String,
    base_url: String,
    retry_config: RetryConfig,
}

#[derive(Serialize, Clone)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Serialize, Deserialize, Clone)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
    role: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct GeminiPart {
    text: String,
}

#[derive(Serialize, Clone)]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
    usage_metadata: Option<GeminiUsage>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiContent,
}

#[derive(Deserialize)]
#[allow(clippy::struct_field_names)]
struct GeminiUsage {
    prompt_token_count: u32,
    candidates_token_count: u32,
    total_token_count: u32,
}

#[derive(Deserialize)]
struct GeminiErrorResponse {
    error: GeminiError,
}

#[derive(Deserialize)]
struct GeminiError {
    message: String,
}

impl GeminiBackend {
    #[must_use]
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
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

    fn convert_messages(request: &CompletionRequest) -> Vec<GeminiContent> {
        let mut contents = Vec::new();
        let mut system_instruction = None;

        for msg in &request.messages {
            match msg.role {
                crate::types::Role::System => {
                    system_instruction = Some(msg.content.clone());
                }
                crate::types::Role::User => {
                    contents.push(GeminiContent {
                        parts: vec![GeminiPart {
                            text: msg.content.clone(),
                        }],
                        role: "user".to_string(),
                    });
                }
                crate::types::Role::Assistant => {
                    contents.push(GeminiContent {
                        parts: vec![GeminiPart {
                            text: msg.content.clone(),
                        }],
                        role: "model".to_string(),
                    });
                }
            }
        }

        if let Some(sys) = system_instruction {
            contents.insert(
                0,
                GeminiContent {
                    parts: vec![GeminiPart { text: sys }],
                    role: "user".to_string(),
                },
            );
        }

        contents
    }
}

#[async_trait]
impl LlmBackend for GeminiBackend {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = request.model.clone();
        let contents = Self::convert_messages(&request);

        let generation_config = GeminiGenerationConfig {
            temperature: request.temperature,
            max_output_tokens: request.max_tokens,
            stop_sequences: request.stop,
        };

        let gemini_request = GeminiRequest {
            contents,
            generation_config: Some(generation_config),
        };

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, model, self.api_key
        );
        let client = self.client.clone();

        let response = retry_with_backoff(&self.retry_config, || {
            let url = url.clone();
            let client = client.clone();
            let body = gemini_request.clone();
            let model_name = model.clone();
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

                if status == 429 {
                    return Err(LlmError::RateLimited {
                        retry_after_ms: 1000,
                    });
                }

                if !status.is_success() {
                    let error_msg = serde_json::from_str::<GeminiErrorResponse>(&body_text)
                        .map_or_else(|_| body_text.clone(), |e| e.error.message);
                    return Err(LlmError::Http {
                        status: status.as_u16(),
                        body: error_msg,
                    });
                }

                let gemini_response: GeminiResponse = serde_json::from_str(&body_text)
                    .map_err(|e| LlmError::Serialization(e.to_string()))?;

                let content = gemini_response
                    .candidates
                    .first()
                    .and_then(|c| c.content.parts.first())
                    .map_or_else(String::new, |p| p.text.clone());

                let usage =
                    gemini_response
                        .usage_metadata
                        .map_or_else(UsageSummary::default, |u| UsageSummary {
                            prompt_tokens: u.prompt_token_count,
                            completion_tokens: u.candidates_token_count,
                            total_tokens: u.total_token_count,
                        });

                Ok(CompletionResponse {
                    content,
                    model: model_name,
                    usage,
                })
            }
        })
        .await?;

        info!(model = %model, "gemini completion finished");
        Ok(response)
    }

    fn name(&self) -> &'static str {
        "gemini"
    }

    async fn health_check(&self) -> Result<(), LlmError> {
        let url = format!("{}/models?key={}", self.base_url, self.api_key);
        let resp = self
            .client
            .get(&url)
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
    fn test_gemini_backend_new() {
        let backend = GeminiBackend::new("test-key".to_string());
        assert_eq!(backend.name(), "gemini");
    }

    #[test]
    fn test_gemini_backend_with_base_url() {
        let backend = GeminiBackend::new("test-key".to_string())
            .with_base_url("http://localhost:8080".to_string());
        assert_eq!(backend.base_url, "http://localhost:8080");
    }

    #[test]
    fn test_gemini_convert_messages() {
        let request = CompletionRequest {
            model: "gemini-1.5-pro".to_string(),
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
        let contents = GeminiBackend::convert_messages(&request);
        assert_eq!(contents.len(), 2);
        assert_eq!(contents[0].role, "user"); // system becomes user
        assert_eq!(contents[1].role, "user");
    }

    #[test]
    fn test_gemini_request_serialization() {
        let req = GeminiRequest {
            contents: vec![GeminiContent {
                parts: vec![GeminiPart {
                    text: "test".to_string(),
                }],
                role: "user".to_string(),
            }],
            generation_config: Some(GeminiGenerationConfig {
                temperature: Some(0.7),
                max_output_tokens: Some(100),
                stop_sequences: None,
            }),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        assert!(json.contains("test"));
        assert!(json.contains("0.7"));
    }

    #[test]
    fn test_gemini_response_deserialization() {
        let json = r#"{
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello!"}],
                    "role": "model"
                }
            }],
            "usage_metadata": {
                "prompt_token_count": 10,
                "candidates_token_count": 5,
                "total_token_count": 15
            }
        }"#;
        let resp: GeminiResponse =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(resp.candidates[0].content.parts[0].text, "Hello!");
        assert_eq!(resp.usage_metadata.unwrap().prompt_token_count, 10);
    }
}
