use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

/// Progress tracker for summarization operations.
#[derive(Clone)]
pub struct ProgressTracker {
    inner: Arc<ProgressInner>,
}

struct ProgressInner {
    running: AtomicBool,
    completed: AtomicU32,
    total: AtomicU32,
    current_file: parking_lot::Mutex<String>,
    elapsed_ms: AtomicU64,
    message: parking_lot::Mutex<String>,
}

impl ProgressTracker {
    /// Create a new progress tracker.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ProgressInner {
                running: AtomicBool::new(false),
                completed: AtomicU32::new(0),
                total: AtomicU32::new(0),
                current_file: parking_lot::Mutex::new(String::new()),
                elapsed_ms: AtomicU64::new(0),
                message: parking_lot::Mutex::new(String::new()),
            }),
        }
    }

    /// Start tracking progress.
    pub fn start(&self, total: u32) {
        self.inner.running.store(true, Ordering::Relaxed);
        self.inner.completed.store(0, Ordering::Relaxed);
        self.inner.total.store(total, Ordering::Relaxed);
    }

    /// Update progress for a completed file.
    pub fn update(&self, file: &str, completed: u32) {
        self.inner
            .current_file
            .lock()
            .replace_range(.., file);
        self.inner.completed.store(completed, Ordering::Relaxed);
    }

    /// Set the elapsed time.
    pub fn set_elapsed(&self, ms: u64) {
        self.inner.elapsed_ms.store(ms, Ordering::Relaxed);
    }

    /// Set a status message.
    pub fn set_message(&self, msg: &str) {
        self.inner.message.lock().replace_range(.., msg);
    }

    /// Mark as complete.
    pub fn finish(&self) {
        self.inner.running.store(false, Ordering::Relaxed);
    }

    /// Check if running.
    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::Relaxed)
    }

    /// Get current progress.
    pub fn progress(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            running: self.inner.running.load(Ordering::Relaxed),
            completed: self.inner.completed.load(Ordering::Relaxed),
            total: self.inner.total.load(Ordering::Relaxed),
            current_file: self.inner.current_file.lock().clone(),
            elapsed_ms: self.inner.elapsed_ms.load(Ordering::Relaxed),
            message: self.inner.message.lock().clone(),
        }
    }
}

impl Default for ProgressTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of current progress.
#[derive(Debug, Clone)]
pub struct ProgressSnapshot {
    pub running: bool,
    pub completed: u32,
    pub total: u32,
    pub current_file: String,
    pub elapsed_ms: u64,
    pub message: String,
}

impl ProgressSnapshot {
    /// Get completion percentage.
    pub fn percentage(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.completed as f64 / self.total as f64) * 100.0
    }
}
