//! Background maintenance and query-history retention configuration.

use serde::Deserialize;

/// Background maintenance configuration (plan 019, C.1).
#[derive(Debug, Clone, Deserialize)]
pub struct MaintenanceConfig {
    /// Cron interval in seconds. `0` disables the periodic ticker.
    #[serde(default = "default_maintenance_interval")]
    pub interval_secs: u64,
    /// Salience floor below which decayed chunks are removed.
    #[serde(default = "default_decay_score_floor")]
    pub decay_score_floor: f32,
}

fn default_maintenance_interval() -> u64 {
    3600
}

fn default_decay_score_floor() -> f32 {
    0.1
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        Self {
            interval_secs: default_maintenance_interval(),
            decay_score_floor: default_decay_score_floor(),
        }
    }
}

/// Query-history retention (plan 020).
#[derive(Debug, Clone, Deserialize)]
pub struct HistoryConfig {
    /// Purge history rows older than this many days via the maintenance
    /// ticker (`0` = keep forever).
    #[serde(default = "default_history_retention_days")]
    pub retention_days: u32,
}

fn default_history_retention_days() -> u32 {
    90
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            retention_days: default_history_retention_days(),
        }
    }
}
