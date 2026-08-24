/// Abstraction over a memory backend (e.g. `arlm-memory`).
///
/// This trait decouples `arlm-core` from any concrete memory implementation so the
/// crate stays free of a hard dependency on `arlm-memory`. Callers inject a backend
/// as `Arc<dyn MemoryProvider>` when they need context retrieval; when unused the
/// engine behaves exactly as before (no context injection, no persistence).
pub trait MemoryProvider: Send + Sync {
    /// Retrieve relevant memory context strings for a given task.
    ///
    /// # Errors
    /// Returns an error message string when the backend fails.
    fn context(&self, task: &str) -> Result<Vec<String>, String>;
}
