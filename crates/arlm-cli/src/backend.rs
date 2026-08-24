//! Resolve an LLM backend from the user's merged configuration.
//!
//! Providers are described provider-agnostically in `~/.arlm/arlm.toml`
//! (`[[llm.backends]]`). When a named backend is found it is used directly via
//! [`arlm_llm::get_backend_from_config`]; otherwise the legacy `BackendKind`
//! preset path (with an env-var API key) is used for backwards compatibility.

use std::sync::Arc;

use anyhow::{Context, Result};
use arlm_llm::{BackendConfig, LlmBackend, LlmConfig, get_backend, get_backend_from_config};

/// Resolve a backend by logical `name` (or the configured default), falling
/// back to the legacy kind-based presets.
///
/// `model_override` optionally forces the model for the request.
///
/// # Errors
///
/// Returns an error if no backend matches and the legacy kind cannot be parsed
/// or requires a missing API key.
pub fn resolve_backend(
    config: Option<&LlmConfig>,
    name: Option<&str>,
    model_override: Option<&str>,
) -> Result<Arc<dyn LlmBackend>> {
    let name = name.map(ToString::to_string);

    // 1. Try a configured provider backend by name (or the first configured one).
    if let Some(cfg) = config {
        if !cfg.backends.is_empty() {
            let chosen: Option<&BackendConfig> = match &name {
                Some(n) => cfg.backends.iter().find(|b| {
                    b.name.as_deref() == Some(n)
                        || b.model.as_deref() == Some(n)
                        || n == b.family.as_str()
                }),
                None => cfg.backends.first(),
            };
            if let Some(cfg) = chosen {
                let mut cfg = cfg.clone();
                if let Some(m) = model_override {
                    cfg.model = Some(m.to_string());
                }
                let backend = get_backend_from_config(cfg.clone()).with_context(|| {
                    format!(
                        "failed to build backend '{}'",
                        cfg.name.unwrap_or_else(|| cfg.family.to_string())
                    )
                })?;
                return Ok(backend);
            }
        }
    }

    // 2. Legacy preset path.
    let kind_name = name.unwrap_or_else(|| "ollama".to_string());
    let kind: arlm_llm::BackendKind = kind_name
        .parse()
        .with_context(|| format!("unknown backend: {kind_name}"))?;

    let api_key = std::env::var(match kind {
        arlm_llm::BackendKind::OpenAI => "OPENAI_API_KEY",
        arlm_llm::BackendKind::Anthropic => "ANTHROPIC_API_KEY",
        arlm_llm::BackendKind::Gemini => "GEMINI_API_KEY",
        arlm_llm::BackendKind::Ollama => "",
        arlm_llm::BackendKind::DeepSeek => "DEEPSEEK_API_KEY",
        arlm_llm::BackendKind::MiMo => "MIMO_API_KEY",
    })
    .ok();

    let backend = get_backend(&kind, api_key, None)
        .with_context(|| format!("failed to create legacy backend '{kind_name}'"))?;
    Ok(backend)
}
