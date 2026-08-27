//! OpenAI-family request/response mapping (also used by DeepSeek & MiMo).

use serde_json::Value;
use serde_json::json;

use super::{messages_simple, usage_from};
use crate::types::{CompletionRequest, CompletionResponse, LlmError};

pub(crate) fn build(req: &CompletionRequest) -> Value {
    let mut v = json!({
        "model": req.model,
        "messages": messages_simple(&req.messages),
    });
    if let Some(t) = req.temperature {
        v["temperature"] = json!(t);
    }
    if let Some(m) = req.max_tokens {
        v["max_tokens"] = json!(m);
    }
    if let Some(s) = &req.stop {
        v["stop"] = json!(s);
    }
    if let Some(s) = req.seed {
        v["seed"] = json!(s);
    }
    if let Some(tools) = &req.tools {
        let tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
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
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    Ok(CompletionResponse {
        content,
        model: model.to_string(),
        usage: usage_from(&v["usage"]),
    })
}
