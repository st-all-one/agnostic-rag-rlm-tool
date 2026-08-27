//! Exploration dataset knobs (plan 022).

use serde::{Deserialize, Serialize};

/// How a non-admin exploration persist is validated before it can surface.
///
/// - `Quorum` (default): a non-admin submitter's map is held non-surfaced and a
///   `candidate` submission is recorded. The actual accept/reject decision is
///   made later by the cosine quorum worker (issues `6d97`/`64af`); until then
///   the map stays gated out of search exactly like a `pending_review` map.
/// - `Review`: preserves the original plan-023 gate — when `require_review` is
///   also set, non-admin maps land as `pending_review` and surface only after an
///   admin approves. When `require_review` is `false`, `Review` mode is
///   fire-and-forget (no gating, no submission).
///
/// Admins auto-approve (maps stay `fresh`) in both modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationMode {
    #[default]
    Quorum,
    Review,
}

/// Exploration dataset knobs (plan 022).
#[derive(Debug, Clone, Deserialize)]
pub struct ExplorationConfig {
    /// Master switch: `false` rejects persists and skips the staleness hook.
    #[serde(default = "default_exploration_enabled")]
    pub enabled: bool,

    /// Similarity at/above which a map surfaces spontaneously (plan 022).
    #[serde(default = "default_hit_high")]
    pub hit_high: f32,

    /// Similarity below which nothing surfaces; between the bounds a map is
    /// returned as "possibly related".
    #[serde(default = "default_hit_low")]
    pub hit_low: f32,

    /// Age (days) at which the full age decay applies.
    #[serde(default = "default_max_age_days")]
    pub max_age_days: u32,

    /// Contradictions needed to auto-retire a map (`0` disables retirement).
    #[serde(default = "default_contradiction_limit")]
    pub contradiction_limit: i64,

    /// Lazy verify-on-hit (plan 022.8): ground each surfaced map's claim
    /// against current chunk vectors; weak evidence forces `stale`.
    #[serde(default)]
    pub verify_on_hit: bool,

    /// Minimum chunk-space similarity for the claim to count as grounded.
    #[serde(default = "default_grounding_min")]
    pub grounding_min_similarity: f32,

    /// Review gate (plan 023, borrowed from the RLM quality gate): maps from
    /// non-admin submitters land as `pending_review` and never surface in
    /// search until an admin approves them. `false` keeps fire-and-forget.
    ///
    /// Only honored when `validation_mode == Review`; in `Quorum` mode non-admin
    /// persists take the quorum candidate path regardless of this flag.
    #[serde(default)]
    pub require_review: bool,

    /// Non-admin validation strategy (see [`ValidationMode`]). Defaults to
    /// `Quorum` (candidate submission + non-surfaced until the quorum worker
    /// decides). `Review` preserves the original admin-approval gate.
    #[serde(default)]
    pub validation_mode: ValidationMode,
}

fn default_grounding_min() -> f32 {
    0.25
}

impl Default for ExplorationConfig {
    fn default() -> Self {
        Self {
            enabled: default_exploration_enabled(),
            hit_high: default_hit_high(),
            hit_low: default_hit_low(),
            max_age_days: default_max_age_days(),
            contradiction_limit: default_contradiction_limit(),
            verify_on_hit: false,
            grounding_min_similarity: default_grounding_min(),
            require_review: false,
            validation_mode: ValidationMode::default(),
        }
    }
}

fn default_exploration_enabled() -> bool {
    true
}
fn default_hit_high() -> f32 {
    0.72
}
fn default_hit_low() -> f32 {
    0.55
}
fn default_max_age_days() -> u32 {
    90
}
fn default_contradiction_limit() -> i64 {
    3
}
