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

use arags_llm::{BackendKind, get_backend};

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
