use super::{Embedder, Embedding, EmbeddingResult};

/// Deterministic hash-based fallback embedder.
///
/// Produces a deterministic pseudo-embedding from the SHA-256 hash of the input.
/// Useful for tests and when no neural model is available. The embedding is
/// unit-normalized and its values are in [0, 1].
pub struct FallbackEmbedder {
    dims: usize,
}

impl FallbackEmbedder {
    #[must_use]
    pub fn new(dims: usize) -> Self {
        Self { dims }
    }

    /// Compute a deterministic embedding from text hash.
    ///
    /// The result is a unit-normalized vector of dimension `dims`.
    #[must_use]
    pub fn embed_deterministic(text: &str, dims: usize) -> Embedding {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        let hash = hasher.finalize();

        let mut embedding = vec![0.0_f32; dims];
        for (i, byte) in hash.iter().enumerate() {
            embedding[i % dims] += f32::from(*byte);
        }

        // Normalize to unit vector
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut embedding {
                *x /= norm;
            }
        }

        embedding
    }
}

impl Embedder for FallbackEmbedder {
    /// # Errors
    ///
    /// This implementation never returns an error.
    fn embed(&self, text: &str) -> EmbeddingResult<Embedding> {
        Ok(Self::embed_deterministic(text, self.dims))
    }

    /// # Errors
    ///
    /// This implementation never returns an error.
    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Embedding>> {
        let _timer = crate::Timer::new("fallback_embed_batch");
        Ok(texts
            .iter()
            .map(|t| Self::embed_deterministic(t, self.dims))
            .collect())
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn name(&self) -> &'static str {
        "fallback-hash"
    }
}
