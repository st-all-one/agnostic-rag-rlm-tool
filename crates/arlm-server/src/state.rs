use arlm_storage::Storage;

use crate::write_queue::WriteQueue;

/// Shared state across gRPC handlers.
#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    pub storage: Storage,
    pub write_queue: WriteQueue,
    pub config: crate::config::ServerConfig,
}

impl AppState {
    /// Create a new AppState.
    ///
    /// # Errors
    ///
    /// Returns an error if the write queue cannot be created.
    pub fn new(
        storage: Storage,
        config: crate::config::ServerConfig,
    ) -> anyhow::Result<Self> {
        let write_queue = WriteQueue::new(
            storage.clone(),
            std::time::Duration::from_millis(config.flush_interval_ms),
            config.max_batch_size,
        );

        Ok(Self {
            storage,
            write_queue,
            config,
        })
    }
}
