//! Live terminal tree visualization of an in-progress RLM run.
//!
//! Split into:
//! - [`model`]: the node-tree types and event [`LiveTree::apply`] logic.
//! - [`render`]: the terminal drawing routines.

pub mod model;
pub mod render;

pub use model::LiveNode;
pub use model::LiveTree;
