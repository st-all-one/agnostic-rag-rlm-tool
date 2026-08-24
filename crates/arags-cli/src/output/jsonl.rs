use serde_json::json;

/// Render a single-object JSONL for content commands.
///
/// Shape: `{"<key>": <query>, "results": [{"file": ..., "text": ...}]}`.
/// This is the default output for `search`/`context`/`query`: an AI agent can
/// read only the matched file paths and their content without touching the
/// filesystem. One JSON object per invocation (printed as a single line).
#[must_use]
pub fn render_content_jsonl(key: &str, query: &str, results: &[(String, String)]) -> String {
    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|(file, text)| json!({ "file": file, "text": text }))
        .collect();
    json!({ key: query, "results": items }).to_string()
}
