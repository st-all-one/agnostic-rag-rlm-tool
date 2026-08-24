#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::needless_borrow,
        clippy::unnecessary_literal_bound,
        clippy::float_cmp,
        clippy::duration_suboptimal_units,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )
)]
pub mod consolidation;
pub mod decay;
pub mod engine;
pub mod history;
pub mod knowledge;
pub mod persist;
pub mod project;
pub mod transfer;
pub mod watch;

pub use consolidation::{ConsolidateOptions, ConsolidateResult, ConsolidationEngine};
pub use decay::{DecayConfig, SalienceInput, compute_salience, should_evict};
pub use engine::{
    IndexProjectOptions, IndexProjectResult, MemoryEngine, SearchOptions, SearchResult,
};
pub use history::{HistoryManager, QueryRecord};
pub use knowledge::KnowledgeEngine;
pub use persist::{
    AnalysisPersistOptions, DecisionPersistOptions, Frontmatter, PersistEngine, PersistResult,
    SearchPersistOptions, SessionPersistOptions, TrajectoryPersistOptions, WikiScope,
};
pub use project::{ProjectInfo, ProjectManager};
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
