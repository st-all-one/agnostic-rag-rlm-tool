//! In-memory per-user fixed-window rate limiter (issue `agnostic-rlm-rs-7222`).
//!
//! Keyed by authenticated username, gating mutating RPCs. No external crate: a
//! `parking_lot::Mutex<HashMap>` holds one small bucket per user. When the
//! config is disabled the limiter is a no-op pass. `now` is supplied by the
//! caller (seconds since epoch) so tests can advance a fake clock.

use std::collections::HashMap;

use parking_lot::Mutex;

use crate::config::RateLimitConfig;

/// One user's request bucket for the current window.
#[derive(Debug, Clone, Copy)]
struct Bucket {
    count: u32,
    window_start: u64,
}

/// Fixed-window per-user rate limiter.
#[derive(Debug)]
pub struct RateLimiter {
    config: RateLimitConfig,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl RateLimiter {
    /// Build a limiter from config.
    #[must_use]
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Check whether `username` is allowed to make another request at time
    /// `now` (seconds since epoch).
    ///
    /// Returns `true` if allowed. When the config is disabled this always
    /// returns `true`. A denied call does NOT consume a slot (the caller
    /// short-circuits before doing work, so we must not double-count).
    #[must_use]
    pub fn check(&self, username: &str, now: u64) -> bool {
        if !self.config.enabled {
            return true;
        }
        let mut map = self.buckets.lock();
        let bucket = map.entry(username.to_string()).or_insert(Bucket {
            count: 0,
            window_start: now,
        });
        // Window expired → reset the bucket to this call.
        if now.saturating_sub(bucket.window_start) >= self.config.window_secs {
            bucket.count = 0;
            bucket.window_start = now;
        }
        if bucket.count >= self.config.max_requests_per_window {
            return false;
        }
        bucket.count += 1;
        true
    }
}

#[cfg(test)]
mod tests;
