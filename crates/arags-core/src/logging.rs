use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

static INIT: Once = Once::new();
static VERBOSE: AtomicBool = AtomicBool::new(false);

/// Initialize the logging system.
pub fn init_logging(verbose: bool) {
    INIT.call_once(|| {
        VERBOSE.store(verbose, Ordering::Relaxed);

        let env_filter = if verbose {
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug"))
        } else {
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        };

        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                fmt::layer()
                    .with_target(true)
                    .with_thread_ids(verbose)
                    .with_file(verbose)
                    .with_line_number(verbose)
                    .compact(),
            )
            .init();
    });
}

/// Check if verbose mode is enabled.
#[must_use]
pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// A scoped timer that logs elapsed time when dropped.
pub struct ScopedTimer {
    start: Instant,
    label: String,
}

impl ScopedTimer {
    /// Create a new timer with a label.
    pub fn new(label: &str) -> Self {
        tracing::info!(label = label, "started");
        Self {
            start: Instant::now(),
            label: label.to_string(),
        }
    }

    /// Create a timer only if verbose mode is enabled.
    #[must_use]
    pub fn new_verbose(label: &str) -> Option<Self> {
        if is_verbose() {
            Some(Self::new(label))
        } else {
            None
        }
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

impl Drop for ScopedTimer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        tracing::info!(
            label = self.label.as_str(),
            elapsed_ms = elapsed.as_millis(),
            elapsed_us = elapsed.as_micros(),
            "completed"
        );
    }
}

/// Macro for timing a block with a label.
#[macro_export]
macro_rules! timed {
    ($label:expr, $block:expr) => {{
        let _timer = $crate::logging::ScopedTimer::new($label);
        $block
    }};
}

/// Macro for timing a block only in verbose mode.
#[macro_export]
macro_rules! timed_verbose {
    ($label:expr, $block:expr) => {{
        let _timer = $crate::logging::ScopedTimer::new_verbose($label);
        $block
    }};
}

/// Log a metric (structured data for profiling).
pub fn log_metric(name: &str, value: f64, unit: &str) {
    tracing::info!(
        metric_name = name,
        metric_value = value,
        metric_unit = unit,
        "metric"
    );
}

/// Log a metric only in verbose mode.
pub fn log_metric_verbose(name: &str, value: f64, unit: &str) {
    if is_verbose() {
        log_metric(name, value, unit);
    }
}
