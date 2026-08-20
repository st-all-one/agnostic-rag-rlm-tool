//! arlm-server: Long-running gRPC server for the arlm platform.
//!
//! Thin binary entry point. All logic lives in the `arlm_server` library
//! so it can be exercised by unit and integration tests.
//!
//! Subcommands:
//! - `up` (default): load config, open storage, run the gRPC server.
//! - `status`: query a running server's health over gRPC (used by Docker HEALTHCHECK).

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
    clippy::match_same_arms
)]

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,arlm_server=debug")),
        )
        .compact()
        .init();

    match std::env::args().nth(1).as_deref() {
        Some("status") => arlm_server::lifecycle::status_check().await,
        _ => arlm_server::run().await,
    }
}
