use std::path::PathBuf;

use crate::metrics::ArlmMetrics;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub project: PathBuf,
    pub project_name: String,
    pub verbose: bool,
    pub metrics: ArlmMetrics,
}
