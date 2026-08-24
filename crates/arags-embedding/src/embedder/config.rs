use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;

use super::minilm::MinilmEmbedder;
use super::{Embedder, LightweightEmbedder};

/// Which embedding backend to instantiate.
///
/// `Minilm` is the single production model of the data plane;
/// `Lightweight` is a deterministic hash fixture for tests and degraded
/// mode — it is not a user-selectable alternative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbeddingModel {
    /// Native all-`MiniLM`-L6-v2 via candle. Requires model weights on disk.
    #[default]
    Minilm,
    /// Lightweight deterministic embedder (no weights, no candle inference).
    Lightweight,
}

/// Weight quantization applied to the `MiniLM` projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quantization {
    /// Full-precision (f32) linear projections.
    None,
    /// INT8 quantization (GGML `Q8_0`). Best balance of speed, memory and
    /// quality — the default.
    #[default]
    Int8,
}

impl Quantization {
    /// Map to the corresponding candle GGML dtype, if any.
    #[must_use]
    pub fn ggml_dtype(self) -> Option<candle_core::quantized::GgmlDType> {
        match self {
            Quantization::None => None,
            Quantization::Int8 => Some(candle_core::quantized::GgmlDType::Q8_0),
        }
    }

    /// Parse a `[embedder] quantization` string (`"int8"` default, `"none"`).
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "f32" | "fp32" => Quantization::None,
            _ => Quantization::Int8,
        }
    }
}

/// Configuration controlling which embedder is built and how.
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    /// The backend to instantiate.
    pub model: EmbeddingModel,
    /// Directory containing `model.safetensors` + `tokenizer.json` (`MiniLM`).
    pub model_dir: Option<PathBuf>,
    /// Weight quantization (INT8 by default).
    pub quantization: Quantization,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            model: EmbeddingModel::Minilm,
            model_dir: None,
            quantization: Quantization::Int8,
        }
    }
}

impl EmbeddingConfig {
    /// Lightweight, fast, weight-free configuration for tests.
    #[must_use]
    pub fn for_tests() -> Self {
        Self {
            model: EmbeddingModel::Lightweight,
            model_dir: None,
            quantization: Quantization::None,
        }
    }
}

/// Build an embedder from a configuration.
///
/// Returns a [`LightweightEmbedder`] for the test fixture model, or a
/// [`MinilmEmbedder`] (with INT8/f32 quantization) for `MiniLM`.
///
/// # Errors
///
/// Returns an error if `MiniLM` is selected but `model_dir` is unset, or if
/// the model/tokenizer cannot be loaded.
pub fn build_embedder(config: &EmbeddingConfig) -> anyhow::Result<Arc<dyn Embedder>> {
    match config.model {
        EmbeddingModel::Lightweight => Ok(Arc::new(LightweightEmbedder::new(
            super::minilm::HIDDEN_SIZE,
        ))),
        EmbeddingModel::Minilm => {
            let dir = config.model_dir.as_ref().ok_or_else(|| {
                anyhow!("EmbeddingConfig.model_dir must be set for model=`MiniLM`")
            })?;
            let embedder = MinilmEmbedder::new(dir, config.quantization)?;
            Ok(Arc::new(embedder))
        }
    }
}
