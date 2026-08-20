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

/// Maximum depth the router will assign to any node.
pub const MAX_DEPTH: u32 = 5;

/// Model tiers: expensive at root, cheaper at leaves.
const MODEL_TIERS: &[&str] = &[
    "gpt-4o",       // depth 0 — root
    "gpt-4o-mini",  // depth 1
    "gpt-4o-mini",  // depth 2
    "gpt-4o-mini",  // depth 3
    "gpt-4o-mini",  // depth 4+
];

#[derive(Debug)]
pub struct DepthRouter {
    depth_successes: HashMap<u32, u32>,
    depth_attempts: HashMap<u32, u32>,
    /// Default routing depth used when no history is available.
    pub default_depth: u32,
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

    /// Number of attempts recorded at a given depth (0 if none).
    #[must_use]
    pub fn attempts(&self, depth: u32) -> u32 {
        self.depth_attempts.get(&depth).copied().unwrap_or(0)
    }

    /// Number of successes recorded at a given depth (0 if none).
    #[must_use]
    pub fn successes(&self, depth: u32) -> u32 {
        self.depth_successes.get(&depth).copied().unwrap_or(0)
    }

    /// Select the best model for a given depth.
    ///
    /// Uses the root model for depth 0, cheaper models for deeper nodes.
    /// If a custom model is provided, it's used at depth 0 and the tier
    /// model for deeper levels.
    #[must_use]
    pub fn select_model(&self, depth: u32, custom_model: Option<&str>) -> String {
        let idx = (depth as usize).min(MODEL_TIERS.len() - 1);
        if depth == 0 {
            custom_model
                .unwrap_or(MODEL_TIERS[0])
                .to_string()
        } else {
            MODEL_TIERS[idx].to_string()
        }
    }

    #[must_use]
    pub fn complexity_score(query: &str) -> f64 {
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
    pub fn budget_adjustment(&self) -> i32 {
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
    pub fn has_high_success_at_depth(&self, depth: u32) -> bool {
        let attempts = self.depth_attempts.get(&depth).copied().unwrap_or(0);
        if attempts < 3 {
            return true;
        }
        let successes = self.depth_successes.get(&depth).copied().unwrap_or(0);
        f64::from(successes) / f64::from(attempts) >= 0.5
    }

    #[must_use]
    pub fn best_performing_depth(&self) -> Option<u32> {
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

