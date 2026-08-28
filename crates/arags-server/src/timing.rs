//! Structured timing utilities.
//!
//! Every handler and long-running operation emits a `completed` log line with
//! structured `elapsed_ms` / `elapsed_us` fields so execução can be monitored
//! without string parsing.

use std::time::{Duration, Instant};
use tracing::info;

/// A scoped timer that logs elapsed time when dropped.
///
/// Drop the timer (or let it go out of scope) to emit a structured
/// `info!` line:
/// `timer completed label=… elapsed_ms=… elapsed_us=…`
pub struct Timer {
    label: &'static str,
    start: Instant,
}

impl Timer {
    /// Start a timer with a label.
    #[must_use]
    pub fn new(label: &'static str) -> Self {
        info!(label, "timer started");
        Self {
            label,
            start: Instant::now(),
        }
    }

    /// Get elapsed time so far.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Get elapsed time in milliseconds.
    #[must_use]
    pub fn elapsed_ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }

    /// Get elapsed time in microseconds.
    #[must_use]
    pub fn elapsed_us(&self) -> u128 {
        self.start.elapsed().as_micros()
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        info!(
            label = self.label,
            duration_ms = elapsed.as_millis(),
            duration_us = elapsed.as_micros(),
            "timer completed"
        );
    }
}

/// Time the execution of a block, logging a structured `completed` line.
#[macro_export]
macro_rules! timed {
    ($label:literal, $block:expr) => {{
        let _timer = $crate::timing::Timer::new($label);
        $block
    }};
}

/// Time an async block, logging a structured `completed` line.
#[macro_export]
macro_rules! timed_async {
    ($label:literal, $block:expr) => {{
        let _timer = $crate::timing::Timer::new($label);
        $block.await
    }};
}
