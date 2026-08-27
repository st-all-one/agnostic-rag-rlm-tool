//! Ollama-family request/response mapping.

use serde_json::Value;
use serde_json::json;

use super::messages_simple;
use crate::types::{CompletionRequest, CompletionResponse, LlmError, UsageSummary};

pub(crate) fn build(req: &CompletionRequest) -> Value {
    let mut v = json!({
        "model": req.model,
        "messages": messages_simple(&req.messages),
        // Ollama streams NDJSON chunks by default; we parse one body.
        "stream": false,
    });
    let mut opts = json!({});
    if let Some(t) = req.temperature {
        opts["temperature"] = json!(t);
    }
    if let Some(m) = req.max_tokens {
        opts["num_predict"] = json!(m);
    }
    if opts.as_object().is_some_and(|o| !o.is_empty()) {
        v["options"] = opts;
    }
    v
}

pub(crate) fn parse(model: &str, body: &str) -> Result<CompletionResponse, LlmError> {
    let v: Value =
        serde_json::from_str(body).map_err(|e| LlmError::Serialization(e.to_string()))?;
    let content = v["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let prompt = v["prompt_eval_count"].as_u64().unwrap_or(0);
    let completion = v["eval_count"].as_u64().unwrap_or(0);
    Ok(CompletionResponse {
        content,
        model: if model.is_empty() {
            "ollama".to_string()
        } else {
            model.to_string()
        },
        usage: UsageSummary {
            prompt_tokens: u32::try_from(prompt).unwrap_or(u32::MAX),
            completion_tokens: u32::try_from(completion).unwrap_or(u32::MAX),
            total_tokens: u32::try_from(prompt + completion).unwrap_or(u32::MAX),
            cost_usd: 0.0,
        },
    })
}
