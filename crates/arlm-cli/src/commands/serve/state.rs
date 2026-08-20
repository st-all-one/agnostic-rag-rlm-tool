use std::path::PathBuf;

use arlm_core::events::EventBus;

use crate::metrics::ArlmMetrics;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub project: PathBuf,
    pub project_name: String,
    pub verbose: bool,
    pub metrics: ArlmMetrics,
    pub event_bus: EventBus,
}
