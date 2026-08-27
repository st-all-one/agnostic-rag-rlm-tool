//! Anthropic-family request/response mapping.

use serde_json::Value;
use serde_json::json;

use super::{anthropic_messages, split_system};
use crate::types::{CompletionRequest, CompletionResponse, LlmError, UsageSummary};

pub(crate) fn build(req: &CompletionRequest) -> Value {
    let (system, rest) = split_system(&req.messages);
    let mut v = json!({
        "model": req.model,
        "messages": anthropic_messages(&rest),
    });
    if let Some(sys) = system {
        v["system"] = json!(sys);
    }
    if let Some(m) = req.max_tokens {
        v["max_tokens"] = json!(m);
    }
    if let Some(t) = req.temperature {
        v["temperature"] = json!(t);
    }
    if let Some(s) = &req.stop {
        v["stop_sequences"] = json!(s);
    }
    if let Some(tools) = &req.tools {
        let tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect();
        v["tools"] = json!(tools);
    }
    v
}

pub(crate) fn parse(model: &str, body: &str) -> Result<CompletionResponse, LlmError> {
    let v: Value =
        serde_json::from_str(body).map_err(|e| LlmError::Serialization(e.to_string()))?;
    let mut content = String::new();
    if let Some(arr) = v["content"].as_array() {
        for block in arr {
            if block["type"] == "text" {
                if let Some(t) = block["text"].as_str() {
                    content.push_str(t);
                }
            }
        }
    }
    let prompt = v["usage"]["input_tokens"].as_u64().unwrap_or(0);
    let completion = v["usage"]["output_tokens"].as_u64().unwrap_or(0);
    Ok(CompletionResponse {
        content,
        model: model.to_string(),
        usage: UsageSummary {
            prompt_tokens: u32::try_from(prompt).unwrap_or(u32::MAX),
            completion_tokens: u32::try_from(completion).unwrap_or(u32::MAX),
            total_tokens: u32::try_from(prompt + completion).unwrap_or(u32::MAX),
            cost_usd: 0.0,
        },
    })
}
