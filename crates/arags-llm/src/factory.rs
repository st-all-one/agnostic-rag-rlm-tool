use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use crate::backend::GenericBackend;
use crate::config::BackendConfig;
use crate::trait_llm::LlmBackend;
use crate::types::LlmError;

/// Selector for one of the built-in provider presets.
///
/// This is the stable entry point used by `arags-server` and other crates
/// (`BackendKind::from_str` + [`get_backend`]). Internally each variant maps
/// to a [`BackendConfig`] preset, so the per-provider structs no longer exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    OpenAI,
    Anthropic,
    Ollama,
    Gemini,
    DeepSeek,
    MiMo,
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenAI => write!(f, "openai"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::Ollama => write!(f, "ollama"),
            Self::Gemini => write!(f, "gemini"),
            Self::DeepSeek => write!(f, "deepseek"),
            Self::MiMo => write!(f, "mimo"),
        }
    }
}

impl FromStr for BackendKind {
    type Err = LlmError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" | "gpt" => Ok(Self::OpenAI),
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "ollama" | "local" => Ok(Self::Ollama),
            "gemini" | "google" => Ok(Self::Gemini),
            "deepseek" | "ds" => Ok(Self::DeepSeek),
            "mimo" => Ok(Self::MiMo),
            _ => Err(LlmError::Backend(format!(
                "unknown backend: {s}. Supported: openai, anthropic, ollama, gemini, deepseek, mimo"
            ))),
        }
    }
}

/// Create a new LLM backend based on the specified kind.
///
/// The `kind` is mapped to a built-in [`BackendConfig`] preset; `api_key` and
/// `base_url` override the preset defaults. This is equivalent to calling
/// [`get_backend_from_config`] with [`BackendConfig::from_kind`].
///
/// # Errors
///
/// Returns [`LlmError`] if the backend requires an API key that is not provided,
/// or if the backend kind is unknown.
pub fn get_backend(
    kind: &BackendKind,
    api_key: Option<String>,
    base_url: Option<String>,
) -> Result<Arc<dyn LlmBackend>, LlmError> {
    let config = BackendConfig::from_kind(*kind, api_key, base_url);
    Ok(Arc::new(GenericBackend::from_config(config)?))
}

/// Create a new LLM backend directly from a [`BackendConfig`].
///
/// This is the fully generic entry point: any provider/model combination that
/// can be described by a [`BackendConfig`] is supported without code changes.
///
/// # Errors
///
/// Returns [`LlmError`] if the configuration requires an API key that is absent.
pub fn get_backend_from_config(config: BackendConfig) -> Result<Arc<dyn LlmBackend>, LlmError> {
    Ok(Arc::new(GenericBackend::from_config(config)?))
}
