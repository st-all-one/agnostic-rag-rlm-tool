use std::time::{SystemTime, UNIX_EPOCH};

/// Configuration for salience decay scoring.
///
/// Uses exponential decay: `score * exp(-lambda * age_hours)`.
/// Higher lambda means faster decay. With lambda=0.01, ~50% decay after 69 hours.
#[derive(Debug, Clone, Copy)]
pub struct DecayConfig {
    /// Decay rate. Higher = faster decay. Default: 0.01.
    pub lambda: f64,
    /// Whether decay is enabled. Default: true.
    pub enabled: bool,
}

impl Default for DecayConfig {
    fn default() -> Self {
        Self {
            lambda: 0.01,
            enabled: true,
        }
    }
}

impl DecayConfig {
    /// Create a new decay config with the given lambda.
    #[must_use]
    pub fn new(lambda: f64) -> Self {
        Self {
            lambda,
            enabled: true,
        }
    }

    /// Create a disabled decay config (no decay applied).
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            lambda: 0.01,
            enabled: false,
        }
    }

    /// Apply decay to a base score given the age in hours.
    ///
    /// Returns `base_score * exp(-lambda * age_hours)`.
    /// If decay is disabled, returns `base_score` unchanged.
    #[must_use]
    pub fn score(&self, base_score: f32, age_hours: f32) -> f32 {
        if !self.enabled || age_hours <= 0.0 {
            return base_score;
        }
        #[allow(clippy::cast_lossless, clippy::cast_possible_truncation)]
        let factor = (-self.lambda * f64::from(age_hours)).exp() as f32;
        base_score * factor
    }

    /// Compute age in hours from a `last_accessed_at` unix timestamp (seconds).
    ///
    /// Returns 0.0 if the timestamp is in the future or if current time cannot be read.
    #[must_use]
    pub fn age_hours(last_accessed_at: i64) -> f32 {
        #[allow(clippy::cast_possible_wrap)]
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);

        let age_secs = now.saturating_sub(last_accessed_at);
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        {
            age_secs as f32 / 3600.0
        }
    }

    /// SQL expression to update `last_accessed_at` for a set of chunk IDs.
    ///
    /// Use with parameterized query: pass chunk IDs as `?1, ?2, ...`.
    #[must_use]
    pub fn refresh_sql(chunk_ids: &[i64]) -> String {
        if chunk_ids.is_empty() {
            return String::new();
        }
        let placeholders: Vec<String> = chunk_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        format!(
            "UPDATE chunks SET last_accessed_at = unixepoch() WHERE id IN ({})",
            placeholders.join(", ")
        )
    }

    /// SQL expression to get the age in hours for a chunk, usable in SELECT.
    ///
    /// Returns `(unixepoch() - last_accessed_at) / 3600.0` as `age_hours`.
    #[must_use]
    pub fn age_hours_sql() -> &'static str {
        "(unixepoch() - last_accessed_at) / 3600.0"
    }
}
