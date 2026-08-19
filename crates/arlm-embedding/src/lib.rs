pub mod chunker;
pub mod embedder;
pub mod pipeline;

use std::time::Instant;
use tracing::info_span;

/// A scoped timer that logs elapsed time when dropped.
pub(crate) struct Timer {
    label: &'static str,
    start: Instant,
}

impl Timer {
    #[must_use]
    pub(crate) fn new(label: &'static str) -> Self {
        let span = info_span!("timer", label = label);
        let _enter = span.enter();
        tracing::info!(label = label, "started");
        Self {
            label,
            start: Instant::now(),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        tracing::info!(
            label = self.label,
            elapsed_ms = elapsed.as_millis(),
            elapsed_us = elapsed.as_micros(),
            "completed"
        );
    }
}

/// The crate version.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
