use super::{Embedder, Embedding, EmbeddingError, EmbeddingResult};
use crate::embedder::cache::EmbeddingCache;

/// Batch embedding with cache integration.
///
/// Processes a list of texts, checking the cache before calling the model.
/// Uncached texts are batched together for efficient inference, then stored
/// in the cache for future lookups.
pub struct BatchEmbedder {
    inner: Box<dyn Embedder>,
    cache: Option<EmbeddingCache>,
    batch_size: usize,
}

impl BatchEmbedder {
    /// Create a batch embedder with optional cache.
    #[must_use]
    pub fn new(inner: Box<dyn Embedder>, cache: Option<EmbeddingCache>, batch_size: usize) -> Self {
        Self {
            inner,
            cache,
            batch_size,
        }
    }

    /// Embed a list of texts, using the cache where possible.
    ///
    /// Returns one embedding per input text, in the same order.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying embedder or cache fails.
    ///
    /// # Panics
    ///
    /// Panics if internal bookkeeping is inconsistent (should not happen in practice).
    pub fn embed_with_cache(&self, texts: &[&str]) -> EmbeddingResult<Vec<Embedding>> {
        let _timer = crate::Timer::new("batch_embed_with_cache");

        let mut results = Vec::with_capacity(texts.len());
        let mut uncached_indices = Vec::with_capacity(texts.len());
        let mut uncached_texts = Vec::with_capacity(texts.len());

        // Phase 1: check cache
        for (i, text) in texts.iter().enumerate() {
            if let Some(ref cache) = self.cache {
                if let Some(emb) = cache.get(text)? {
                    results.push(Some(emb));
                } else {
                    results.push(None);
                    uncached_indices.push(i);
                    uncached_texts.push(*text);
                }
            } else {
                results.push(None);
                uncached_indices.push(i);
                uncached_texts.push(*text);
            }
        }

        // Phase 2: batch embed uncached texts
        if !uncached_texts.is_empty() {
            let _timer = crate::Timer::new("batch_embed_model_inference");

            let mut all_embeddings = Vec::with_capacity(uncached_texts.len());
            for chunk in uncached_texts.chunks(self.batch_size) {
                let embeddings = self.inner.embed_batch(chunk)?;
                all_embeddings.extend(embeddings);
            }

            // Phase 3: fill results and store in cache
            for (idx_in_uncached, &orig_idx) in uncached_indices.iter().enumerate() {
                let emb = &all_embeddings[idx_in_uncached];
                results[orig_idx] = Some(emb.clone());

                if let Some(ref cache) = self.cache {
                    cache.put(texts[orig_idx], emb)?;
                }
            }
        }

        // SAFETY: all results are filled by the loops above
        results
            .into_iter()
            .map(|opt| {
                opt.ok_or_else(|| {
                    EmbeddingError::ModelNotLoaded("result slot not filled".into())
                })
            })
            .collect::<EmbeddingResult<Vec<_>>>()
    }

    /// The underlying embedder.
    #[must_use]
    pub fn inner(&self) -> &dyn Embedder {
        self.inner.as_ref()
    }
}

impl Embedder for BatchEmbedder {
    /// # Errors
    ///
    /// Returns an error if the underlying embedder fails.
    fn embed(&self, text: &str) -> EmbeddingResult<Embedding> {
        let texts = [text];
        let mut result = self.embed_with_cache(&texts)?;
        result
            .pop()
            .ok_or_else(|| EmbeddingError::ModelNotLoaded("empty batch result".into()))
    }

    /// # Errors
    ///
    /// Returns an error if the underlying embedder fails.
    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Embedding>> {
        self.embed_with_cache(texts)
    }

    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::fallback::FallbackEmbedder;

    #[test]
    fn test_batch_embedder_no_cache() {
        let embedder = FallbackEmbedder::new(32);
        let batch = BatchEmbedder::new(Box::new(embedder), None, 2);
        let texts = vec!["hello", "world", "foo"];
        let refs: Vec<&str> = texts.iter().map(|s| &**s).collect();
        let embeddings = batch.embed_with_cache(&refs).expect("batch");
        assert_eq!(embeddings.len(), 3);
        for emb in &embeddings {
            assert_eq!(emb.len(), 32);
        }
    }

    #[test]
    fn test_batch_embedder_with_cache() {
        let embedder = FallbackEmbedder::new(16);
        let cache = EmbeddingCache::in_memory(16).expect("cache");
        let batch = BatchEmbedder::new(Box::new(embedder), Some(cache), 2);

        // First call: all uncached
        let texts = vec!["hello", "world"];
        let refs: Vec<&str> = texts.iter().map(|s| &**s).collect();
        let e1 = batch.embed_with_cache(&refs).expect("batch1");
        assert_eq!(e1.len(), 2);

        // Second call: all cached
        let e2 = batch.embed_with_cache(&refs).expect("batch2");
        assert_eq!(e1, e2);
    }

    #[test]
    fn test_batch_embedder_partial_cache() {
        let embedder = FallbackEmbedder::new(16);
        let cache = EmbeddingCache::in_memory(16).expect("cache");

        // Pre-populate cache for "hello"
        let emb = FallbackEmbedder::embed_deterministic("hello", 16);
        cache.put("hello", &emb).expect("put");

        let batch = BatchEmbedder::new(Box::new(embedder), Some(cache), 2);
        let texts = vec!["hello", "world"];
        let refs: Vec<&str> = texts.iter().map(|s| &**s).collect();
        let embeddings = batch.embed_with_cache(&refs).expect("batch");
        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0], emb);
    }
}
