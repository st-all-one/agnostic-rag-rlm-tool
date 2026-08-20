use std::sync::Arc;

use crate::types::{RlmRunResult, StartRunInput};

/// Abstraction over a memory backend (e.g. `arlm-memory`).
///
/// This trait decouples `arlm-core` from any concrete memory implementation so the
/// crate stays free of a hard dependency on `arlm-memory`. Callers inject a backend
/// as `Option<Arc<dyn MemoryProvider>>` into the solver and engine; when `None` the
/// engine behaves exactly as before (no context injection, no persistence).
pub trait MemoryProvider: Send + Sync {
    /// Retrieve relevant memory context strings for a given task.
    ///
    /// # Errors
    /// Returns an error message string when the backend fails.
    fn context(&self, task: &str) -> Result<Vec<String>, String>;

    /// Persist a completed run's trajectory.
    ///
    /// # Errors
    /// Returns an error message string when the backend fails.
    fn save_trajectory(
        &self,
        input: &StartRunInput,
        result: &RlmRunResult,
    ) -> Result<(), String>;
}

/// Convenience wrapper so `Option<Arc<dyn MemoryProvider>>` can be used ergonomically.
pub type SharedMemory = Option<Arc<dyn MemoryProvider>>;
