//! Helpers to build the embedding backend and locate the per-buffer vector
//! store used for semantic (BGE-M3) search.
//!
//! Semantic filtering was previously never applied: indexing stored no vectors
//! and the query path was built with `semantic = None` + `query_vector = None`.
//! These helpers close that gap by constructing a real embedder (BGE-M3 when
//! weights are available, otherwise a deterministic lightweight fallback) and a
//! [`VectorStore`] directory scoped per project buffer.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use arlm_embedding::embedder::config::{
    EmbeddingConfig as EmbConfig, EmbeddingModel, Quantization,
};
use arlm_embedding::embedder::{Embedder, build_embedder};
use arlm_storage::VectorStore;

use crate::config::Config;
use crate::util::data_dir;

/// Vector store dimensionality for BGE-M3 / the hash fallback (1024).
pub const VECTOR_DIMS_BGE: usize = 1024;
/// Default dimensionality for Ollama/nomic embeddings (768).
pub const VECTOR_DIMS_OLLAMA: usize = 768;

/// Directory holding the per-buffer vector stores (`~/.arlm/vectors/<buffer_id>`).
#[must_use]
pub fn vector_dir(buffer_id: i64) -> PathBuf {
    data_dir().join("vectors").join(buffer_id.to_string())
}

/// Build an embedder from the resolved CLI [`Config`].
///
/// `prefix` is the Nomic task prefix prepended to every input (e.g.
/// `"search_document: "` when indexing, `"search_query: "` when querying). It
/// is only applied by the Ollama backend; other backends ignore it.
///
/// Resolution:
/// 1. `model = "ollama"` → remote Ollama embedder (`nomic-embed-text-v2-moe`
///    by default, 768-dim). Laptop-friendly: runs on CPU with no weights.
/// 2. `model = "bge-m3"` (or unset with `model_dir` set) → local BGE-M3.
/// 3. Otherwise → deterministic `lightweight` hash embedder (no semantic value,
///    only keeps the pipeline alive).
///
/// If the selected backend fails to construct (e.g. Ollama unreachable), it
/// falls back to `lightweight` rather than aborting.
pub fn build_embedder_from_config(config: &Config, prefix: &str) -> Arc<dyn Embedder> {
    let emb = &config.embedding;
    let dims = emb.dims.unwrap_or(VECTOR_DIMS_BGE);

    if let Some("ollama") = emb.model.as_deref() {
        let url = emb
            .ollama_url
            .clone()
            .unwrap_or_else(|| "http://localhost:11434".to_string());
        let omodel = emb
            .ollama_model
            .clone()
            .unwrap_or_else(|| "nomic-embed-text-v2-moe".to_string());
        let odims = emb.dims.unwrap_or(VECTOR_DIMS_OLLAMA);
        let cfg = EmbConfig {
            model: EmbeddingModel::Ollama,
            quantization: Quantization::None,
            matryoshka_dims: None,
            model_dir: None,
            dims: odims,
            ollama_url: Some(url),
            ollama_model: Some(omodel),
            ollama_prefix: Some(prefix.to_string()),
        };
        match build_embedder(&cfg) {
            Ok(embedder) => {
                tracing::info!(model = "ollama", prefix, "loaded Ollama embedder");
                return embedder;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to load Ollama embedder, falling back to lightweight"
                );
            }
        }
    }

    let want_bge = matches!(emb.model.as_deref(), Some("bge-m3") | None) && emb.model_dir.is_some();

    if want_bge {
        if let Some(dir) = &emb.model_dir {
            let cfg = EmbConfig {
                model: EmbeddingModel::BgeM3,
                quantization: Quantization::None,
                matryoshka_dims: Some(dims),
                model_dir: Some(dir.clone()),
                dims: 1024,
                ollama_url: None,
                ollama_model: None,
                ollama_prefix: None,
            };
            match build_embedder(&cfg) {
                Ok(embedder) => {
                    tracing::info!(model = "bge-m3", dir = %dir.display(), "loaded BGE-M3 embedder");
                    return embedder;
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "failed to load BGE-M3 embedder, falling back to lightweight"
                    );
                }
            }
        }
    }

    let model = emb.model.as_deref().unwrap_or("lightweight");
    tracing::info!(
        model,
        dims,
        "using lightweight embedder for semantic search"
    );
    Arc::new(arlm_embedding::embedder::LightweightEmbedder::new(dims))
}

/// Open (or create) the per-buffer [`VectorStore`] for semantic search at the
/// given embedding dimensionality.
///
/// # Errors
///
/// Returns an error if the underlying `usearch` index cannot be created or
/// restored.
pub async fn open_vector_store(buffer_id: i64, dims: usize) -> Result<Arc<VectorStore>> {
    let dir = vector_dir(buffer_id);
    let store = VectorStore::open_with_dims(&dir, dims)
        .await
        .with_context(|| format!("failed to open vector store at {}", dir.display()))?;
    Ok(Arc::new(store))
}
