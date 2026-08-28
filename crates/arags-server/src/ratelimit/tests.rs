//! Tests for the per-user fixed-window rate limiter (issue
//! `agnostic-rag-rlm-tool-7222`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::config::RateLimitConfig;
use crate::ratelimit::RateLimiter;

fn disabled_config() -> RateLimitConfig {
    RateLimitConfig {
        enabled: false,
        max_requests_per_window: 3,
        window_secs: 60,
    }
}

fn enabled_config() -> RateLimitConfig {
    RateLimitConfig {
        enabled: true,
        max_requests_per_window: 3,
        window_secs: 60,
    }
}

#[test]
fn rate_limit_allows_up_to_window_then_rejects() {
    let limiter = RateLimiter::new(enabled_config());
    // First 3 calls (within the window) are allowed.
    assert!(limiter.check("alice", 1_000), "call 1 allowed");
    assert!(limiter.check("alice", 1_001), "call 2 allowed");
    assert!(limiter.check("alice", 1_002), "call 3 allowed");
    // The 4th within the same window is rejected.
    assert!(!limiter.check("alice", 1_003), "call 4 rejected");
    // A different user has its own bucket.
    assert!(limiter.check("bob", 1_003), "bob still allowed");
}

#[test]
fn rate_limit_resets_after_window() {
    let limiter = RateLimiter::new(enabled_config());
    for _ in 0..3 {
        assert!(limiter.check("alice", 1_000));
    }
    assert!(!limiter.check("alice", 1_050), "still in window → rejected");
    // Advance past the 60s window: the bucket resets.
    assert!(
        limiter.check("alice", 1_061),
        "after window → allowed again"
    );
}

#[test]
fn rate_limit_disabled_is_always_pass() {
    let limiter = RateLimiter::new(disabled_config());
    for i in 0..100 {
        assert!(
            limiter.check("alice", 1_000 + u64::try_from(i).unwrap_or(0)),
            "disabled limiter never rejects"
        );
    }
}
