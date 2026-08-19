pub mod consolidation;
pub mod history;
pub mod knowledge;
pub mod project;
pub mod session;
pub mod trajectory;
pub mod transfer;
pub mod watch;

pub use consolidation::{ConsolidateOptions, ConsolidateResult, ConsolidationEngine};
pub use history::{HistoryManager, QueryRecord};
pub use knowledge::KnowledgeEngine;
pub use project::{ProjectInfo, ProjectManager};
pub use session::{SessionManager, SessionRecord};
pub use trajectory::{RunTrajectory, TrajectoryEngine};
pub use transfer::{TransferEngine, TransferOptions};
pub use watch::{WatchEvent, WatchHandle, WatchMonitor};

/// A scoped timer that logs elapsed time when dropped.
pub struct ScopedTimer {
    label: String,
    start: std::time::Instant,
}

impl ScopedTimer {
    /// Create a new timer with a label.
    #[must_use]
    pub fn new(label: &str) -> Self {
        tracing::info!(label = label, "started");
        Self {
            label: label.to_string(),
            start: std::time::Instant::now(),
        }
    }

    /// Get elapsed time in milliseconds.
    #[must_use]
    pub fn elapsed_ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }
}

impl Drop for ScopedTimer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        tracing::info!(
            label = self.label.as_str(),
            elapsed_ms = elapsed.as_millis(),
            "completed"
        );
    }
}

#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert!(!version().is_empty());
    }

    #[test]
    fn test_scoped_timer() {
        let timer = ScopedTimer::new("test_op");
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(timer.elapsed_ms() >= 5);
    }
}
