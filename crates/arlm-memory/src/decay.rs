//! Salience / decay scoring for memory entries.
//!
//! Pure, self-contained heuristics used to rank and evict wiki pages and chunks.
//! Salience is a value in `[0.0, 1.0]` derived from three signals:
//!
//! - **recency** — how recently the entry was last accessed (exponential decay),
//! - **frequency** — how often the entry has been accessed (diminishing returns),
//! - **age** — how old the entry is (gentle penalty).
//!
//! This module intentionally has no dependency on `arlm-search` so it can be reused
//! by any memory backend.

use std::time::{SystemTime, UNIX_EPOCH};

/// Configuration for the salience computation.
#[derive(Debug, Clone)]
pub struct DecayConfig {
    /// Milliseconds after which recency score halves.
    pub recency_half_life_ms: f64,
    /// Weight of the recency component (0.0–1.0).
    pub recency_weight: f64,
    /// Weight of the frequency component (0.0–1.0).
    pub frequency_weight: f64,
    /// Weight of the age component (0.0–1.0).
    pub age_weight: f64,
}

impl Default for DecayConfig {
    fn default() -> Self {
        Self {
            recency_half_life_ms: 30.0 * 24.0 * 3_600.0 * 1_000.0,
            recency_weight: 0.6,
            frequency_weight: 0.3,
            age_weight: 0.1,
        }
    }
}

/// Input signals for a single memory entry.
#[derive(Debug, Clone, Copy)]
pub struct SalienceInput {
    /// Creation timestamp in milliseconds since the Unix epoch.
    pub created_at_ms: u64,
    /// Last-access timestamp in milliseconds since the Unix epoch.
    pub last_accessed_at_ms: u64,
    /// Number of times the entry has been accessed.
    pub access_count: u64,
    /// Current time in milliseconds since the Unix epoch.
    pub now_ms: u64,
}

/// Return the current time in milliseconds since the Unix epoch.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Clamp a value into `[min, max]`.
#[must_use]
pub fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// Compute the recency component in `(0.0, 1.0]`.
///
/// Newer accesses score closer to `1.0`; older accesses decay exponentially.
#[must_use]
pub fn recency_score(age_ms: u64, half_life_ms: f64) -> f64 {
    if half_life_ms <= 0.0 {
        return 0.0;
    }
    let exponent = f64::from(u32::try_from(age_ms).unwrap_or(u32::MAX)) / half_life_ms;
    0.5f64.powf(exponent)
}

/// Compute the frequency component in `[0.0, 1.0)` with diminishing returns.
///
/// Uses `1 - 1/(1 + n)` so the first few accesses matter most.
#[must_use]
pub fn frequency_score(access_count: u64) -> f64 {
    let n = f64::from(u32::try_from(access_count).unwrap_or(u32::MAX));
    1.0 - (1.0 / (1.0 + n))
}

/// Compute the age penalty in `[0.0, 1.0]`.
///
/// Older entries receive a larger penalty (closer to `1.0`).
#[must_use]
pub fn age_penalty(age_ms: u64, half_life_ms: f64) -> f64 {
    if half_life_ms <= 0.0 {
        return 1.0;
    }
    let exponent = f64::from(u32::try_from(age_ms).unwrap_or(u32::MAX)) / half_life_ms;
    1.0 - 0.5f64.powf(exponent)
}

/// Compute the combined salience score in `[0.0, 1.0]`.
///
/// The three weighted components (recency, frequency, age) are normalized by the
/// sum of their weights. A non-positive total weight is treated as a degenerate
/// configuration and yields a salience of `0.0` instead of panicking.
#[must_use]
pub fn compute_salience(input: &SalienceInput, config: &DecayConfig) -> f64 {
    let total = config.recency_weight + config.frequency_weight + config.age_weight;
    if total <= 0.0 {
        return 0.0;
    }

    let recency_age = input.now_ms.saturating_sub(input.last_accessed_at_ms);
    let creation_age = input.now_ms.saturating_sub(input.created_at_ms);

    let recency = recency_score(recency_age, config.recency_half_life_ms);
    let frequency = frequency_score(input.access_count);
    let age = age_penalty(creation_age, config.recency_half_life_ms);

    let raw = config.recency_weight * recency
        + config.frequency_weight * frequency
        + config.age_weight * (1.0 - age);

    clamp(raw / total, 0.0, 1.0)
}

/// Decide whether an entry should be evicted given a minimum salience threshold.
#[must_use]
pub fn should_evict(salience: f64, min_salience: f64) -> bool {
    salience < min_salience
}
