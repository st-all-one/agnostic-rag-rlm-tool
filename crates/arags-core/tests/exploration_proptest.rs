//! Property-based tests for the pure confidence model (plan 022).
//!
//! The score must be monotone in every input, bounded, and treat balanced
//! feedback as a no-op — the guarantees the server relies on when ranking
//! exploration candidates.

use arags_core::exploration::{ConfidenceConfig, confidence_score};
use proptest::prelude::*;

fn config() -> ConfidenceConfig {
    ConfidenceConfig::default()
}

fn sim_strategy() -> impl Strategy<Value = f32> {
    0.0f32..=1.0f32
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn score_is_always_finite_and_bounded(
        sim in sim_strategy(),
        drift in 0u32..10_000,
        age in 0u32..100_000,
        confirmed in 0u32..1_000,
        contradicted in 0u32..1_000,
        cfg in Just(config()),
    ) {
        let score = confidence_score(sim, drift, age, confirmed, contradicted, &cfg);
        prop_assert!(score.is_finite());
        prop_assert!((0.0..=1.0).contains(&score));
    }

    #[test]
    fn higher_similarity_never_lowers_the_score(
        lo in sim_strategy(),
        delta in 0.0f32..0.25,
        drift in 0u32..50,
        age in 0u32..3_000,
        confirmed in 0u32..20,
        contradicted in 0u32..20,
        cfg in Just(config()),
    ) {
        let hi = (lo + delta).min(1.0);
        let s_lo = confidence_score(lo, drift, age, confirmed, contradicted, &cfg);
        let s_hi = confidence_score(hi, drift, age, confirmed, contradicted, &cfg);
        prop_assert!(s_hi >= s_lo - 1e-6, "hi {s_hi} < lo {s_lo}");
    }

    #[test]
    fn more_drift_or_age_never_raises_the_score(
        sim in sim_strategy(),
        drift_lo in 0u32..5_000,
        extra_drift in 0u32..5_000,
        age_lo in 0u32..50_000,
        extra_age in 0u32..50_000,
        cfg in Just(config()),
    ) {
        let s_lo = confidence_score(sim, drift_lo, age_lo, 0, 0, &cfg);
        let s_hi = confidence_score(sim, drift_lo + extra_drift, age_lo + extra_age, 0, 0, &cfg);
        prop_assert!(s_hi <= s_lo + 1e-6, "decayed {s_hi} > fresh {s_lo}");
    }

    #[test]
    fn confirms_never_lower_and_contradictions_never_raise_the_score(
        sim in sim_strategy(),
        confirmed in 0u32..500,
        contradicted in 0u32..500,
        cfg in Just(config()),
    ) {
        let with_confirm = confidence_score(sim, 0, 0, confirmed + 1, contradicted, &cfg);
        let base = confidence_score(sim, 0, 0, confirmed, contradicted, &cfg);
        prop_assert!(with_confirm >= base - 1e-6);

        let with_contradict = confidence_score(sim, 0, 0, confirmed, contradicted + 1, &cfg);
        prop_assert!(with_contradict <= base + 1e-6);
    }

    #[test]
    fn balanced_feedback_matches_no_feedback(
        sim in sim_strategy(),
        n in 0u32..200,
        cfg in Just(config()),
    ) {
        let balanced = confidence_score(sim, 3, 30, n, n, &cfg);
        let neutral = confidence_score(sim, 3, 30, 0, 0, &cfg);
        prop_assert!((balanced - neutral).abs() < 1e-6);
    }
}
