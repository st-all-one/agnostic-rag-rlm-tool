//! arlm-server: Long-running gRPC server for the arlm platform.
//!
//! Thin binary entry point. All logic lives in the `arlm_server` library
//! so it can be exercised by unit and integration tests.

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

    arlm_server::run().await
}