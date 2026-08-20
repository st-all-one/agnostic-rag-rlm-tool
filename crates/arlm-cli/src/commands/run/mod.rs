//! `arlm run` command: orchestrates a single RLM execution.
//!
//! The implementation is split into focused modules:
//! - [`config`]: the stable [`RunConfig`] request type.
//! - [`setup`]: backend/session/input construction helpers.
//! - [`engine`]: the top-level [`execute`] orchestrator.
//! - [`live`]: live terminal rendering integration.
//! - [`finalize`]: persistence, session save, and output formatting.

pub mod config;
pub mod engine;
pub mod finalize;
pub mod live;
pub mod setup;

pub use config::RunConfig;
pub use engine::execute;
