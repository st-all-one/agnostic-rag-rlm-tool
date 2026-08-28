//! Shared exploration domain types, constants and the pure confidence model.
//!
//! Single source of truth for values every side must agree on: the CLI
//! (contract parsing/persisting), the server data plane (`arags-server`) and
//! persistence (`arags-storage`, which re-exports these).
//!
//! The confidence model ([`confidence_score`]) is a pure function so it can be
//! unit-tested and property-tested in isolation; the server only supplies
//! inputs (vector similarity, epoch drift, age, feedback counters).

use serde::{Deserialize, Serialize};

/// Contract version emitted by the current parser (`EXPLORATIONS.md`).
pub const TEMPLATE_VERSION_V1: &str = "v1";

/// Lifecycle states of an exploration map.
pub const STATUS_FRESH: &str = "fresh";
/// Anchor broke or admin invalidation; kept as auditable history.
pub const STATUS_STALE: &str = "stale";
/// Accumulated contradictions crossed the limit; excluded from search.
pub const STATUS_RETIRED: &str = "retired";
/// Awaiting admin approval under `[exploration] require_review` (plan 023,
/// quality gate borrowed from RLM); excluded from search until approved.
pub const STATUS_PENDING: &str = "pending_review";

/// Anchor roles: only `cited` anchors invalidate a map; `context` rows are
/// provenance-only.
pub const ROLE_CITED: &str = "cited";
pub const ROLE_CONTEXT: &str = "context";

/// Payload exchanged when persisting an exploration map.
///
/// All fields default on deserialization so readers tolerate older payloads;
/// writers omit empty vectors to keep messages small.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ExplorationPayload {
    /// Objective that drove the exploration (required by the contract).
    #[serde(default)]
    pub goal: String,
    /// Short digest used for embedding (required by the contract).
    #[serde(default)]
    pub summary: String,
    /// Full markdown contract document.
    #[serde(default)]
    pub body_markdown: String,
    /// Cited/context file paths relative to the project root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// Agent username (audit/provenance).
    #[serde(default)]
    pub created_by: String,
    /// LLM that produced the map (metadata).
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub template_version: String,
}

/// Thresholds and decay weights of the confidence model.
///
/// Defaults are conservative per the asymmetry principle: serving a wrong map
/// costs more than not serving a good one, so precision beats recall.
#[derive(Debug, Clone, Copy)]
pub struct ConfidenceConfig {
    /// Similarity above which a map surfaces spontaneously.
    pub hit_high: f32,
    /// Similarity below which nothing surfaces; between the bounds a map is
    /// returned as "possibly related".
    pub hit_low: f32,
    /// Confidence multiplier removed per epoch of drift.
    pub drift_weight: f32,
    /// Lower bound of the drift multiplier.
    pub drift_floor: f32,
    /// Confidence fraction removed at `max_age_days`.
    pub age_weight: f32,
    /// Lower bound of the age multiplier.
    pub age_floor: f32,
    /// Additive weight of the normalized feedback signal in [-1, 1].
    pub feedback_weight: f32,
    /// Age (in days) at which the full [`Self::age_weight`] applies.
    pub max_age_days: f32,
}

impl Default for ConfidenceConfig {
    fn default() -> Self {
        Self {
            hit_high: 0.72,
            hit_low: 0.55,
            drift_weight: 0.10,
            drift_floor: 0.25,
            age_weight: 0.30,
            age_floor: 0.40,
            feedback_weight: 0.10,
            max_age_days: 90.0,
        }
    }
}

impl ConfidenceConfig {
    /// Classify a raw vector similarity into a surfacing decision.
    ///
    /// # Panics
    ///
    /// Never panics; all comparisons are total.
    #[must_use]
    pub fn classify(&self, similarity: f32) -> HitClass {
        if similarity >= self.hit_high {
            HitClass::Strong
        } else if similarity >= self.hit_low {
            HitClass::Related
        } else {
            HitClass::None
        }
    }
}

/// Surfacing decision for a candidate map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitClass {
    /// Below `hit_low`: do not surface.
    None,
    /// Between the bounds: surface flagged as "possibly related".
    Related,
    /// At or above `hit_high`: surface spontaneously.
    Strong,
}

/// Composite trust score of an exploration candidate in `[0.0, 1.0]`.
///
/// `similarity * drift_factor * age_factor + feedback_term`, clamped. The
/// multiplicative terms never reach zero ([`ConfidenceConfig`] floors), so a
/// very similar recent-but-contradicted map still scores honestly instead of
/// vanishing.
///
/// Properties guaranteed for any inputs (see the proptest suite):
/// - monotone non-decreasing in `similarity` and `confirmed`;
/// - monotone non-increasing in `epoch_drift` and `age_days`;
/// - balanced feedback (`confirmed == contradicted`) is a no-op;
/// - result is always finite and within `[0, 1]`.
#[must_use]
pub fn confidence_score(
    similarity: f32,
    epoch_drift: u32,
    age_days: u32,
    confirmed: u32,
    contradicted: u32,
    config: &ConfidenceConfig,
) -> f32 {
    let sim = clamp01(finite_or(similarity, 0.0));

    let drift = exact_count(epoch_drift);
    let drift_factor = (1.0 - config.drift_weight * drift).max(config.drift_floor);

    let age_ratio = if config.max_age_days <= 0.0 {
        1.0
    } else {
        exact_count(age_days) / config.max_age_days
    };
    let age_factor = (1.0 - config.age_weight * age_ratio).max(config.age_floor);

    let confirmed_f = exact_count(confirmed);
    let contradicted_f = exact_count(contradicted);
    let total = confirmed_f + contradicted_f;
    let feedback = if total == 0.0 {
        0.0
    } else {
        (confirmed_f - contradicted_f) / total
    };

    clamp01(sim * drift_factor * age_factor + config.feedback_weight * feedback)
}

/// Convert a count to `f32` exactly: values at or above `2^24` saturate to
/// `2^24` (the f32 mantissa limit), keeping every conversion lossless so the
/// monotonicity properties hold without precision artifacts.
#[allow(clippy::cast_precision_loss)] // capped below the f32 mantissa limit
fn exact_count(value: u32) -> f32 {
    const MANTISSA_LIMIT: u32 = 1 << 24;
    if value >= MANTISSA_LIMIT {
        MANTISSA_LIMIT as f32
    } else {
        value as f32
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// Extract the claim text of a map for grounding verification (plan 022.8):
/// the content of the `## Conexões` section, falling back to `## Mapa` and
/// then to the whole document. Returns an empty slice only when the document
/// is empty.
#[must_use]
pub fn claim_text(body_markdown: &str) -> &str {
    let body = body_markdown.trim();
    if body.is_empty() {
        return "";
    }
    for section in ["## Conexões", "## Mapa"] {
        if let Some((_, tail)) = body.split_once(section) {
            let end = tail.find("\n## ").unwrap_or(tail.len());
            let text = tail[..end].trim();
            if !text.is_empty() {
                return text;
            }
        }
    }
    body
}

#[cfg(test)]
mod tests;
