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
    // Accept BOTH response shapes the Ollama `family` can hit:
    //  - OpenAI-compatible `/v1/chat/completions` (our default `completions_path`):
    //    `choices[0].message.content` + `usage.{prompt,completion}_tokens`.
    //  - Native Ollama `/api/chat`: top-level `message.content` +
    //    `prompt_eval_count`/`eval_count`.
    let content = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .or_else(|| {
            v.get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
        })
        .unwrap_or_default()
        .to_string();
    let (prompt, completion) = if let Some(u) = v.get("usage").and_then(Value::as_object) {
        (
            u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
            u.get("completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        )
    } else {
        (
            v.get("prompt_eval_count")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            v.get("eval_count").and_then(Value::as_u64).unwrap_or(0),
        )
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_openai_compat_shape_extracts_content_and_usage() {
        // Default `completions_path = "chat/completions"` hits Ollama's
        // OpenAI-compatible endpoint, which returns `choices[0].message.content`
        // + `usage`. Regression test for the empty-content bug (family=ollama
        // previously only read the native `message.content` top-level field).
        let body = r#"{
            "model": "llama3.2:1b",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Summary of the controller."}}],
            "usage": {"prompt_tokens": 12, "completion_tokens": 5, "total_tokens": 17}
        }"#;
        let resp = parse("llama3.2:1b", body).unwrap();
        assert_eq!(resp.content, "Summary of the controller.");
        assert_eq!(resp.usage.prompt_tokens, 12);
        assert_eq!(resp.usage.completion_tokens, 5);
    }

    #[test]
    fn parse_native_ollama_shape_still_extracts_content() {
        let body = r#"{
            "model": "llama3.2:1b",
            "message": {"role": "assistant", "content": "Native summary."},
            "prompt_eval_count": 9,
            "eval_count": 4
        }"#;
        let resp = parse("llama3.2:1b", body).unwrap();
        assert_eq!(resp.content, "Native summary.");
        assert_eq!(resp.usage.prompt_tokens, 9);
        assert_eq!(resp.usage.completion_tokens, 4);
    }
}
