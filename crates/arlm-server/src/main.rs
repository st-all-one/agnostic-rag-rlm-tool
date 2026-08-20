//! arlm-server: Long-running gRPC server for the arlm platform.
//!
//! Thin binary entry point. All logic lives in the `arlm_server` library
//! so it can be exercised by unit and integration tests.
//!
//! Subcommands:
//! - `up` (default): load config, open storage, run the gRPC server.
//! - `status`: query a running server's health over gRPC (used by Docker HEALTHCHECK).

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,arlm_server=debug".parse().expect("valid env filter")),
        )
        .compact()
        .init();

    match std::env::args().nth(1).as_deref() {
        Some("status") => arlm_server::lifecycle::status_check().await,
        _ => arlm_server::run().await,
    }
}
