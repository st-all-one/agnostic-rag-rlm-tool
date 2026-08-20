//! Domain types for the RLM engine.
//!
//! This module is split into cohesive submodules:
//! - [`enums`] — backend/mode enums, custom tools, compaction policy, run output types
//! - [`node`] — the [`RlmNode`] decision-tree node and its constructors
//! - [`input`] — [`StartRunInput`] run configuration
//!
//! Tool traits/registry (`ExecutableTool`, `ToolRegistry`, built-in tools) live in
//! the sibling [`crate::tools`] module and are re-exported here so `types::*` keeps
//! covering the full public surface.

pub mod enums;
pub mod input;
pub mod node;

pub use enums::*;
pub use input::*;
pub use node::*;

pub use crate::tools::{
    CodeSearch, ExecutableTool, ListFilesTool, ReadFileTool, SearchCodeTool, ToolRegistry,
};
