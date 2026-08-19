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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::cast_possible_wrap)]
mod tests {
    use super::*;

    #[test]
    fn test_decay_config_default() {
        let cfg = DecayConfig::default();
        assert!((cfg.lambda - 0.01).abs() < f64::EPSILON);
        assert!(cfg.enabled);
    }

    #[test]
    fn test_decay_config_new() {
        let cfg = DecayConfig::new(0.05);
        assert!((cfg.lambda - 0.05).abs() < f64::EPSILON);
        assert!(cfg.enabled);
    }

    #[test]
    fn test_decay_config_disabled() {
        let cfg = DecayConfig::disabled();
        assert!(!cfg.enabled);
    }

    #[test]
    fn test_score_no_decay_when_disabled() {
        let cfg = DecayConfig::disabled();
        let score = cfg.score(1.0, 100.0);
        assert!((score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_score_zero_age() {
        let cfg = DecayConfig::default();
        let score = cfg.score(0.8, 0.0);
        assert!((score - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_score_negative_age_treated_as_zero() {
        let cfg = DecayConfig::default();
        let score = cfg.score(0.8, -5.0);
        assert!((score - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_score_decay_factor_one_hour() {
        let cfg = DecayConfig::new(0.01);
        let score = cfg.score(1.0, 1.0);
        // exp(-0.01 * 1) ≈ 0.99005
        assert!((score - 0.99005).abs() < 0.001);
    }

    #[test]
    fn test_score_decay_factor_69_hours() {
        let cfg = DecayConfig::new(0.01);
        let score = cfg.score(1.0, 69.0);
        // exp(-0.01 * 69) ≈ 0.5002 ≈ 50%
        assert!((score - 0.5002).abs() < 0.01);
    }

    #[test]
    fn test_score_decay_factor_large_age() {
        let cfg = DecayConfig::new(0.01);
        let score = cfg.score(1.0, 1000.0);
        // exp(-0.01 * 1000) = exp(-10) ≈ 0.000045
        assert!(score < 0.001);
    }

    #[test]
    fn test_score_higher_lambda_faster_decay() {
        let cfg_slow = DecayConfig::new(0.01);
        let cfg_fast = DecayConfig::new(0.1);
        let age = 10.0;
        let score_slow = cfg_slow.score(1.0, age);
        let score_fast = cfg_fast.score(1.0, age);
        assert!(score_slow > score_fast);
    }

    #[test]
    fn test_age_hours() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let ts = now - 7200; // 2 hours ago
        let age = DecayConfig::age_hours(ts);
        assert!((age - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_age_hours_future_timestamp() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let age = DecayConfig::age_hours(now + 1000);
        assert!(
            age < 0.001,
            "future timestamp should yield ~0 age, got {age}"
        );
    }

    #[test]
    fn test_refresh_sql_empty() {
        let sql = DecayConfig::refresh_sql(&[]);
        assert!(sql.is_empty());
    }

    #[test]
    fn test_refresh_sql_single_id() {
        let sql = DecayConfig::refresh_sql(&[42]);
        assert_eq!(
            sql,
            "UPDATE chunks SET last_accessed_at = unixepoch() WHERE id IN (?1)"
        );
    }

    #[test]
    fn test_refresh_sql_multiple_ids() {
        let sql = DecayConfig::refresh_sql(&[1, 2, 3]);
        assert_eq!(
            sql,
            "UPDATE chunks SET last_accessed_at = unixepoch() WHERE id IN (?1, ?2, ?3)"
        );
    }

    #[test]
    fn test_age_hours_sql() {
        let sql = DecayConfig::age_hours_sql();
        assert_eq!(sql, "(unixepoch() - last_accessed_at) / 3600.0");
    }

    #[test]
    fn test_score_preserves_zero_base() {
        let cfg = DecayConfig::new(0.01);
        let score = cfg.score(0.0, 100.0);
        assert!((score - 0.0).abs() < f32::EPSILON);
    }
}
