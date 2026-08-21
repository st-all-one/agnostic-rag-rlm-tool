use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;

use super::bge_m3::BgeM3Embedder;
use super::{Embedder, LightweightEmbedder, OllamaEmbedder};

/// Which embedding backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingModel {
    /// Full BGE-M3 transformer (candle). Requires model weights on disk.
    BgeM3,
    /// Lightweight deterministic embedder (no weights, no candle inference).
    Lightweight,
    /// Remote embedding via an Ollama `/api/embed` endpoint (e.g.
    /// `nomic-embed-text-v2-moe`). Runs on CPU with negligible latency.
    Ollama,
}

/// Weight quantization applied to the BGE-M3 projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quantization {
    /// Full-precision (f32) linear projections. Default.
    None,
    /// INT8 quantization (GGML `Q8_0`).
    Int8,
    /// INT4 quantization (GGML `Q4_0`).
    Int4,
}

impl Quantization {
    /// Map to the corresponding candle GGML dtype, if any.
    #[must_use]
    pub fn ggml_dtype(self) -> Option<candle_core::quantized::GgmlDType> {
        match self {
            Quantization::None => None,
            Quantization::Int8 => Some(candle_core::quantized::GgmlDType::Q8_0),
            Quantization::Int4 => Some(candle_core::quantized::GgmlDType::Q4_0),
        }
    }
}

/// Configuration controlling which embedder is built and how.
///
/// The three fields required by the public config contract are
/// `model`, `quantization`, and `matryoshka_dims`. `model_dir` and `dims`
/// are auxiliary and only used by the BGE-M3 backend.
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    /// The backend to instantiate.
    pub model: EmbeddingModel,
    /// Quantization applied to BGE-M3 projections.
    pub quantization: Quantization,
    /// If `Some(d)`, embeddings are truncated (or zero-padded) to `d` dims.
    pub matryoshka_dims: Option<usize>,
    /// Directory containing `model.safetensors` + `tokenizer.json` (BGE-M3).
    pub model_dir: Option<PathBuf>,
    /// Base embedding dimensionality of the underlying model (BGE-M3 = 1024).
    pub dims: usize,
    /// Ollama base URL (e.g. `http://localhost:11434`). Used by `Ollama`.
    pub ollama_url: Option<String>,
    /// Ollama model name (e.g. `nomic-embed-text-v2-moe`). Used by `Ollama`.
    pub ollama_model: Option<String>,
    /// Task prefix prepended to inputs for the Ollama model (e.g.
    /// `search_document: ` for documents, `search_query: ` for queries).
    pub ollama_prefix: Option<String>,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        // REAL usage default: full BGE-M3, matryoshka 512.
        Self {
            model: EmbeddingModel::BgeM3,
            quantization: Quantization::None,
            matryoshka_dims: Some(512),
            model_dir: None,
            dims: 1024,
            ollama_url: None,
            ollama_model: None,
            ollama_prefix: None,
        }
    }
}

impl EmbeddingConfig {
    /// Lightweight, fast, weight-free configuration for tests.
    #[must_use]
    pub fn for_tests() -> Self {
        Self {
            model: EmbeddingModel::Lightweight,
            quantization: Quantization::None,
            matryoshka_dims: Some(256),
            model_dir: None,
            dims: 384,
            ollama_url: None,
            ollama_model: None,
            ollama_prefix: None,
        }
    }
}

/// Build an embedder from a configuration.
///
/// Returns a `LightweightEmbedder` for the lightweight model, or a
/// `BgeM3Embedder` (with quantization/matryoshka applied) for BGE-M3.
///
/// # Errors
///
/// Returns an error if BGE-M3 is selected but `model_dir` is unset, or if the
/// model/tokenizer cannot be loaded.
pub fn build_embedder(config: &EmbeddingConfig) -> anyhow::Result<Arc<dyn Embedder>> {
    match config.model {
        EmbeddingModel::Lightweight => Ok(Arc::new(LightweightEmbedder::new(
            config.matryoshka_dims.unwrap_or(384),
        ))),
        EmbeddingModel::BgeM3 => {
            let dir = config
                .model_dir
                .as_ref()
                .ok_or_else(|| anyhow!("EmbeddingConfig.model_dir must be set for model=BgeM3"))?;
            let embedder = BgeM3Embedder::new_with_config(dir, config)?;
            Ok(Arc::new(embedder))
        }
        EmbeddingModel::Ollama => {
            let url = config
                .ollama_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            let model = config
                .ollama_model
                .clone()
                .ok_or_else(|| anyhow!("embedding.ollama_model must be set for model=Ollama"))?;
            let dims = if config.dims == 0 { 768 } else { config.dims };
            let prefix = config
                .ollama_prefix
                .clone()
                .unwrap_or_else(|| "search_document: ".to_string());
            Ok(Arc::new(OllamaEmbedder::new(url, model, dims, prefix)))
        }
    }
}
