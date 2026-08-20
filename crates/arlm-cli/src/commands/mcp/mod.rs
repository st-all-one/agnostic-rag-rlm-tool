#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    dead_code
)]

pub mod handlers;
pub mod protocol;
pub mod session;

pub use protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
pub use session::{McpState, handle_jsonrpc};
