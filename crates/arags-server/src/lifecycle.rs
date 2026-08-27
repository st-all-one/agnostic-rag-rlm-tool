use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use arags_proto::proto::arags_service_client::AragsServiceClient;
use arags_proto::proto::arags_service_server::AragsServiceServer;
use arags_storage::{QuestionVectorStore, Storage, VectorStore};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tracing::{info, warn};

use crate::config::ServerConfig;
use crate::grpc::AragsGrpcService;
use crate::state::AppState;
use crate::timing::Timer;

/// Load config, open storage, wire the service and run the gRPC server.
///
/// Blocks until a shutdown signal is received.
///
/// # Errors
///
/// Returns an error if configuration, storage, the LLM backend or the server
/// setup fails.
pub async fn run() -> Result<()> {
    let _timer = Timer::new("server_startup");

    let config = ServerConfig::load().context("failed to load server config")?;

    info!(addr = %config.listen_addr, "starting arags-server");

    // Hybrid pooled mode (plan 020 `pool_size`): the writer pool serves
    // `connection()`-based writes while a dedicated shared connection keeps
    // the `conn()`-based read helpers valid. `pool_size == 1` degrades to
    // single-connection mode.
    let storage = if config.pool_size > 1 {
        Storage::open_pooled(&config.data_dir, config.pool_size)
            .context("failed to open pooled storage")?
    } else {
        Storage::open(&config.data_dir).context("failed to open storage")?
    };

    let vector_store = match VectorStore::open_with_dims(
        &config.data_dir,
        crate::state::embedder_dimension(),
    )
    .await
    {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            tracing::warn!(error = %e, "vector store unavailable, continuing without semantic search");
            None
        }
    };

    let question_vector_store = match arags_storage::QuestionVectorStore::open(
        &config.data_dir,
        crate::state::embedder_dimension(),
    ) {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            tracing::warn!(error = %e, "question vector store unavailable, semantic cache lookup disabled");
            None
        }
    };

    let rlm_vector_store = match arags_storage::RlmVectorStore::open(
        &config.data_dir,
        crate::state::embedder_dimension(),
    ) {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            tracing::warn!(error = %e, "rlm vector store unavailable, summary semantic search disabled");
            None
        }
    };

    let exploration_vector_store = match arags_storage::ExplorationVectorStore::open(
        &config.data_dir,
        crate::state::embedder_dimension(),
    ) {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            tracing::warn!(error = %e, "exploration vector store unavailable, map search disabled");
            None
        }
    };

    run_server(
        config,
        storage,
        vector_store,
        question_vector_store,
        rlm_vector_store,
        exploration_vector_store,
    )
    .await
}

/// Run the gRPC server with graceful shutdown.
///
/// # Errors
///
/// Returns an error if the server cannot be started or terminates uncleanly.
pub async fn run_server(
    config: ServerConfig,
    storage: Storage,
    vector_store: Option<Arc<VectorStore>>,
    question_vector_store: Option<Arc<QuestionVectorStore>>,
    rlm_vector_store: Option<Arc<arags_storage::RlmVectorStore>>,
    exploration_vector_store: Option<Arc<arags_storage::ExplorationVectorStore>>,
) -> Result<()> {
    let state = AppState::with_vector_stores(
        storage.clone(),
        config.clone(),
        vector_store,
        question_vector_store,
        rlm_vector_store,
        exploration_vector_store,
    )?;

    // Bootstrap rebuild: reconcile the four usearch vector spaces with canonical
    // SQLite before serving (issue `agnostic-rlm-rs-620d`). If a count diverges
    // the space is rebuilt from source-of-truth text; in-sync spaces are left
    // untouched. Best-effort — a failure is logged, never fatal to startup.
    if let Err(e) = crate::bootstrap::bootstrap_vector_spaces(&state).await {
        tracing::warn!(error = %e, "vector space bootstrap failed; serving with possibly stale indexes");
    }

    let grpc_service = AragsServiceServer::new(AragsGrpcService::new(state.clone()));

    // Periodic memory maintenance (plan 019, C.1). Runs in the background on a
    // fixed interval; `interval_secs == 0` disables it. The loop is tied to the
    // server process lifetime — when the runtime shuts down the spawned task is
    // dropped alongside it.
    if config.maintenance.interval_secs > 0 {
        let maint_storage = storage.clone();
        let recon_state = state.clone();
        let interval = config.maintenance.interval_secs;
        let floor = config.maintenance.decay_score_floor;
        // `[history] retention_days` (plan 020): 0 keeps history forever.
        let retention_days = config.history.retention_days;
        // `[chunk_retention_days]` (issue `agnostic-rlm-rs-8dcc`): 0 keeps
        // superseded chunk history forever.
        let chunk_retention_days = config.chunk_retention_days;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                // RLM: requeue claimed jobs whose lease expired without
                // completion so crashed volunteers do not strand work units.
                match maint_storage.requeue_expired_rlm_leases() {
                    Ok(n) if n > 0 => tracing::info!(requeued = n, "rlm expired leases requeued"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "rlm lease requeue failed"),
                }
                if let Err(e) =
                    crate::maintenance::run_maintenance("", &maint_storage, floor, false).await
                {
                    tracing::warn!(error = %e, "maintenance tick failed");
                } else {
                    tracing::info!("maintenance tick completed");
                }
                // QA re-digest queue (issue `agnostic-rlm-rs-d172`): revert
                // expired leases (default 300s) so the next cycle re-offers the
                // work to volunteers and a crashed volunteer never strands it.
                if let Err(e) = crate::reconcile::reclaim_expired_pending_qa(&recon_state).await {
                    tracing::warn!(error = %e, "reclaim expired pending qa failed");
                }
                // Reconcile worker (`agnostic-rlm-rs-36ae`): re-derive any
                // `pending_vector` rows from canonical SQLite text so the
                // usearch spaces catch up with the source of truth.
                if let Err(e) = crate::reconcile::reconcile_pending_vectors(&recon_state).await {
                    tracing::warn!(error = %e, "reconcile pending vectors tick failed");
                }
                if retention_days > 0 {
                    let cutoff =
                        chrono::Utc::now().timestamp() - i64::from(retention_days) * 86_400;
                    match maint_storage.purge_history_before(cutoff) {
                        Ok(0) => {}
                        Ok(n) => tracing::info!(purged = n, "history retention purge"),
                        Err(e) => tracing::warn!(error = %e, "history purge failed"),
                    }
                }
                if chunk_retention_days > 0 {
                    let storage = maint_storage.clone();
                    match crate::store::blocking(move || {
                        crate::store::chunks::purge_inactive_chunks(&storage, chunk_retention_days)
                    })
                    .await
                    {
                        Ok(0) => {}
                        Ok(n) => tracing::info!(purged = n, "inactive chunk purge"),
                        Err(e) => tracing::warn!(error = %e, "inactive chunk purge failed"),
                    }
                }
            }
        });
    }

    // Background WAL flush (plan 020 `flush_interval_ms`): a passive
    // checkpoint folds the write-ahead log back into the database on a fixed
    // cadence. `flush_interval_ms == 0` disables it. This is a pure
    // *optimization* — durability/consistency are guaranteed by SQLite WAL
    // itself, the bootstrap rebuild (`agnostic-rlm-rs-620d`) and the reconcile
    // worker (`agnostic-rlm-rs-36ae`); the periodic flush merely bounds the
    // WAL growth window and is safe to skip.
    if config.flush_interval_ms > 0 {
        let flush_storage = storage.clone();
        let flush_interval = std::time::Duration::from_millis(config.flush_interval_ms);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(flush_interval).await;
                if let Err(e) = flush_storage.wal_checkpoint() {
                    tracing::warn!(error = %e, "WAL flush tick failed");
                }
            }
        });
    }

    let addr = config
        .listen_addr
        .parse()
        .context("failed to parse listen address")?;

    let mut builder = Server::builder();

    if let (Some(cert), Some(key)) = (config.tls_cert(), config.tls_key()) {
        let identity = Identity::from_pem(&load_file(&cert)?, &load_file(&key)?);
        let mut tls = ServerTlsConfig::new().identity(identity);
        // mTLS (plan 020): when `mtls_ca` is set, clients must present a
        // certificate signed by this CA.
        if let Some(ca) = config.mtls_ca() {
            tls = tls.client_ca_root(Certificate::from_pem(&load_file(&ca)?));
            info!(ca = %ca.display(), "gRPC server requires client certificates (mTLS)");
        }
        builder = builder.tls_config(tls)?;
        info!(cert = %cert.display(), "gRPC server TLS enabled");
    } else {
        info!("gRPC server running without TLS (dev mode)");
    }

    info!(addr = %addr, "arags-server listening");

    builder
        .add_service(grpc_service)
        .serve_with_shutdown(addr, shutdown_signal())
        .await?;

    // Graceful shutdown: flush debounced vector-index mutations (optimization,
    // not a consistency requirement — bootstrap + reconcile own correctness).
    state.flush_vector_stores();

    info!("arags-server shut down gracefully");
    Ok(())
}

fn load_file(path: &PathBuf) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("failed to read TLS file {}", path.display()))
}

/// Query a running server's health over gRPC and print a summary.
///
/// Used by the `arags-server status` subcommand (and the Docker HEALTHCHECK).
///
/// # Errors
///
/// Returns an error if the config cannot be loaded or the server is unreachable.
pub async fn status_check() -> anyhow::Result<()> {
    let config = ServerConfig::load().context("failed to load server config")?;
    let endpoint = format!("http://{}", config.listen_addr);

    let mut client = AragsServiceClient::connect(endpoint)
        .await
        .context("failed to connect to arags-server (is it running?)")?;

    let status = client
        .get_server_status(())
        .await
        .context("GetServerStatus RPC failed")?
        .into_inner();

    println!(
        "OK version={} uptime_s={} active_runs={} total_projects={} total_chunks={}",
        status.version,
        status.uptime_seconds,
        status.active_runs,
        status.total_projects,
        status.total_chunks,
    );
    Ok(())
}

/// Wait for a shutdown signal (SIGINT or SIGTERM).
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                tokio::select! {
                    _ = ctrl_c => {
                        info!("received SIGINT, shutting down");
                    }
                    _ = sigterm.recv() => {
                        info!("received SIGTERM, shutting down");
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to install SIGTERM handler; waiting on Ctrl+C only");
                let _ = ctrl_c.await;
            }
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
        info!("received Ctrl+C, shutting down");
    }
}
