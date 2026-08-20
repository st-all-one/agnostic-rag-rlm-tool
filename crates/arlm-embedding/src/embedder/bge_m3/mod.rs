//! BGE-M3 embedding backend (candle transformer).

pub mod embedder;
pub mod model;
pub mod ops;

pub(crate) mod attention;
pub(crate) mod weights;

pub use embedder::BgeM3Embedder;
pub use ops::{apply_matryoshka, gelu, half_to_f32, layer_norm, masked_fill};
