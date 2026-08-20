use super::{Embedder, Embedding, EmbeddingResult};

/// Deterministic, weight-free embedder for tests and fast paths.
///
/// Produces a stable pseudo-embedding from the SHA-256 hash of the input by
/// expanding the hash seed through an xorshift PRNG, then L2-normalizing the
/// result. No candle inference, no model weights. Dimensionality is fixed at
/// construction time.
pub struct LightweightEmbedder {
    dims: usize,
}

impl LightweightEmbedder {
    /// Create a lightweight embedder producing `dims`-dimensional vectors.
    #[must_use]
    pub fn new(dims: usize) -> Self {
        Self { dims: dims.max(1) }
    }

    /// Deterministic embedding: hash text -> xorshift expansion -> L2 normalize.
    #[must_use]
    pub fn embed_deterministic(text: &str, dims: usize) -> Embedding {
        use sha2::{Digest, Sha256};

        let hash = Sha256::new().chain_update(text.as_bytes()).finalize();
        let mut seed = u64::from_le_bytes(hash[..8].try_into().unwrap_or([0u8; 8]));
        if seed == 0 {
            seed = 0x9E37_79B9_7F4A_7C15;
        }
        // Collapse to a 32-bit state for the xorshift PRNG.
        #[allow(clippy::cast_possible_truncation)]
        let mut state = (seed ^ (seed >> 32)) as u32;
        if state == 0 {
            state = 0x7F4A_7C15;
        }

        // IEEE-754 trick: build an f32 in [1.0, 2.0) from the top 23 bits of
        // state, then map to [-1.0, 1.0). No float casts, fully deterministic.
        let mut vec = vec![0.0_f32; dims];
        for slot in &mut vec {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let mantissa = state & 0x7F_FF_FF;
            let f = f32::from_bits(0x3F80_0000 | mantissa) - 1.0;
            *slot = f * 2.0 - 1.0;
        }

        normalize(&mut vec);
        vec
    }
}

/// L2-normalize a vector in place.
fn normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

impl Embedder for LightweightEmbedder {
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
        let _timer = crate::Timer::new("lightweight_embed_batch");
        Ok(texts
            .iter()
            .map(|t| Self::embed_deterministic(t, self.dims))
            .collect())
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn name(&self) -> &'static str {
        "lightweight"
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic, clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn test_lightweight_deterministic_same() {
        let a = LightweightEmbedder::embed_deterministic("hello", 256);
        let b = LightweightEmbedder::embed_deterministic("hello", 256);
        assert_eq!(a.len(), 256);
        assert_eq!(a, b);
    }

    #[test]
    fn test_lightweight_deterministic_different() {
        let a = LightweightEmbedder::embed_deterministic("hello", 256);
        let b = LightweightEmbedder::embed_deterministic("world", 256);
        assert_ne!(a, b);
    }

    #[test]
    fn test_lightweight_normalized() {
        let emb = LightweightEmbedder::embed_deterministic("test", 128);
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "norm = {norm}");
    }

    #[test]
    fn test_lightweight_batch() {
        let embedder = LightweightEmbedder::new(64);
        let texts = ["a", "b", "c"];
        let refs: Vec<&str> = texts.iter().map(|s| &**s).collect();
        let embeddings = embedder.embed_batch(&refs).expect("batch");
        assert_eq!(embeddings.len(), 3);
        for emb in &embeddings {
            assert_eq!(emb.len(), 64);
        }
    }

    #[test]
    fn test_lightweight_single() {
        let embedder = LightweightEmbedder::new(32);
        let emb = embedder.embed("x").expect("embed");
        assert_eq!(emb.len(), 32);
    }
}
