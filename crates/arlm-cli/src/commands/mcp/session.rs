use std::path::PathBuf;

use serde_json::json;

use crate::commands::mcp::handlers;
use crate::commands::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};

/// MCP server state shared across handlers.
#[allow(dead_code)]
pub struct McpState {
    pub project: PathBuf,
    pub project_name: String,
    pub verbose: bool,
}

/// Handle a JSON-RPC 2.0 request for the MCP protocol.
#[tracing::instrument(skip_all, fields(method = %req.method))]
pub fn handle_jsonrpc(req: &JsonRpcRequest, state: &McpState) -> JsonRpcResponse {
    tracing::debug!(method = %req.method, "dispatch jsonrpc request");
    match req.method.as_str() {
        "initialize" => handle_initialize(req),
        "notifications/initialized" | "ping" => JsonRpcResponse::ok(req.id.clone(), json!({})),
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
    tracing::debug!("handling initialize");
    JsonRpcResponse::ok(
        req.id.clone(),
        json!({
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
    tracing::debug!("handling tools/list");
    JsonRpcResponse::ok(
        req.id.clone(),
        json!({
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
        tracing::warn!("tools/call received without a tool name");
        return JsonRpcResponse::err(req.id.clone(), -32602, "Missing tool name");
    };

    let result = match name {
        "rlm_context" => handlers::call_rlm_context(state, arguments),
        "rlm_search" => handlers::call_rlm_search(state, arguments),
        _ => {
            tracing::warn!(tool = name, "tools/call referenced unknown tool");
            return JsonRpcResponse::err(req.id.clone(), -32602, format!("Unknown tool: {name}"));
        }
    };

    match result {
        Ok(content) => JsonRpcResponse::ok(
            req.id.clone(),
            json!({
                "content": [{
                    "type": "text",
                    "text": content
                }]
            }),
        ),
        Err(e) => JsonRpcResponse::ok(
            req.id.clone(),
            json!({
                "content": [{
                    "type": "text",
                    "text": format!("Error: {e}")
                }],
                "isError": true
            }),
        ),
    }
}
