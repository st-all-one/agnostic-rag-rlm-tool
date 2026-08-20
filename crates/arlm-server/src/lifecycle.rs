use anyhow::Result;
use arlm_storage::Storage;
use arlm_proto::proto::arlm_service_server::ArlmServiceServer;
use tonic::transport::Server;
use tracing::info;

use crate::config::ServerConfig;
use crate::grpc::ArlmGrpcService;
use crate::state::AppState;

/// Run the gRPC server.
///
/// This function blocks until the server is shut down.
///
/// # Errors
///
/// Returns an error if the server cannot be started.
pub async fn run_server(config: &ServerConfig, storage: Storage) -> Result<()> {
    let state = AppState::new(storage, config.clone())?;

    let grpc_service = ArlmGrpcService::new(state);
    let addr = config.listen_addr.parse()?;

    info!(addr = %addr, "arlm-server listening");

    // Build the gRPC server with graceful shutdown
    Server::builder()
        .add_service(ArlmServiceServer::new(grpc_service))
        .serve_with_shutdown(addr, shutdown_signal())
        .await?;

    info!("arlm-server shut down gracefully");
    Ok(())
}

/// Wait for a shutdown signal (SIGINT or SIGTERM).
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler");

        tokio::select! {
            _ = ctrl_c => {
                info!("received SIGINT, shutting down");
            }
            _ = sigterm.recv() => {
                info!("received SIGTERM, shutting down");
            }
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
        info!("received Ctrl+C, shutting down");
    }
}
