#![allow(
    unsafe_code,
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

use std::path::PathBuf;

use arlm_cli::commands::mcp::protocol::JsonRpcRequest;
use arlm_cli::commands::mcp::session::{McpState, handle_jsonrpc};
use serde_json::json;

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
        params: json!({}),
        id: Some(json!(1)),
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
        params: json!({}),
        id: Some(json!(2)),
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
        params: json!({
            "name": "nonexistent_tool",
            "arguments": {}
        }),
        id: Some(json!(3)),
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
        params: json!({
            "arguments": {}
        }),
        id: Some(json!(4)),
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
        params: json!({
            "name": "rlm_context",
            "arguments": {}
        }),
        id: Some(json!(5)),
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
        params: json!({
            "name": "rlm_search",
            "arguments": {}
        }),
        id: Some(json!(6)),
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
        params: json!({}),
        id: Some(json!(7)),
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
        params: json!({}),
        id: Some(json!(8)),
    };
    let state = make_state();
    let resp = handle_jsonrpc(&req, &state);
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, -32601);
}
