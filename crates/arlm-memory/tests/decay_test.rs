#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::all, clippy::pedantic, clippy::nursery)]

use arlm_memory::decay::*;

fn input(created: u64, last: u64, count: u64, now: u64) -> SalienceInput {
    SalienceInput {
        created_at_ms: created,
        last_accessed_at_ms: last,
        access_count: count,
        now_ms: now,
    }
}

#[test]
fn test_recency_score_fresh_is_one() {
    // Age 0 -> no decay -> score 1.0
    assert!((recency_score(0, 1_000.0) - 1.0).abs() < 1e-9);
}

#[test]
fn test_recency_score_halves_at_half_life() {
    // Age == half_life -> score 0.5
    assert!((recency_score(1_000, 1_000.0) - 0.5).abs() < 1e-9);
}

#[test]
fn test_recency_score_decay_monotonic() {
    let a = recency_score(1_000, 1_000.0);
    let b = recency_score(2_000, 1_000.0);
    let c = recency_score(4_000, 1_000.0);
    assert!(a > b);
    assert!(b > c);
    assert!(c > 0.0);
}

#[test]
fn test_frequency_score_zero_access() {
    assert!((frequency_score(0) - 0.0).abs() < 1e-9);
}

#[test]
fn test_frequency_score_diminishing_returns() {
    let f1 = frequency_score(1);
    let f2 = frequency_score(2);
    let f10 = frequency_score(10);
    assert!(f1 > 0.0);
    assert!(f2 > f1);
    assert!(f10 > f2);
    // diminishing: each additional access adds less
    let delta1 = f2 - f1;
    let delta2 = f10 - frequency_score(9);
    assert!(delta1 > delta2);
}

#[test]
fn test_compute_salience_fresh_frequent_high() {
    let now = 1_000_000;
    let cfg = DecayConfig::default();
    let s = compute_salience(&input(now - 1_000, now, 50, now), &cfg);
    assert!(s > 0.8, "fresh + frequent should score high, got {s}");
}

#[test]
fn test_compute_salience_stale_rare_low() {
    let now = 100_000_000_000u64; // ~1157 days, large enough for a 90-day offset
    let cfg = DecayConfig::default();
    let old = now - (90u64 * 24 * 3_600 * 1_000); // ~90 days
    let s = compute_salience(&input(old, old, 0, now), &cfg);
    assert!(s < 0.3, "stale + rare should score low, got {s}");
}

#[test]
fn test_compute_salience_within_range() {
    let now = 1_000_000;
    let cfg = DecayConfig::default();
    for (c, l) in [(0u64, 0u64), (5, 10), (100, 1_000)] {
        let s = compute_salience(&input(now - l * 1_000, now - l * 1_000, c, now), &cfg);
        assert!((0.0..=1.0).contains(&s), "salience {s} out of range");
    }
}

#[test]
fn test_should_evict_below_threshold() {
    assert!(should_evict(0.1, 0.3));
    assert!(!should_evict(0.5, 0.3));
}

#[test]
fn test_clamp() {
    assert_eq!(clamp(-1.0, 0.0, 1.0), 0.0);
    assert_eq!(clamp(2.0, 0.0, 1.0), 1.0);
    assert_eq!(clamp(0.4, 0.0, 1.0), 0.4);
}

#[test]
fn test_now_ms_monotonic() {
    let a = now_ms();
    let b = now_ms();
    assert!(b >= a);
}
