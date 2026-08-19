use std::collections::HashMap;

const COMPLEXITY_WORDS: &[&str] = &[
    "why",
    "how",
    "explain",
    "analyze",
    "compare",
    "evaluate",
    "refactor",
    "optimize",
    "architect",
    "design",
    "integrate",
    "implement",
    "implementar",
    "debug",
    "fix",
    "troubleshoot",
    "migrate",
    "transform",
    "redesign",
];

const TECHNICAL_SIGNALS: &[&str] = &[
    "async",
    "concurrency",
    "trait",
    "lifecycle",
    "memory",
    "ownership",
    "borrow",
    "lifetime",
    "generics",
    "macro",
    "unsafe",
    "atomic",
    "channel",
    "mutex",
    "serialization",
    "pagination",
    "transaction",
];

const SIMPLE_SIGNALS: &[&str] = &[
    "what is", "define", "list", "show", "get", "read", "find", "name", "count", "size", "length",
    "format", "print", "echo",
];

const MAX_DEPTH: u32 = 5;

#[derive(Debug)]
pub struct DepthRouter {
    depth_successes: HashMap<u32, u32>,
    depth_attempts: HashMap<u32, u32>,
    #[allow(dead_code)]
    default_depth: u32,
}

impl DepthRouter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            depth_successes: HashMap::new(),
            depth_attempts: HashMap::new(),
            default_depth: 2,
        }
    }

    #[must_use]
    pub fn with_default_depth(default_depth: u32) -> Self {
        Self {
            depth_successes: HashMap::new(),
            depth_attempts: HashMap::new(),
            default_depth: default_depth.min(MAX_DEPTH),
        }
    }

    #[must_use]
    pub fn suggest_depth(&self, query: &str) -> u32 {
        let score = Self::complexity_score(query);
        let budget_factor = self.budget_adjustment();

        let raw: i32 = if score < 0.3 {
            1
        } else if score < 0.6 {
            2
        } else if score < 0.8 {
            3
        } else {
            4
        };

        #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
        let adjusted = {
            let val = (raw + budget_factor).max(1).min(MAX_DEPTH as i32);
            val as u32
        };

        if self.has_high_success_at_depth(adjusted) {
            adjusted
        } else if let Some(better) = self.best_performing_depth() {
            better
        } else {
            adjusted
        }
    }

    pub fn record_outcome(&mut self, depth: u32, success: bool) {
        let d = depth.min(MAX_DEPTH);
        *self.depth_attempts.entry(d).or_insert(0) += 1;
        if success {
            *self.depth_successes.entry(d).or_insert(0) += 1;
        }
    }

    #[must_use]
    fn complexity_score(query: &str) -> f64 {
        let lower = query.to_lowercase();
        let words: Vec<&str> = lower.split_whitespace().collect();
        #[allow(clippy::cast_precision_loss)]
        let word_count = words.len() as f64;
        if word_count == 0.0 {
            return 0.0;
        }

        let mut score = 0.0;

        let length_score = (word_count / 30.0).min(1.0);
        score += length_score * 0.25;

        #[allow(clippy::cast_precision_loss)]
        let complex_hits = COMPLEXITY_WORDS
            .iter()
            .filter(|w| lower.contains(*w))
            .count() as f64;
        let complex_score = (complex_hits / 3.0).min(1.0);
        score += complex_score * 0.35;

        #[allow(clippy::cast_precision_loss)]
        let tech_hits = TECHNICAL_SIGNALS
            .iter()
            .filter(|s| lower.contains(*s))
            .count() as f64;
        let tech_score = (tech_hits / 2.0).min(1.0);
        score += tech_score * 0.25;

        #[allow(clippy::cast_precision_loss)]
        let simple_hits = SIMPLE_SIGNALS.iter().filter(|s| lower.contains(*s)).count() as f64;
        let simple_penalty = (simple_hits / 2.0).min(1.0);
        score -= simple_penalty * 0.3;

        score.clamp(0.0, 1.0)
    }

    #[must_use]
    fn budget_adjustment(&self) -> i32 {
        let total_attempts: u32 = self.depth_attempts.values().sum();
        if total_attempts < 5 {
            return 0;
        }

        let avg_success_rate: f64 = self
            .depth_successes
            .iter()
            .map(|(d, s)| {
                let attempts = self.depth_attempts.get(d).copied().unwrap_or(1);
                let rate = f64::from(*s) / f64::from(attempts);
                rate * f64::from(*d)
            })
            .sum::<f64>()
            / f64::from(total_attempts);

        if avg_success_rate > 0.7 {
            -1
        } else {
            i32::from(avg_success_rate < 0.3)
        }
    }

    #[must_use]
    fn has_high_success_at_depth(&self, depth: u32) -> bool {
        let attempts = self.depth_attempts.get(&depth).copied().unwrap_or(0);
        if attempts < 3 {
            return true;
        }
        let successes = self.depth_successes.get(&depth).copied().unwrap_or(0);
        f64::from(successes) / f64::from(attempts) >= 0.5
    }

    #[must_use]
    fn best_performing_depth(&self) -> Option<u32> {
        let mut best_depth = None;
        let mut best_rate = 0.0_f64;

        for (depth, attempts) in &self.depth_attempts {
            if *attempts < 2 {
                continue;
            }
            let successes = self.depth_successes.get(depth).copied().unwrap_or(0);
            let rate = f64::from(successes) / f64::from(*attempts);
            if rate > best_rate {
                best_rate = rate;
                best_depth = Some(*depth);
            }
        }

        best_depth
    }
}

impl Default for DepthRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::manual_range_contains
)]
mod tests {
    use super::*;

    #[test]
    fn test_new_router() {
        let router = DepthRouter::new();
        assert_eq!(router.default_depth, 2);
    }

    #[test]
    fn test_with_default_depth() {
        let router = DepthRouter::with_default_depth(3);
        assert_eq!(router.default_depth, 3);
    }

    #[test]
    fn test_simple_query_shallow_depth() {
        let router = DepthRouter::new();
        let depth = router.suggest_depth("what is a hash map");
        assert!(depth <= 2, "simple query should route shallow, got {depth}");
    }

    #[test]
    fn test_complex_query_deeper_depth() {
        let router = DepthRouter::new();
        let depth = router.suggest_depth(
            "how to implement async concurrency with atomic transactions and memory ownership patterns",
        );
        assert!(depth >= 2, "complex query should route deep, got {depth}");
    }

    #[test]
    fn test_empty_query() {
        let router = DepthRouter::new();
        let depth = router.suggest_depth("");
        assert!(depth >= 1 && depth <= MAX_DEPTH);
    }

    #[test]
    fn test_technical_query() {
        let router = DepthRouter::new();
        let depth = router.suggest_depth("explain trait lifetime generics atomic channel mutex");
        assert!(
            depth >= 2,
            "technical query should route deeper, got {depth}"
        );
    }

    #[test]
    fn test_record_outcome() {
        let mut router = DepthRouter::new();
        router.record_outcome(2, true);
        router.record_outcome(2, true);
        router.record_outcome(2, false);
        assert_eq!(router.depth_attempts.get(&2), Some(&3));
        assert_eq!(router.depth_successes.get(&2), Some(&2));
    }

    #[test]
    fn test_record_outcome_caps_at_max_depth() {
        let mut router = DepthRouter::new();
        router.record_outcome(100, true);
        assert_eq!(router.depth_attempts.get(&MAX_DEPTH), Some(&1));
    }

    #[test]
    fn test_history_influences_routing() {
        let mut router = DepthRouter::new();
        for _ in 0..10 {
            router.record_outcome(3, true);
        }
        for _ in 0..10 {
            router.record_outcome(1, false);
        }
        let depth = router.suggest_depth("get value from map");
        assert!(
            depth >= 2,
            "historical success should influence routing, got {depth}"
        );
    }

    #[test]
    fn test_high_success_rate_adjusts_down() {
        let mut router = DepthRouter::new();
        for _ in 0..10 {
            router.record_outcome(3, true);
        }
        let adjustment = router.budget_adjustment();
        assert!(adjustment <= 0, "high success rate should adjust down");
    }

    #[test]
    fn test_low_success_rate_adjusts_up() {
        let mut router = DepthRouter::new();
        for _ in 0..10 {
            router.record_outcome(3, false);
        }
        let adjustment = router.budget_adjustment();
        assert!(adjustment >= 0, "low success rate should adjust up or stay");
    }

    #[test]
    fn test_best_performing_depth() {
        let mut router = DepthRouter::new();
        router.record_outcome(2, false);
        router.record_outcome(2, false);
        router.record_outcome(3, true);
        router.record_outcome(3, true);
        router.record_outcome(3, true);
        let best = router.best_performing_depth();
        assert_eq!(best, Some(3));
    }

    #[test]
    fn test_best_performing_depth_none_when_no_data() {
        let router = DepthRouter::new();
        assert!(router.best_performing_depth().is_none());
    }

    #[test]
    fn test_complexity_score_simple() {
        let score = DepthRouter::complexity_score("hello");
        assert!(
            score < 0.3,
            "simple query should have low score, got {score}"
        );
    }

    #[test]
    fn test_complexity_score_complex() {
        let score = DepthRouter::complexity_score(
            "how to refactor async concurrency with atomic memory ownership lifetime traits",
        );
        assert!(
            score > 0.5,
            "complex query should have high score, got {score}"
        );
    }

    #[test]
    fn test_suggest_depth_never_exceeds_max() {
        let router = DepthRouter::new();
        let depth = router.suggest_depth("why how explain analyze compare evaluate refactor optimize architect design integrate implement debug fix troubleshoot migrate transform redesign async concurrency trait lifecycle memory ownership borrow lifetime generics macro unsafe atomic channel mutex serialization pagination transaction");
        assert!(depth <= MAX_DEPTH);
    }

    #[test]
    fn test_suggest_depth_at_least_one() {
        let router = DepthRouter::new();
        let depth = router.suggest_depth("");
        assert!(depth >= 1);
    }

    #[test]
    fn test_default_trait() {
        let router = DepthRouter::default();
        assert_eq!(router.default_depth, 2);
    }
}
