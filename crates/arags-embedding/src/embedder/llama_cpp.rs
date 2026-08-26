//! Self-contained `llama.cpp` embedding backend (Vulkan/iGPU, no external daemon).
//!
//! Uses the `llama-cpp-4` crate with the `prebuilt` + `vulkan` features so the
//! native library is downloaded at build time (no C++ toolchain) and runs on the
//! machine's iGPU via Vulkan — covering both AMD and Intel integrated GPUs. This
//! is the daemon-free alternative to [`crate::ollama::OllamaEmbedder`]: same
//! acceleration, single static binary.

use std::path::Path;

use llama_cpp_4::context::params::LlamaContextParams;
use llama_cpp_4::llama_backend::LlamaBackend;
use llama_cpp_4::llama_batch::LlamaBatch;
use llama_cpp_4::model::params::LlamaModelParams;
use llama_cpp_4::model::{AddBos, LlamaModel};

use crate::embedder::{Embedding, EmbeddingError, EmbeddingResult, Embedder};

/// Embedder backed by a local GGUF model loaded through `llama.cpp`.
pub struct LlamaCppEmbedder {
    backend: LlamaBackend,
    model: LlamaModel,
    n_ctx: u32,
}

impl LlamaCppEmbedder {
    /// Load `model_path` (GGUF), set the context window to `n_ctx` tokens, and
    /// offload `n_gpu_layers` layers to the GPU (pass a large number like `99`
    /// to offload everything; `0` forces CPU). `n_ctx` of `0` uses the model
    /// default.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::LlamaCpp`] if the backend fails to initialise
    /// or the GGUF model cannot be loaded.
    pub fn new(model_path: &Path, n_gpu_layers: u32, n_ctx: u32) -> EmbeddingResult<Self> {
        let backend =
            LlamaBackend::init().map_err(|e| EmbeddingError::LlamaCpp(e.to_string()))?;
        let params = LlamaModelParams::default().with_n_gpu_layers(n_gpu_layers);
        let params = std::pin::pin!(params);
        let model = LlamaModel::load_from_file(&backend, model_path, &params)
            .map_err(|e| EmbeddingError::LlamaCpp(e.to_string()))?;
        Ok(Self {
            backend,
            model,
            n_ctx,
        })
    }
}

/// L2-normalise an embedding in place (idempotent; matches the candle path so
/// vector-search similarity stays consistent).
fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

impl Embedder for LlamaCppEmbedder {
    fn embed(&self, text: &str) -> EmbeddingResult<Embedding> {
        let mut v = self.embed_batch(&[text])?;
        v.pop()
            .ok_or_else(|| EmbeddingError::LlamaCpp("empty embed result".into()))
    }

    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Embedding>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let ctx_params = LlamaContextParams::default()
            .with_embeddings(true)
            .with_n_ctx(std::num::NonZeroU32::new(self.n_ctx));
        let mut ctx = self
            .model
            .new_context(&self.backend, ctx_params)
            .map_err(|e| EmbeddingError::LlamaCpp(e.to_string()))?;

        // One decode per text (the llama.cpp embeddings path pools the whole
        // sequence, so a single-sequence batch is the supported shape). The
        // context is created once and reused across the texts in the batch.
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            let toks = self
                .model
                .str_to_token(text, AddBos::Always)
                .map_err(|e| EmbeddingError::LlamaCpp(e.to_string()))?;
            let mut batch = LlamaBatch::new(toks.len(), 1);
            batch
                .add_sequence(&toks, 0, true)
                .map_err(|e| EmbeddingError::LlamaCpp(e.to_string()))?;
            ctx.decode(&mut batch)
                .map_err(|e| EmbeddingError::LlamaCpp(e.to_string()))?;
            let mut v = ctx
                .embeddings_seq_ith(0)
                .map_err(|e| EmbeddingError::LlamaCpp(e.to_string()))?
                .to_vec();
            normalize(&mut v);
            out.push(v);
        }
        Ok(out)
    }

    fn dimensions(&self) -> usize {
        self.model.n_embd().unsigned_abs() as usize
    }

    fn name(&self) -> &'static str {
        "llama-cpp"
    }
}

impl LlamaCppEmbedder {
    /// Number of BPE tokens `text` would be split into (used to keep chunks
    /// within the model's context window before embedding).
    #[must_use]
    pub fn count_tokens(&self, text: &str) -> usize {
        self.model
            .str_to_token(text, AddBos::Always)
            .map_or(0, |v| v.len())
    }
}

#[cfg(all(test, feature = "llamacpp"))]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn gguf_path() -> PathBuf {
        PathBuf::from(std::env::var("ARAGS_TEST_GGUF").expect("set ARAGS_TEST_GGUF"))
    }

    #[test]
    fn llamacpp_loads_and_embeds() {
        let path = gguf_path();
        let embedder =
            LlamaCppEmbedder::new(&path, 0, 512).expect("load gguf (cpu, gpu_layers=0)");
        assert_eq!(embedder.dimensions(), 384, "all-MiniLM-L6-v2 is 384-dim");

        let batch = embedder
            .embed_batch(&["hello world", "the quick brown fox"])
            .expect("embed batch");
        assert_eq!(batch.len(), 2);
        for v in &batch {
            assert_eq!(v.len(), 384);
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-4, "embedding must be L2-normalised");
        }
        // Distinct texts should yield distinct vectors.
        assert_ne!(batch[0], batch[1]);
    }
}
