#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    dead_code
)]

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::util::data_dir;

/// MCP server state shared across handlers.
#[allow(dead_code)]
pub struct McpState {
    pub project: PathBuf,
    pub project_name: String,
    pub verbose: bool,
}

/// JSON-RPC 2.0 request.
#[derive(Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub id: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 response.
#[derive(Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 error object.
#[derive(Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    fn ok(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    fn err(id: Option<serde_json::Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
            id,
        }
    }
}

/// Handle a JSON-RPC 2.0 request for the MCP protocol.
pub fn handle_jsonrpc(req: &JsonRpcRequest, state: &McpState) -> JsonRpcResponse {
    match req.method.as_str() {
        "initialize" => handle_initialize(req),
        "notifications/initialized" | "ping" => {
            JsonRpcResponse::ok(req.id.clone(), serde_json::json!({}))
        }
        "tools/list" => handle_tools_list(req),
        "tools/call" => handle_tools_call(req, state),
        _ => JsonRpcResponse::err(
            req.id.clone(),
            -32601,
            format!("Method not found: {}", req.method),
        ),
    }
}

fn handle_initialize(req: &JsonRpcRequest) -> JsonRpcResponse {
    JsonRpcResponse::ok(
        req.id.clone(),
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {
                    "listChanged": false
                }
            },
            "serverInfo": {
                "name": "arlm",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    )
}

fn handle_tools_list(req: &JsonRpcRequest) -> JsonRpcResponse {
    JsonRpcResponse::ok(
        req.id.clone(),
        serde_json::json!({
            "tools": [
                {
                    "name": "rlm_context",
                    "description": "Search project context using RLM. Returns relevant code chunks for a given task or question.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "task": {
                                "type": "string",
                                "description": "Task, question, or description of needed context"
                            },
                            "top_k": {
                                "type": "integer",
                                "default": 10,
                                "description": "Number of results to return"
                            }
                        },
                        "required": ["task"]
                    }
                },
                {
                    "name": "rlm_search",
                    "description": "Search project code with hybrid BM25. Returns code chunks with relevance scores.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "Search terms"
                            },
                            "top_k": {
                                "type": "integer",
                                "default": 10,
                                "description": "Number of results to return"
                            },
                            "file_pattern": {
                                "type": "string",
                                "description": "Optional file path filter pattern"
                            },
                            "min_score": {
                                "type": "number",
                                "description": "Optional minimum score threshold"
                            }
                        },
                        "required": ["query"]
                    }
                }
            ]
        }),
    )
}

fn handle_tools_call(req: &JsonRpcRequest, state: &McpState) -> JsonRpcResponse {
    let tool_name = req.params.get("name").and_then(|v| v.as_str());
    let arguments = req.params.get("arguments");

    let Some(name) = tool_name else {
        return JsonRpcResponse::err(req.id.clone(), -32602, "Missing tool name");
    };

    let result = match name {
        "rlm_context" => call_rlm_context(state, arguments),
        "rlm_search" => call_rlm_search(state, arguments),
        _ => {
            return JsonRpcResponse::err(req.id.clone(), -32602, format!("Unknown tool: {name}"));
        }
    };

    match result {
        Ok(content) => JsonRpcResponse::ok(
            req.id.clone(),
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": content
                }]
            }),
        ),
        Err(e) => JsonRpcResponse::ok(
            req.id.clone(),
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Error: {e}")
                }],
                "isError": true
            }),
        ),
    }
}

fn call_rlm_context(state: &McpState, args: Option<&serde_json::Value>) -> Result<String> {
    let default_params = serde_json::json!({});
    let params = args.unwrap_or(&default_params);

    let task = params
        .get("task")
        .and_then(|v| v.as_str())
        .context("Missing required parameter: task")?;

    let top_k = params
        .get("top_k")
        .and_then(serde_json::Value::as_u64)
        .map_or(10, |v| v as usize);

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(&state.project_name)
        .context("failed to check buffer")?
        .context("project not found. Run `arlm index` first.")?;

    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
    let hybrid = arlm_search::HybridSearch::new(bm25, None, None);

    let results = hybrid
        .search_fts(task, buffer.id, top_k, None)
        .context("FTS search failed")?;

    let context =
        arlm_search::build_context(&storage, &results, arlm_search::OutputFormat::Prompt, None)
            .context("failed to build context")?;

    Ok(format!(
        "Context for task: {task}\nProject: {}\n\n{context}",
        state.project_name
    ))
}

fn call_rlm_search(state: &McpState, args: Option<&serde_json::Value>) -> Result<String> {
    let default_params = serde_json::json!({});
    let params = args.unwrap_or(&default_params);

    let query = params
        .get("query")
        .and_then(|v| v.as_str())
        .context("Missing required parameter: query")?;

    let top_k = params
        .get("top_k")
        .and_then(serde_json::Value::as_u64)
        .map_or(10, |v| v as usize);

    let file_pattern = params.get("file_pattern").and_then(|v| v.as_str());

    let min_score = params
        .get("min_score")
        .and_then(serde_json::Value::as_f64)
        .map(|v| v as f32);

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    let buffer = storage
        .get_buffer_by_name(&state.project_name)
        .context("failed to check buffer")?
        .context("project not found. Run `arlm index` first.")?;

    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
    let hybrid = arlm_search::HybridSearch::new(bm25, None, None);

    let results = hybrid
        .search_fts(query, buffer.id, top_k, None)
        .context("FTS search failed")?;

    let search_results = arlm_search::build_search_results(&storage, &results, None)
        .context("failed to build results")?;

    let items: Vec<serde_json::Value> = search_results
        .iter()
        .filter(|r| min_score.is_none_or(|min| r.score >= min))
        .filter(|r| {
            #[allow(clippy::unnecessary_map_or)]
            file_pattern
                .as_ref()
                .map_or(true, |pat| r.file_path.contains(&**pat))
        })
        .map(|r| {
            serde_json::json!({
                "file": r.file_path,
                "line_start": r.line_start,
                "line_end": r.line_end,
                "score": r.score,
                "content": r.content,
                "language": r.language,
            })
        })
        .collect();

    let output = serde_json::json!({
        "query": query,
        "results": items,
        "count": items.len(),
    });

    serde_json::to_string_pretty(&output).context("failed to serialize search results")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsonrpc_response_ok() {
        let resp = JsonRpcResponse::ok(
            Some(serde_json::json!(1)),
            serde_json::json!({"key": "val"}),
        );
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        assert_eq!(resp.id, Some(serde_json::json!(1)));
    }

    #[test]
    fn test_jsonrpc_response_err() {
        let resp = JsonRpcResponse::err(Some(serde_json::json!(1)), -32600, "Invalid Request");
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.result.is_none());
        let err = resp.error.as_ref().expect("error should be present");
        assert_eq!(err.code, -32600);
        assert_eq!(err.message, "Invalid Request");
    }

    fn make_state() -> McpState {
        McpState {
            project: PathBuf::from("."),
            project_name: "test".to_string(),
            verbose: false,
        }
    }

    #[test]
    fn test_handle_initialize() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "initialize".to_string(),
            params: serde_json::json!({}),
            id: Some(serde_json::json!(1)),
        };
        let state = make_state();
        let resp = handle_jsonrpc(&req, &state);
        assert!(resp.result.is_some());
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "arlm");
        assert!(result["capabilities"]["tools"].is_object());
    }

    #[test]
    fn test_handle_tools_list() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/list".to_string(),
            params: serde_json::json!({}),
            id: Some(serde_json::json!(2)),
        };
        let state = make_state();
        let resp = handle_jsonrpc(&req, &state);
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().expect("tools should be array");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "rlm_context");
        assert_eq!(tools[1]["name"], "rlm_search");
    }

    #[test]
    fn test_handle_tools_call_unknown_tool() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: serde_json::json!({
                "name": "nonexistent_tool",
                "arguments": {}
            }),
            id: Some(serde_json::json!(3)),
        };
        let state = make_state();
        let resp = handle_jsonrpc(&req, &state);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[test]
    fn test_handle_tools_call_missing_name() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: serde_json::json!({
                "arguments": {}
            }),
            id: Some(serde_json::json!(4)),
        };
        let state = make_state();
        let resp = handle_jsonrpc(&req, &state);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[test]
    fn test_handle_tools_call_missing_task() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: serde_json::json!({
                "name": "rlm_context",
                "arguments": {}
            }),
            id: Some(serde_json::json!(5)),
        };
        let state = make_state();
        let resp = handle_jsonrpc(&req, &state);
        let result = resp.result.expect("should have result");
        assert_eq!(result["isError"], true);
    }

    #[test]
    fn test_handle_tools_call_missing_query() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: serde_json::json!({
                "name": "rlm_search",
                "arguments": {}
            }),
            id: Some(serde_json::json!(6)),
        };
        let state = make_state();
        let resp = handle_jsonrpc(&req, &state);
        let result = resp.result.expect("should have result");
        assert_eq!(result["isError"], true);
    }

    #[test]
    fn test_handle_ping() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "ping".to_string(),
            params: serde_json::json!({}),
            id: Some(serde_json::json!(7)),
        };
        let state = make_state();
        let resp = handle_jsonrpc(&req, &state);
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_handle_unknown_method() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "unknown/method".to_string(),
            params: serde_json::json!({}),
            id: Some(serde_json::json!(8)),
        };
        let state = make_state();
        let resp = handle_jsonrpc(&req, &state);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }
}
