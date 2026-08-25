use super::*;

#[test]
fn miss_below_floor() {
    let t = QaThresholds::default();
    let p = resolve_plan(0.2, 0.0, &t);
    assert!(p.is_miss);
    assert_eq!(p.digest_k, t.novel_k);
}

#[test]
fn false_positive_blocked_by_jaccard() {
    let t = QaThresholds::default();
    // "login" vs "logout" can be cos-similar but disjoint provenance.
    let p = resolve_plan(0.92, 0.1, &t);
    assert!(p.is_miss);
}

#[test]
fn top_tier_hit() {
    let t = QaThresholds::default();
    let p = resolve_plan(0.95, 0.8, &t);
    assert!(!p.is_miss);
    assert!(p.is_top_tier());
    assert_eq!(p.digest_k, 10);
    assert_eq!(p.provenance_k, 5);
    assert!(p.provenance_k <= p.digest_k);
    assert!(p.digest_k <= t.novel_k);
}

#[test]
fn widening_lower_tier() {
    let t = QaThresholds::default();
    let p = resolve_plan(0.65, 0.6, &t);
    assert!(!p.is_miss);
    assert!(p.digest_k >= 10);
    assert!(p.provenance_k <= p.digest_k);
}

#[test]
fn invariant_holds_at_every_tier() {
    let t = QaThresholds::default();
    for s in [0.50, 0.55, 0.62, 0.71, 0.83, 0.91, 0.99] {
        let p = resolve_plan(s, 0.9, &t);
        if !p.is_miss {
            assert!(p.provenance_k <= p.digest_k);
            assert!(p.digest_k <= t.novel_k);
        }
    }
}

#[test]
fn content_hash_is_deterministic_sha256_hex() {
    let a = chunk_content_hash("hello world");
    let b = chunk_content_hash("hello world");
    assert_eq!(a, b);
    assert_eq!(a.len(), 64);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, chunk_content_hash("hello world!"));
}
