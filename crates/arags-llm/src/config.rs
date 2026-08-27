//! Backend configuration for the generic, provider-agnostic LLM backend.
//!
//! A single [`BackendConfig`] fully describes how to talk to an LLM provider:
//! which wire [`BackendFamily`] to use for request/response mapping, the base
//! URL, the authentication scheme, and the endpoint paths. This replaces the
//! previous per-provider implementations (OpenAI, Anthropic, Gemini, Ollama,
//! DeepSeek, MiMo), which are now *presets* over this config.
//!
//! Because [`BackendConfig`] is fully deserializable (TOML/JSON), adding a new
//! provider or model requires only a new configuration entry — no code changes.

use serde::{Deserialize, Serialize};
use std::fmt;

pub(crate) mod llm_config;
pub(crate) mod presets;

pub use llm_config::LlmConfig;

/// Protocol family controlling how a [`CompletionRequest`] is mapped to/from
/// the provider wire format.
///
/// DeepSeek and MiMo speak the OpenAI protocol, so they use [`Self::OpenAi`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendFamily {
    OpenAi,
    Anthropic,
    Gemini,
    Ollama,
}

impl BackendFamily {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Ollama => "ollama",
        }
    }
}

impl fmt::Display for BackendFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How the API key (if any) is presented to the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthScheme {
    /// `Authorization: Bearer <key>` (the default for most providers).
    #[default]
    Bearer,
    /// Custom header, e.g. `x-api-key: <key>` (Anthropic).
    Header,
    /// Query parameter, e.g. `?key=<key>` (Gemini).
    Query,
    /// No authentication (Ollama).
    None,
}

/// HTTP method used for the health-check endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthMethod {
    /// `GET` request (the default).
    #[default]
    Get,
    /// `POST` request with an empty body (Anthropic).
    Post,
}

fn default_api_base() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_completions_path() -> String {
    "chat/completions".to_string()
}

fn default_auth_header() -> String {
    "Authorization".to_string()
}

fn default_auth_prefix() -> String {
    "Bearer".to_string()
}

fn default_auth_query_param() -> String {
    "key".to_string()
}

fn default_health_path() -> String {
    "models".to_string()
}

/// Full, provider-agnostic description of an LLM backend.
///
/// This struct is the single source of truth that drives [`crate::backend::GenericBackend`].
/// It is fully deserializable from `config.toml` (or JSON).
///
/// # Example (TOML)
///
/// ```toml
/// [[backends]]
/// name = "my-openai"
/// family = "openai"
/// api_key = "sk-..."            # or injected from secrets / env
/// base_url = "https://api.openai.com/v1"
/// model = "gpt-4o"
/// completions_path = "chat/completions"
/// auth = "bearer"
/// auth_prefix = "Bearer"
/// health_path = "models"
/// health_method = "get"
///
/// [[backends]]
/// name = "my-anthropic"
/// family = "anthropic"
/// api_key = "..."
/// auth = "header"
/// auth_header = "x-api-key"
/// extra_headers = [["anthropic-version", "2023-06-01"]]
/// completions_path = "messages"
/// health_path = "messages"
/// health_method = "post"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    /// Protocol family controlling request/response mapping.
    pub family: BackendFamily,
    /// Base URL of the provider API (trailing slash is normalized away).
    #[serde(default = "default_api_base")]
    pub base_url: String,
    /// Default model to use when a request does not specify one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// API key. Required unless [`auth`](Self::auth) is [`AuthScheme::None`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Path (relative to `base_url`) for completion requests.
    /// May contain a `{model}` placeholder (e.g. Gemini).
    #[serde(default = "default_completions_path")]
    pub completions_path: String,
    /// Authentication scheme.
    #[serde(default)]
    pub auth: AuthScheme,
    /// Header name used for [`AuthScheme::Header`] authentication.
    #[serde(default = "default_auth_header")]
    pub auth_header: String,
    /// Prefix used for [`AuthScheme::Bearer`] authentication (e.g. `Bearer`).
    #[serde(default = "default_auth_prefix")]
    pub auth_prefix: String,
    /// Query parameter name used for [`AuthScheme::Query`] authentication.
    #[serde(default = "default_auth_query_param")]
    pub auth_query_param: String,
    /// Extra static headers sent with every request (e.g. API version pins).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_headers: Vec<(String, String)>,
    /// Path used for the health check.
    #[serde(default = "default_health_path")]
    pub health_path: String,
    /// HTTP method for the health check.
    #[serde(default)]
    pub health_method: HealthMethod,
    /// Logical name for logs/metrics. Defaults to `family` if unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}
