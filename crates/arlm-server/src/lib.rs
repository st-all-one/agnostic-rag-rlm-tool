//! arlm-server: reference implementation of the long-running gRPC service
//! for the arlm (Agnostic RLM) platform.
//!
//! The crate is split into focused, type-driven modules:
//!
//! - [`config`]: TOML configuration + LLM backend wiring
//! - [`state`]: shared handler state (`AppState`)
//! - [`events`]: run/summarize event hub for streaming RPCs
//! - [`store`]: typed, pool-safe data access
//! - [`grpc`]: tonic service implementation (one file per RPC group)
//! - [`summarizer`]: hierarchical summarization engine
//! - [`write_queue`]: batched SQLite writer
//! - [`timing`]: structured scoped timers for execution monitoring

pub mod config;
pub mod events;
pub mod grpc;
pub mod indexing;
pub mod lifecycle;
pub mod state;
pub mod store;
pub mod summarizer;
pub mod timing;
pub mod write_queue;

pub use config::ServerConfig;
pub use state::AppState;

/// Load config, open storage, wire the service and run the gRPC server.
///
/// Blocks until a shutdown signal is received.
///
/// # Errors
///
/// Returns an error if configuration, storage, the LLM backend or the server
/// setup fails.
pub async fn run() -> anyhow::Result<()> {
    lifecycle::run().await
}
