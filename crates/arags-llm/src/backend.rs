//! Generic, config-driven LLM backend.
//!
//! [`GenericBackend`] implements [`LlmBackend`] for *any* provider described by
//! a [`BackendConfig`]. Request building and response parsing are dispatched on
//! [`BackendFamily`]. This replaces the previous per-provider backend structs
//! (OpenAiBackend, AnthropicBackend, GeminiBackend, OllamaBackend, DeepSeekBackend,
//! MiMoBackend).

use async_trait::async_trait;
use reqwest::Client;

use crate::config::{AuthScheme, BackendConfig, HealthMethod};
use crate::retry::RetryConfig;
use crate::trait_llm::LlmBackend;
use crate::transport::request_completion;
use crate::types::{CompletionRequest, CompletionResponse, LlmError};

/// Family-specific request builders and response parsers live in the `family`
/// submodule so this file stays focused on the transport-agnostic backend.
pub(crate) mod family;

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
        // Honour a caller-supplied model, but fall back to the backend's
        // configured default (e.g. `[[llm.backends]].model`) when the request
        // leaves it empty — otherwise providers like Ollama 404 on a bare
        // `model ''` / hardcoded family default.
        let resolved_model = if request.model.trim().is_empty() {
            self.config.model.clone().unwrap_or_default()
        } else {
            request.model.clone()
        };
        let mut request = request;
        request.model = resolved_model.clone();
        let payload = self.config.family.build_request(&request);
        let url = self.completions_url(&resolved_model);
        let headers = self.auth_headers();
        let family = self.config.family;
        request_completion(
            &self.client,
            &url,
            &headers,
            &payload,
            &self.retry_config,
            move |_, body| family.error_message(body),
            move |body| family.parse_response(&resolved_model, body),
        )
        .await
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn default_model(&self) -> Option<String> {
        self.config.model.clone()
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
