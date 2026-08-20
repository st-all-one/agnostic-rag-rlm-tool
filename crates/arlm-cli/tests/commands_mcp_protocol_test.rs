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

use arlm_cli::commands::mcp::protocol::JsonRpcResponse;
use serde_json::json;

#[test]
fn test_jsonrpc_response_ok() {
    let resp = JsonRpcResponse::ok(Some(json!(1)), json!({"key": "val"}));
    assert_eq!(resp.jsonrpc, "2.0");
    assert!(resp.result.is_some());
    assert!(resp.error.is_none());
    assert_eq!(resp.id, Some(json!(1)));
}

#[test]
fn test_jsonrpc_response_err() {
    let resp = JsonRpcResponse::err(Some(json!(1)), -32600, "Invalid Request");
    assert_eq!(resp.jsonrpc, "2.0");
    assert!(resp.result.is_none());
    let err = resp.error.as_ref().expect("error should be present");
    assert_eq!(err.code, -32600);
    assert_eq!(err.message, "Invalid Request");
}
