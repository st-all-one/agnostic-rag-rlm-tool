//! Per-[`BackendFamily`] request builders and response parsers.
//!
//! The dispatch lives here; the concrete wire mappings for each provider are in
//! the sibling `openai` / `anthropic` / `gemini` / `ollama` submodules so the
//! surface of each provider stays cohesive and below the line budget.

use serde_json::Value;

use crate::config::BackendFamily;
use crate::types::{CompletionRequest, LlmError};

pub(crate) mod anthropic;
pub(crate) mod gemini;
pub(crate) mod ollama;
pub(crate) mod openai;

impl BackendFamily {
    #[must_use]
    pub fn build_request(self, req: &CompletionRequest) -> Value {
        match self {
            BackendFamily::OpenAi => openai::build(req),
            BackendFamily::Anthropic => anthropic::build(req),
            BackendFamily::Gemini => gemini::build(req),
            BackendFamily::Ollama => ollama::build(req),
        }
    }

    /// Parse a provider response body into a [`CompletionResponse`].
    ///
    /// # Errors
    ///
    /// Returns a family-specific [`LlmError`] when the body cannot be
    /// interpreted (missing fields, unexpected shape).
    pub fn parse_response(
        self,
        model: &str,
        body: &str,
    ) -> Result<crate::types::CompletionResponse, LlmError> {
        match self {
            BackendFamily::OpenAi => openai::parse(model, body),
            BackendFamily::Anthropic => anthropic::parse(model, body),
            BackendFamily::Gemini => gemini::parse(model, body),
            BackendFamily::Ollama => ollama::parse(model, body),
        }
    }

    pub(crate) fn error_message(self, body: &str) -> String {
        match self {
            BackendFamily::Ollama => body.to_string(),
            _ => crate::transport::extract_json_error_message(body),
        }
    }
}

/// Render messages as a flat `role`/`content` list (OpenAI / Ollama shape).
pub(crate) fn messages_simple(messages: &[crate::types::Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": m.role.to_string(),
                "content": m.content,
            })
        })
        .collect()
}

pub(crate) fn anthropic_messages(messages: &[crate::types::Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": m.role.to_string(),
                "content": [{ "type": "text", "text": m.content }],
            })
        })
        .collect()
}

pub(crate) fn gemini_contents(messages: &[crate::types::Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            let role = match m.role {
                crate::types::Role::User | crate::types::Role::System => "user",
                crate::types::Role::Assistant => "model",
            };
            serde_json::json!({ "role": role, "parts": [{ "text": m.content }] })
        })
        .collect()
}

pub(crate) fn split_system(
    messages: &[crate::types::Message],
) -> (Option<String>, Vec<crate::types::Message>) {
    let mut system: Option<String> = None;
    let mut rest = Vec::new();
    for m in messages {
        if m.role == crate::types::Role::System {
            system = Some(m.content.clone());
        } else {
            rest.push(m.clone());
        }
    }
    (system, rest)
}

/// Build a [`UsageSummary`] from a parsed `usage` JSON object. Shared by every
/// family parser so token accounting stays consistent.
pub(crate) fn usage_from(v: &Value) -> crate::types::UsageSummary {
    let prompt = v["prompt_tokens"].as_u64().unwrap_or(0);
    let completion = v["completion_tokens"].as_u64().unwrap_or(0);
    let total = v["total_tokens"].as_u64().unwrap_or(prompt + completion);
    crate::types::UsageSummary {
        prompt_tokens: u32::try_from(prompt).unwrap_or(u32::MAX),
        completion_tokens: u32::try_from(completion).unwrap_or(u32::MAX),
        total_tokens: u32::try_from(total).unwrap_or(u32::MAX),
        cost_usd: 0.0,
    }
}
