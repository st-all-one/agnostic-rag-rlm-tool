//! Provider presets and factory helpers for [`BackendConfig`].
//!
//! Each preset returns a fully-populated [`BackendConfig`]; [`BackendConfig::from_kind`]
//! maps the legacy [`BackendKind`] enum onto the matching preset.

use crate::config::{AuthScheme, BackendConfig, BackendFamily, HealthMethod};
use crate::factory::BackendKind;

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

impl BackendConfig {
    /// OpenAI-compatible preset (Bearer auth).
    #[must_use]
    pub fn openai(api_key: Option<String>) -> Self {
        Self {
            family: BackendFamily::OpenAi,
            base_url: "https://api.openai.com/v1".to_string(),
            model: None,
            api_key,
            completions_path: default_completions_path(),
            auth: AuthScheme::Bearer,
            auth_header: default_auth_header(),
            auth_prefix: default_auth_prefix(),
            auth_query_param: default_auth_query_param(),
            extra_headers: Vec::new(),
            health_path: default_health_path(),
            health_method: HealthMethod::Get,
            name: Some("openai".to_string()),
        }
    }

    /// Anthropic preset (header auth, `x-api-key`, version pin, POST health).
    #[must_use]
    pub fn anthropic(api_key: Option<String>) -> Self {
        Self {
            family: BackendFamily::Anthropic,
            base_url: "https://api.anthropic.com/v1".to_string(),
            model: None,
            api_key,
            completions_path: "messages".to_string(),
            auth: AuthScheme::Header,
            auth_header: "x-api-key".to_string(),
            auth_prefix: default_auth_prefix(),
            auth_query_param: default_auth_query_param(),
            extra_headers: vec![("anthropic-version".to_string(), "2023-06-01".to_string())],
            health_path: "messages".to_string(),
            health_method: HealthMethod::Post,
            name: Some("anthropic".to_string()),
        }
    }

    /// Google Gemini preset (query-param auth, `models/{model}:generateContent`).
    #[must_use]
    pub fn gemini(api_key: Option<String>) -> Self {
        Self {
            family: BackendFamily::Gemini,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            model: None,
            api_key,
            completions_path: "models/{model}:generateContent".to_string(),
            auth: AuthScheme::Query,
            auth_header: default_auth_header(),
            auth_prefix: default_auth_prefix(),
            auth_query_param: default_auth_query_param(),
            extra_headers: Vec::new(),
            health_path: default_health_path(),
            health_method: HealthMethod::Get,
            name: Some("gemini".to_string()),
        }
    }

    /// Ollama preset (no auth, local server).
    #[must_use]
    pub fn ollama() -> Self {
        Self {
            family: BackendFamily::Ollama,
            base_url: "http://localhost:11434".to_string(),
            model: None,
            api_key: None,
            completions_path: "api/chat".to_string(),
            auth: AuthScheme::None,
            auth_header: default_auth_header(),
            auth_prefix: default_auth_prefix(),
            auth_query_param: default_auth_query_param(),
            extra_headers: Vec::new(),
            health_path: "api/tags".to_string(),
            health_method: HealthMethod::Get,
            name: Some("ollama".to_string()),
        }
    }

    /// DeepSeek preset (OpenAI protocol, Bearer auth).
    #[must_use]
    pub fn deepseek(api_key: Option<String>) -> Self {
        Self {
            family: BackendFamily::OpenAi,
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: None,
            api_key,
            completions_path: default_completions_path(),
            auth: AuthScheme::Bearer,
            auth_header: default_auth_header(),
            auth_prefix: default_auth_prefix(),
            auth_query_param: default_auth_query_param(),
            extra_headers: Vec::new(),
            health_path: default_health_path(),
            health_method: HealthMethod::Get,
            name: Some("deepseek".to_string()),
        }
    }

    /// MiMo preset (OpenAI protocol, Bearer auth).
    #[must_use]
    pub fn mimo(api_key: Option<String>) -> Self {
        Self {
            family: BackendFamily::OpenAi,
            base_url: "https://api.openai.com/v1".to_string(),
            model: None,
            api_key,
            completions_path: default_completions_path(),
            auth: AuthScheme::Bearer,
            auth_header: default_auth_header(),
            auth_prefix: default_auth_prefix(),
            auth_query_param: default_auth_query_param(),
            extra_headers: Vec::new(),
            health_path: default_health_path(),
            health_method: HealthMethod::Get,
            name: Some("mimo".to_string()),
        }
    }

    /// Build a config from a [`BackendKind`], applying the supplied key/URL
    /// overrides. Used by the legacy [`get_backend`](crate::get_backend) API.
    #[must_use]
    pub fn from_kind(kind: BackendKind, api_key: Option<String>, base_url: Option<String>) -> Self {
        let mut cfg = match kind {
            BackendKind::OpenAI => Self::openai(api_key),
            BackendKind::Anthropic => Self::anthropic(api_key),
            BackendKind::Ollama => Self::ollama(),
            BackendKind::Gemini => Self::gemini(api_key),
            BackendKind::DeepSeek => Self::deepseek(api_key),
            BackendKind::MiMo => Self::mimo(api_key),
        };
        if let Some(url) = base_url {
            cfg.base_url = url;
        }
        cfg
    }
}
