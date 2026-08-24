#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::float_cmp,
    clippy::useless_vec
)]

use arlm_embedding::embedder::batch::BatchEmbedder;
use arlm_embedding::embedder::cache::EmbeddingCache;
use arlm_embedding::embedder::config::{
    EmbeddingConfig, EmbeddingModel, Quantization, build_embedder,
};
use arlm_embedding::embedder::fallback::FallbackEmbedder;
use arlm_embedding::embedder::lightweight::LightweightEmbedder;
use arlm_embedding::embedder::{Embedder, EmbeddingError, OwnedFile, matryoshka_truncate};

// ---- embedder/mod.rs ----

#[test]
fn test_owned_file_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("test.rs");
    std::fs::write(&file_path, "fn main() {}").expect("write");

    let owned = OwnedFile::new(&file_path).expect("OwnedFile::new");
    assert_eq!(owned.content(), "fn main() {}");
    assert_eq!(owned.language_hint(), "rust");
    assert_eq!(owned.path(), file_path.as_path());
}

#[test]
fn test_owned_file_not_utf8() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("binary.bin");
    std::fs::write(&file_path, [0xFF, 0xFE, 0x00]).expect("write");

    let result = OwnedFile::new(&file_path);
    assert!(result.is_err());
}

#[test]
fn test_embedder_trait_name() {
    let embedder = FallbackEmbedder::new(128);
    assert_eq!(embedder.name(), "fallback-hash");
}

// ---- embedder/batch.rs ----

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

    let texts = vec!["hello", "world"];
    let refs: Vec<&str> = texts.iter().map(|s| &**s).collect();
    let e1 = batch.embed_with_cache(&refs).expect("batch1");
    assert_eq!(e1.len(), 2);

    let e2 = batch.embed_with_cache(&refs).expect("batch2");
    assert_eq!(e1, e2);
}

#[test]
fn test_batch_embedder_partial_cache() {
    let embedder = FallbackEmbedder::new(16);
    let cache = EmbeddingCache::in_memory(16).expect("cache");

    let emb = FallbackEmbedder::embed_deterministic("hello", 16);
    cache.put("hello", &emb).expect("put");

    let batch = BatchEmbedder::new(Box::new(embedder), Some(cache), 2);
    let texts = vec!["hello", "world"];
    let refs: Vec<&str> = texts.iter().map(|s| &**s).collect();
    let embeddings = batch.embed_with_cache(&refs).expect("batch");
    assert_eq!(embeddings.len(), 2);
    assert_eq!(embeddings[0], emb);
}

// ---- embedder/cache.rs ----

#[test]
fn test_cache_put_get() {
    let cache = EmbeddingCache::in_memory(4).expect("cache");
    let emb = vec![0.1, 0.2, 0.3, 0.4];
    cache.put("hello", &emb).expect("put");
    let got = cache.get("hello").expect("get").expect("found");
    assert_eq!(got, emb);
}

#[test]
fn test_cache_miss() {
    let cache = EmbeddingCache::in_memory(4).expect("cache");
    let result = cache.get("nonexistent").expect("get");
    assert!(result.is_none());
}

#[test]
fn test_cache_contains() {
    let cache = EmbeddingCache::in_memory(4).expect("cache");
    assert!(!cache.contains("hello"));
    cache.put("hello", &vec![1.0, 2.0, 3.0, 4.0]).expect("put");
    assert!(cache.contains("hello"));
}

#[test]
fn test_cache_len() {
    let cache = EmbeddingCache::in_memory(4).expect("cache");
    assert_eq!(cache.len(), 0);
    cache.put("a", &vec![1.0, 2.0, 3.0, 4.0]).expect("put");
    cache.put("b", &vec![5.0, 6.0, 7.0, 8.0]).expect("put");
    assert_eq!(cache.len(), 2);
}

#[test]
fn test_cache_clear() {
    let cache = EmbeddingCache::in_memory(4).expect("cache");
    cache.put("a", &vec![1.0, 2.0, 3.0, 4.0]).expect("put");
    cache.clear().expect("clear");
    assert!(cache.is_empty());
}

#[test]
fn test_cache_dimension_mismatch() {
    let cache = EmbeddingCache::in_memory(4).expect("cache");
    let result = cache.put("a", &vec![1.0, 2.0]);
    assert!(result.is_err());
}

#[test]
fn test_content_hash_deterministic() {
    let h1 = EmbeddingCache::content_hash("hello");
    let h2 = EmbeddingCache::content_hash("hello");
    assert_eq!(h1, h2);
}

#[test]
fn test_content_hash_different() {
    let h1 = EmbeddingCache::content_hash("hello");
    let h2 = EmbeddingCache::content_hash("world");
    assert_ne!(h1, h2);
}

#[test]
fn test_cache_overwrite() {
    let cache = EmbeddingCache::in_memory(4).expect("cache");
    cache.put("a", &vec![1.0, 2.0, 3.0, 4.0]).expect("put");
    cache
        .put("a", &vec![5.0, 6.0, 7.0, 8.0])
        .expect("overwrite");
    let got = cache.get("a").expect("get").expect("found");
    assert_eq!(got, vec![5.0, 6.0, 7.0, 8.0]);
}

// ---- embedder/config.rs ----

#[test]
fn test_default_is_minilm_int8() {
    let cfg = EmbeddingConfig::default();
    assert_eq!(cfg.model, EmbeddingModel::Minilm);
    assert_eq!(cfg.quantization, Quantization::Int8);
    assert!(cfg.model_dir.is_none());
}

#[test]
fn test_for_tests_is_lightweight() {
    let cfg = EmbeddingConfig::for_tests();
    assert_eq!(cfg.model, EmbeddingModel::Lightweight);
    assert_eq!(cfg.quantization, Quantization::None);
}

#[test]
fn test_quantization_ggml_dtype() {
    assert_eq!(Quantization::None.ggml_dtype(), None);
    assert_eq!(
        Quantization::Int8.ggml_dtype(),
        Some(candle_core::quantized::GgmlDType::Q8_0)
    );
}

#[test]
fn test_lightweight_builds_without_weights() {
    let cfg = EmbeddingConfig::for_tests();
    let embedder = build_embedder(&cfg).expect("build lightweight");
    assert_eq!(embedder.name(), "lightweight");
    assert_eq!(embedder.dimensions(), 384);
}

#[test]
fn test_minilm_build_requires_model_dir() {
    let cfg = EmbeddingConfig::default();
    let result = build_embedder(&cfg);
    assert!(result.is_err());
}

// ---- embedder/fallback.rs ----

#[test]
fn test_fallback_deterministic() {
    let a = FallbackEmbedder::embed_deterministic("hello", 128);
    let b = FallbackEmbedder::embed_deterministic("hello", 128);
    assert_eq!(a.len(), 128);
    assert_eq!(a, b);
}

#[test]
fn test_fallback_different_inputs() {
    let a = FallbackEmbedder::embed_deterministic("hello", 128);
    let b = FallbackEmbedder::embed_deterministic("world", 128);
    assert_ne!(a, b);
}

#[test]
fn test_fallback_normalized() {
    let emb = FallbackEmbedder::embed_deterministic("test", 64);
    let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-6);
}

#[test]
fn test_fallback_batch() {
    let embedder = FallbackEmbedder::new(32);
    let texts = vec!["a", "b", "c"];
    let refs: Vec<&str> = texts.iter().map(|s| &**s).collect();
    let embeddings = embedder.embed_batch(&refs).expect("batch");
    assert_eq!(embeddings.len(), 3);
    for emb in &embeddings {
        assert_eq!(emb.len(), 32);
    }
}

#[test]
fn test_fallback_single_embed() {
    let embedder = FallbackEmbedder::new(16);
    let emb = embedder.embed("test").expect("embed");
    assert_eq!(emb.len(), 16);
}

// ---- embedder/lightweight.rs ----

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

// ---- matryoshka ----

#[test]
fn test_matryoshka_truncate_shorter() {
    let emb = vec![1.0_f32, 2.0, 3.0];
    let out = matryoshka_truncate(&emb, 5);
    assert_eq!(out.len(), 5);
    assert_eq!(out[..3], [1.0, 2.0, 3.0]);
    assert_eq!(out[3], 0.0);
    assert_eq!(out[4], 0.0);
}

#[test]
fn test_matryoshka_truncate_equal() {
    let emb = vec![1.0_f32, 2.0, 3.0];
    let out = matryoshka_truncate(&emb, 3);
    assert_eq!(out, emb);
}

#[test]
fn test_matryoshka_truncate_longer() {
    let emb = vec![1.0_f32, 2.0, 3.0, 4.0, 5.0];
    let out = matryoshka_truncate(&emb, 2);
    assert_eq!(out, vec![1.0, 2.0]);
}

#[test]
fn test_embedding_error_display() {
    let _ = EmbeddingError::CacheMiss;
    let msg = format!("{}", EmbeddingError::ModelNotLoaded("x".into()));
    assert!(msg.contains('x'));
}
