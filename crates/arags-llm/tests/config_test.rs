#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::needless_borrow,
    clippy::unnecessary_literal_bound,
    clippy::float_cmp,
    clippy::duration_suboptimal_units,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use arags_llm::config::{AuthScheme, BackendConfig, BackendFamily, HealthMethod, LlmConfig};
use arags_llm::{BackendKind, get_backend_from_config};

#[test]
fn test_presets_families() {
    assert_eq!(BackendConfig::openai(None).family, BackendFamily::OpenAi);
    assert_eq!(
        BackendConfig::anthropic(None).family,
        BackendFamily::Anthropic
    );
    assert_eq!(BackendConfig::gemini(None).family, BackendFamily::Gemini);
    assert_eq!(BackendConfig::ollama().family, BackendFamily::Ollama);
    assert_eq!(BackendConfig::deepseek(None).family, BackendFamily::OpenAi);
    assert_eq!(BackendConfig::mimo(None).family, BackendFamily::OpenAi);
}

#[test]
fn test_presets_auth() {
    assert_eq!(BackendConfig::openai(None).auth, AuthScheme::Bearer);
    assert_eq!(BackendConfig::anthropic(None).auth, AuthScheme::Header);
    assert_eq!(BackendConfig::gemini(None).auth, AuthScheme::Query);
    assert_eq!(BackendConfig::ollama().auth, AuthScheme::None);
}

#[test]
fn test_presets_names() {
    assert_eq!(BackendConfig::openai(None).name.as_deref(), Some("openai"));
    assert_eq!(
        BackendConfig::anthropic(None).name.as_deref(),
        Some("anthropic")
    );
    assert_eq!(BackendConfig::gemini(None).name.as_deref(), Some("gemini"));
    assert_eq!(BackendConfig::ollama().name.as_deref(), Some("ollama"));
    assert_eq!(
        BackendConfig::deepseek(None).name.as_deref(),
        Some("deepseek")
    );
    assert_eq!(BackendConfig::mimo(None).name.as_deref(), Some("mimo"));
}

#[test]
fn test_preset_health() {
    assert_eq!(
        BackendConfig::anthropic(None).health_method,
        HealthMethod::Post
    );
    assert_eq!(BackendConfig::anthropic(None).health_path, "messages");
    assert_eq!(BackendConfig::ollama().health_path, "api/tags");
    assert_eq!(
        BackendConfig::gemini(None).completions_path,
        "models/{model}:generateContent"
    );
}

#[test]
fn test_from_kind_ollama_no_key() {
    let c = BackendConfig::from_kind(BackendKind::Ollama, None, None);
    assert_eq!(c.family, BackendFamily::Ollama);
    assert_eq!(c.auth, AuthScheme::None);
    assert!(c.api_key.is_none());
}

#[test]
fn test_from_kind_overrides() {
    let c = BackendConfig::from_kind(
        BackendKind::OpenAI,
        Some("K".to_string()),
        Some("http://x".to_string()),
    );
    assert_eq!(c.api_key.as_deref(), Some("K"));
    assert_eq!(c.base_url, "http://x");
}

#[test]
fn test_deserialize_from_json() {
    let json = r#"{
        "family": "anthropic",
        "base_url": "https://example.com/v1",
        "api_key": "secret",
        "auth": "header",
        "auth_header": "x-api-key",
        "completions_path": "messages",
        "health_method": "post",
        "name": "my-cli"
    }"#;
    let c: BackendConfig = serde_json::from_str(json).expect("valid json");
    assert_eq!(c.family, BackendFamily::Anthropic);
    assert_eq!(c.auth, AuthScheme::Header);
    assert_eq!(c.auth_header, "x-api-key");
    assert_eq!(c.health_method, HealthMethod::Post);
    assert_eq!(c.name.as_deref(), Some("my-cli"));
}

#[test]
fn test_get_backend_from_config_openai_ok() {
    let cfg = BackendConfig::openai(Some("K".to_string()));
    let backend = get_backend_from_config(cfg).expect("should build");
    assert_eq!(backend.name(), "openai");
}

#[test]
fn test_get_backend_from_config_requires_key() {
    let cfg = BackendConfig::openai(None);
    assert!(get_backend_from_config(cfg).is_err());
}

#[test]
fn test_llm_config_parse_backends() {
    let toml = r#"
[[backends]]
name = "openai"
family = "openai"
api_key = "K"

[[backends]]
name = "ollama"
family = "ollama"
auth = "none"
"#;
    let cfg = toml.parse::<LlmConfig>().expect("valid toml");
    assert_eq!(cfg.backends.len(), 2);
    assert_eq!(cfg.backends[0].family, BackendFamily::OpenAi);
    assert_eq!(cfg.backends[1].family, BackendFamily::Ollama);
    assert_eq!(cfg.backends[0].name.as_deref(), Some("openai"));
    assert!(cfg.backends[1].api_key.is_none());
}
