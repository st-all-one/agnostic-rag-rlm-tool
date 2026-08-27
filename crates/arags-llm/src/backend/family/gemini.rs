//! Gemini-family request/response mapping.

use serde_json::Value;
use serde_json::json;

use super::{gemini_contents, split_system};
use crate::types::{CompletionRequest, CompletionResponse, LlmError, UsageSummary};

pub(crate) fn build(req: &CompletionRequest) -> Value {
    let (system, rest) = split_system(&req.messages);
    let mut v = json!({
        "contents": gemini_contents(&rest),
    });
    if let Some(sys) = system {
        v["systemInstruction"] = json!({ "parts": [{ "text": sys }] });
    }
    let mut gen_cfg = json!({});
    if let Some(t) = req.temperature {
        gen_cfg["temperature"] = json!(t);
    }
    if let Some(m) = req.max_tokens {
        gen_cfg["maxOutputTokens"] = json!(m);
    }
    if let Some(s) = &req.stop {
        gen_cfg["stopSequences"] = json!(s);
    }
    if gen_cfg.as_object().is_some_and(|o| !o.is_empty()) {
        v["generationConfig"] = gen_cfg;
    }
    if let Some(tools) = &req.tools {
        let decls: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            })
            .collect();
        v["tools"] = json!({ "functionDeclarations": decls });
    }
    v
}

pub(crate) fn parse(model: &str, body: &str) -> Result<CompletionResponse, LlmError> {
    let v: Value =
        serde_json::from_str(body).map_err(|e| LlmError::Serialization(e.to_string()))?;
    let mut content = String::new();
    if let Some(cands) = v["candidates"].as_array() {
        for c in cands {
            if let Some(parts) = c["content"]["parts"].as_array() {
                for p in parts {
                    if let Some(t) = p["text"].as_str() {
                        content.push_str(t);
                    }
                }
            }
        }
    }
    let prompt = v["usageMetadata"]["promptTokenCount"].as_u64().unwrap_or(0);
    let completion = v["usageMetadata"]["candidatesTokenCount"]
        .as_u64()
        .unwrap_or(0);
    let total = v["usageMetadata"]["totalTokenCount"]
        .as_u64()
        .unwrap_or(prompt + completion);
    Ok(CompletionResponse {
        content,
        model: model.to_string(),
        usage: UsageSummary {
            prompt_tokens: u32::try_from(prompt).unwrap_or(u32::MAX),
            completion_tokens: u32::try_from(completion).unwrap_or(u32::MAX),
            total_tokens: u32::try_from(total).unwrap_or(u32::MAX),
            cost_usd: 0.0,
        },
    })
}
