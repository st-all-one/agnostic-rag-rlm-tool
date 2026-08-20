use std::sync::Arc;

use crate::anthropic::AnthropicBackend;
use crate::deepseek::DeepSeekBackend;
use crate::gemini::GeminiBackend;
use crate::mimo::MiMoBackend;
use crate::ollama::OllamaBackend;
use crate::openai::OpenAiBackend;
use crate::retry::RetryConfig;
use crate::trait_llm::LlmBackend;
use crate::types::LlmError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendKind {
    OpenAI,
    Anthropic,
    Ollama,
    Gemini,
    DeepSeek,
    MiMo,
}

impl std::fmt::Display for BackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

impl std::str::FromStr for BackendKind {
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
/// # Errors
///
/// Returns `LlmError` if the backend requires an API key that is not provided,
/// or if the backend kind is unknown.
pub fn get_backend(
    kind: &BackendKind,
    api_key: Option<String>,
    base_url: Option<String>,
) -> Result<Arc<dyn LlmBackend>, LlmError> {
    let retry_config = RetryConfig::default();

    let backend: Arc<dyn LlmBackend> = match kind {
        BackendKind::OpenAI => {
            let key = api_key.ok_or_else(|| {
                LlmError::Auth("OPENAI_API_KEY required for OpenAI backend".to_string())
            })?;
            let mut b = OpenAiBackend::new(key).with_retry_config(retry_config);
            if let Some(url) = base_url {
                b = b.with_base_url(url);
            }
            Arc::new(b)
        }
        BackendKind::Anthropic => {
            let key = api_key.ok_or_else(|| {
                LlmError::Auth("ANTHROPIC_API_KEY required for Anthropic backend".to_string())
            })?;
            let mut b = AnthropicBackend::new(key).with_retry_config(retry_config);
            if let Some(url) = base_url {
                b = b.with_base_url(url);
            }
            Arc::new(b)
        }
        BackendKind::Ollama => {
            let mut b = OllamaBackend::new().with_retry_config(retry_config);
            if let Some(url) = base_url {
                b = b.with_base_url(url);
            }
            Arc::new(b)
        }
        BackendKind::Gemini => {
            let key = api_key.ok_or_else(|| {
                LlmError::Auth("GEMINI_API_KEY required for Gemini backend".to_string())
            })?;
            let mut b = GeminiBackend::new(key).with_retry_config(retry_config);
            if let Some(url) = base_url {
                b = b.with_base_url(url);
            }
            Arc::new(b)
        }
        BackendKind::DeepSeek => {
            let key = api_key.ok_or_else(|| {
                LlmError::Auth("DEEPSEEK_API_KEY required for DeepSeek backend".to_string())
            })?;
            let mut b = DeepSeekBackend::new(key).with_retry_config(retry_config);
            if let Some(url) = base_url {
                b = b.with_base_url(url);
            }
            Arc::new(b)
        }
        BackendKind::MiMo => {
            let key = api_key.ok_or_else(|| {
                LlmError::Auth("MIMO_API_KEY required for MiMo backend".to_string())
            })?;
            let mut b = MiMoBackend::new(key).with_retry_config(retry_config);
            if let Some(url) = base_url {
                b = b.with_base_url(url);
            }
            Arc::new(b)
        }
    };

    Ok(backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_kind_display() {
        assert_eq!(BackendKind::OpenAI.to_string(), "openai");
        assert_eq!(BackendKind::Anthropic.to_string(), "anthropic");
        assert_eq!(BackendKind::Ollama.to_string(), "ollama");
        assert_eq!(BackendKind::Gemini.to_string(), "gemini");
        assert_eq!(BackendKind::DeepSeek.to_string(), "deepseek");
        assert_eq!(BackendKind::MiMo.to_string(), "mimo");
    }

    #[test]
    fn test_backend_kind_from_str() {
        assert_eq!(
            "openai".parse::<BackendKind>().unwrap(),
            BackendKind::OpenAI
        );
        assert_eq!("gpt".parse::<BackendKind>().unwrap(), BackendKind::OpenAI);
        assert_eq!(
            "anthropic".parse::<BackendKind>().unwrap(),
            BackendKind::Anthropic
        );
        assert_eq!(
            "claude".parse::<BackendKind>().unwrap(),
            BackendKind::Anthropic
        );
        assert_eq!(
            "ollama".parse::<BackendKind>().unwrap(),
            BackendKind::Ollama
        );
        assert_eq!("local".parse::<BackendKind>().unwrap(), BackendKind::Ollama);
        assert_eq!(
            "gemini".parse::<BackendKind>().unwrap(),
            BackendKind::Gemini
        );
        assert_eq!(
            "google".parse::<BackendKind>().unwrap(),
            BackendKind::Gemini
        );
        assert_eq!(
            "deepseek".parse::<BackendKind>().unwrap(),
            BackendKind::DeepSeek
        );
        assert_eq!("ds".parse::<BackendKind>().unwrap(), BackendKind::DeepSeek);
        assert_eq!("mimo".parse::<BackendKind>().unwrap(), BackendKind::MiMo);
    }

    #[test]
    fn test_backend_kind_from_str_invalid() {
        let result = "unknown".parse::<BackendKind>();
        assert!(result.is_err());
    }

    #[test]
    fn test_get_backend_ollama() {
        let backend = get_backend(&BackendKind::Ollama, None, None);
        assert!(backend.is_ok());
        assert_eq!(backend.expect("should succeed").name(), "ollama");
    }

    #[test]
    fn test_get_backend_openai_requires_key() {
        let backend = get_backend(&BackendKind::OpenAI, None, None);
        assert!(backend.is_err());
    }

    #[test]
    fn test_get_backend_openai_with_key() {
        let backend = get_backend(&BackendKind::OpenAI, Some("test-key".to_string()), None);
        assert!(backend.is_ok());
        assert_eq!(backend.expect("should succeed").name(), "openai");
    }

    #[test]
    fn test_get_backend_anthropic_requires_key() {
        let backend = get_backend(&BackendKind::Anthropic, None, None);
        assert!(backend.is_err());
    }

    #[test]
    fn test_get_backend_gemini_requires_key() {
        let backend = get_backend(&BackendKind::Gemini, None, None);
        assert!(backend.is_err());
    }

    #[test]
    fn test_get_backend_deepseek_requires_key() {
        let backend = get_backend(&BackendKind::DeepSeek, None, None);
        assert!(backend.is_err());
    }

    #[test]
    fn test_get_backend_deepseek_with_key() {
        let backend = get_backend(&BackendKind::DeepSeek, Some("test-key".to_string()), None);
        assert!(backend.is_ok());
        assert_eq!(backend.expect("should succeed").name(), "deepseek");
    }

    #[test]
    fn test_get_backend_mimo_requires_key() {
        let backend = get_backend(&BackendKind::MiMo, None, None);
        assert!(backend.is_err());
    }

    #[test]
    fn test_get_backend_mimo_with_key() {
        let backend = get_backend(&BackendKind::MiMo, Some("test-key".to_string()), None);
        assert!(backend.is_ok());
        assert_eq!(backend.expect("should succeed").name(), "mimo");
    }

    #[test]
    fn test_get_backend_with_custom_url() {
        let backend = get_backend(
            &BackendKind::Ollama,
            None,
            Some("http://custom:11434".to_string()),
        );
        assert!(backend.is_ok());
    }
}
