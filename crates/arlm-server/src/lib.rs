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
//! - [`timing`]: structured scoped timers for execution monitoring

#![allow(
    clippy::missing_errors_doc,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::wildcard_imports,
    clippy::similar_names,
    clippy::needless_raw_string_hashes,
    clippy::redundant_closure_for_method_calls,
    clippy::field_reassign_with_default,
    clippy::items_after_statements,
    clippy::unused_self,
    clippy::unnecessary_wraps,
    clippy::unnecessary_fallible_conversions,
    clippy::useless_conversion,
    clippy::result_large_err,
    clippy::must_use_candidate,
    clippy::cloned_instead_of_copied,
    clippy::map_unwrap_or,
    clippy::map_identity,
    clippy::needless_borrow,
    clippy::trivially_copy_pass_by_ref,
    clippy::match_same_arms
)]

pub mod admin;
pub mod auth;
pub mod config;
pub mod events;
pub mod grpc;
pub mod indexing;
pub mod lifecycle;
pub mod state;
pub mod store;
pub mod summarizer;
pub mod timing;

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
