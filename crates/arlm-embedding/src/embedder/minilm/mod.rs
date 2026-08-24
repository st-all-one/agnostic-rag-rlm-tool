//! Native all-`MiniLM`-L6-v2 backend (candle, in-process CPU inference).
//!
//! The single embedding model of the `arlm` data plane — 22M parameters,
//! 384 dims, ~25 MB with INT8 weights. No external services.

mod embedder;
mod model;

pub use embedder::MinilmEmbedder;
pub use model::{HIDDEN_SIZE, MiniLmModel, NUM_HEADS, NUM_LAYERS};
