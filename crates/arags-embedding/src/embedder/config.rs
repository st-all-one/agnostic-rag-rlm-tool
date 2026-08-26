use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;

use super::minilm::MinilmEmbedder;
use super::ollama::OllamaEmbedder;
use super::{Embedder, LightweightEmbedder};

#[cfg(feature = "llamacpp")]
use super::llama_cpp::LlamaCppEmbedder;

/// Which embedding backend to instantiate.
///
/// `Minilm` is the in-process candle data-plane model; `Ollama` delegates to a
/// local Ollama daemon (e.g. `all-minilm:22m`) for a faster engine while
/// keeping the same 384-dimensional space; `Lightweight` is a deterministic
/// hash fixture for tests and degraded mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbeddingModel {
    /// Native all-`MiniLM`-L6-v2 via candle. Requires model weights on disk.
    #[default]
    Minilm,
    /// Local Ollama daemon over `/api/embed` (e.g. `all-minilm:22m`).
    Ollama,
    /// Lightweight deterministic embedder (no weights, no candle inference).
    Lightweight,
    /// Local `llama.cpp` (GGUF) embedder on the iGPU via Vulkan — daemon-free.
    #[cfg(feature = "llamacpp")]
    LlamaCpp,
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
    /// Base URL of the Ollama daemon (`Ollama` backend), default
    /// `http://localhost:11434`.
    pub ollama_url: Option<String>,
    /// Ollama model name (`Ollama` backend), default `all-minilm:22m`.
    pub ollama_model: Option<String>,
    /// Path to a GGUF model (`LlamaCpp` backend).
    #[cfg(feature = "llamacpp")]
    pub llama_cpp_model: Option<PathBuf>,
    /// Layers to offload to the GPU for the `LlamaCpp` backend (`99` = all,
    /// `0` = CPU only).
    #[cfg(feature = "llamacpp")]
    pub llama_cpp_gpu_layers: u32,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            model: EmbeddingModel::Minilm,
            model_dir: None,
            quantization: Quantization::Int8,
            ollama_url: None,
            ollama_model: None,
            #[cfg(feature = "llamacpp")]
            llama_cpp_model: None,
            #[cfg(feature = "llamacpp")]
            llama_cpp_gpu_layers: 99,
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
            ollama_url: None,
            ollama_model: None,
            #[cfg(feature = "llamacpp")]
            llama_cpp_model: None,
            #[cfg(feature = "llamacpp")]
            llama_cpp_gpu_layers: 99,
        }
    }
}

/// Build an embedder from a configuration.
///
/// Returns a [`LightweightEmbedder`] for the test fixture model, a
/// [`MinilmEmbedder`] (with INT8/f32 quantization) for `MiniLM`, or an
/// [`OllamaEmbedder`] for `Ollama`.
///
/// # Errors
///
/// Returns an error if `MiniLM` is selected but `model_dir` is unset, or if
/// the model/tokenizer/Ollama daemon cannot be loaded.
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
        EmbeddingModel::Ollama => {
            let url = config
                .ollama_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            let model = config
                .ollama_model
                .clone()
                .unwrap_or_else(|| "all-minilm:22m".to_string());
            let embedder = OllamaEmbedder::new(&url, &model)?;
            Ok(Arc::new(embedder))
        }
        #[cfg(feature = "llamacpp")]
        EmbeddingModel::LlamaCpp => {
            let path = config.llama_cpp_model.as_ref().ok_or_else(|| {
                anyhow!("EmbeddingConfig.llama_cpp_model must be set for model=`LlamaCpp`")
            })?;
            let embedder = LlamaCppEmbedder::new(path, config.llama_cpp_gpu_layers, 512)?;
            Ok(Arc::new(embedder))
        }
    }
}
