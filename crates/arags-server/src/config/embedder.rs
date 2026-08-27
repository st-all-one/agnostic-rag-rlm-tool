//! Server-side chunking + embedding configuration.

use serde::Deserialize;

/// Server-side chunking + embedding parameters.
///
/// The embedding model is **fixed**: native all-`MiniLM`-L6-v2 via candle,
/// in-process. The server chunks raw file content it receives over gRPC using
/// `max_tokens`/`overlap_tokens`, then embeds and stores vectors (384 dims).
/// All of this is configured exclusively here — the client has no data config.
#[derive(Debug, Clone, Deserialize)]
pub struct EmbedderConfig {
    /// Backend selector. Default (no `kind`): candle `Minilm` when its weights
    /// are present, otherwise the hash fallback — this keeps the shipped binary
    /// portable (no C++/Vulkan toolchain). To accelerate on a GPU with zero
    /// external daemon, build the server with `--features llamacpp-vulkan`,
    /// point `llama_cpp_model` at a GGUF, and set `kind = "llamacpp"`. The
    /// simpler GPU path is `kind = "ollama"` (local Ollama daemon, e.g.
    /// `all-minilm:22m`), which needs no special build. `lightweight` is a test
    /// fixture.
    #[serde(default)]
    pub kind: Option<String>,

    /// Checkpoint directory (`model.safetensors` + `tokenizer.json`, as
    /// shipped by `sentence-transformers/all-MiniLM-L6-v2`). Without weights
    /// the server degrades to a hash embedder (no semantic search).
    #[serde(default)]
    pub model_dir: Option<PathBuf>,

    /// Base URL of the Ollama daemon (`kind = "ollama"`).
    #[serde(default)]
    pub ollama_url: Option<String>,

    /// Ollama model name (`kind = "ollama"`).
    #[serde(default)]
    pub ollama_model: Option<String>,

    /// Path to a GGUF model (`kind = "llamacpp"`). Runs in-process via
    /// `llama.cpp` + Vulkan on the iGPU — no external daemon required.
    #[serde(default)]
    pub llama_cpp_model: Option<PathBuf>,

    /// Layers to offload to the GPU for `kind = "llamacpp"` (`99` = all,
    /// `0` = CPU only).
    #[serde(default)]
    pub llama_cpp_gpu_layers: Option<u32>,

    /// Weight quantization: `int8` (default, best speed/memory/quality
    /// balance) or `none` (f32).
    #[serde(default)]
    pub quantization: Option<String>,

    /// Chunks per embedding request.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    /// Target chunk size in tokens (server chunks raw file content it
    /// receives over gRPC).
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// Overlap between adjacent chunks in tokens.
    #[serde(default = "default_overlap_tokens")]
    pub overlap_tokens: usize,
    /// Whether to keep the embedder's in-memory vector cache warm.
    #[serde(default = "default_cache_enabled")]
    pub cache: bool,
}

use std::path::PathBuf;

pub(crate) fn default_batch_size() -> usize {
    32
}

pub(crate) fn default_max_tokens() -> usize {
    512
}

fn default_overlap_tokens() -> usize {
    64
}

fn default_cache_enabled() -> bool {
    true
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            kind: None,
            model_dir: None,
            ollama_url: None,
            ollama_model: None,
            llama_cpp_model: None,
            llama_cpp_gpu_layers: None,
            batch_size: default_batch_size(),
            quantization: None,
            max_tokens: default_max_tokens(),
            overlap_tokens: default_overlap_tokens(),
            cache: default_cache_enabled(),
        }
    }
}

impl EmbedderConfig {
    /// The configured weight quantization (INT8 by default).
    #[must_use]
    pub fn resolved_quantization(&self) -> arags_embedding::embedder::config::Quantization {
        self.quantization.as_deref().map_or(
            arags_embedding::embedder::config::Quantization::Int8,
            arags_embedding::embedder::config::Quantization::parse,
        )
    }
}
