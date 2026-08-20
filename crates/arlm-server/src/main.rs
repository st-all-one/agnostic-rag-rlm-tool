//! arlm-server: Long-running gRPC server for the arlm platform.
//!
//! This binary provides the server-side component that manages storage,
//! indexing, search, summarization, and RLM execution for teams.

mod config;
mod grpc;
mod lifecycle;
mod state;
mod write_queue;

use anyhow::Result;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,arlm_server=debug".parse().expect("valid env filter")),
        )
        .compact()
        .init();

    let config = config::ServerConfig::load()?;

    info!(addr = %config.listen_addr, "starting arlm-server");

    // Initialize storage
    let storage = arlm_storage::Storage::open_pooled(&config.data_dir, config.pool_size)?;

    // Run migrations (on a single connection)
    {
        let conn = storage.conn();
        let conn = conn.lock();
        conn.execute_batch("PRAGMA optimize;")?;
    }

    // Start the gRPC server
    lifecycle::run_server(&config, storage).await
}
