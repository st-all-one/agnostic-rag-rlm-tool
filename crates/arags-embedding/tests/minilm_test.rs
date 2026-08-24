//! Tests for the native all-MiniLM-L6-v2 embedder.
//!
//! The full inference path needs the real checkpoint; those tests are
//! `#[ignore]`d and gated on `ARAGS_MINILM_DIR`. Everything else runs on
//! synthetic fixtures.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss, clippy::too_many_lines)]
#![allow(clippy::vec_init_then_push)]

use std::collections::HashMap;
use std::path::Path;

use safetensors::tensor::TensorView;
use tempfile::TempDir;

use arags_embedding::embedder::Embedder;

/// Deterministic pseudo-random f32 fill (small values, no NaN).
fn prng(n: usize, seed: u32) -> Vec<f32> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((x >> 16) & 0xFF) as f32 / 12_800.0 - 0.01
        })
        .collect()
}

const ONES: fn(usize) -> Vec<f32> = |n| vec![1.0_f32; n];
const ZEROS: fn(usize) -> Vec<f32> = |n| vec![0.0_f32; n];

/// Write a tiny valid `MiniLM` checkpoint (hidden 64, 2 layers, 4 heads,
/// intermediate 128) plus a matching `config.json`.
fn toy_checkpoint(dir: &Path) {
    let (hidden, layers, heads, inter) = (64_usize, 2_usize, 4_usize, 128_usize);
    let mut tensors: Vec<(String, Vec<usize>, Vec<f32>)> = Vec::new();

    tensors.push((
        "embeddings.word_embeddings.weight".into(),
        vec![100, hidden],
        prng(100 * hidden, 7),
    ));
    tensors.push((
        "embeddings.position_embeddings.weight".into(),
        vec![32, hidden],
        prng(32 * hidden, 11),
    ));
    tensors.push((
        "embeddings.token_type_embeddings.weight".into(),
        vec![2, hidden],
        prng(2 * hidden, 13),
    ));
    tensors.push((
        "embeddings.LayerNorm.weight".into(),
        vec![hidden],
        ONES(hidden),
    ));
    tensors.push((
        "embeddings.LayerNorm.bias".into(),
        vec![hidden],
        ZEROS(hidden),
    ));

    for i in 0..layers {
        let p = format!("encoder.layer.{i}");
        let seed = 17 + i as u32;
        for proj in [
            "attention.self.query",
            "attention.self.key",
            "attention.self.value",
            "attention.output.dense",
        ] {
            tensors.push((
                format!("{p}.{proj}.weight"),
                vec![hidden, hidden],
                prng(hidden * hidden, seed),
            ));
            tensors.push((format!("{p}.{proj}.bias"), vec![hidden], ZEROS(hidden)));
        }
        tensors.push((
            format!("{p}.attention.output.LayerNorm.weight"),
            vec![hidden],
            ONES(hidden),
        ));
        tensors.push((
            format!("{p}.attention.output.LayerNorm.bias"),
            vec![hidden],
            ZEROS(hidden),
        ));
        tensors.push((
            format!("{p}.intermediate.dense.weight"),
            vec![inter, hidden],
            prng(inter * hidden, seed + 5),
        ));
        tensors.push((
            format!("{p}.intermediate.dense.bias"),
            vec![inter],
            ZEROS(inter),
        ));
        tensors.push((
            format!("{p}.output.dense.weight"),
            vec![hidden, inter],
            prng(hidden * inter, seed + 9),
        ));
        tensors.push((
            format!("{p}.output.dense.bias"),
            vec![hidden],
            ZEROS(hidden),
        ));
        tensors.push((
            format!("{p}.output.LayerNorm.weight"),
            vec![hidden],
            ONES(hidden),
        ));
        tensors.push((
            format!("{p}.output.LayerNorm.bias"),
            vec![hidden],
            ZEROS(hidden),
        ));
    }

    // Materialize the little-endian payloads first; TensorView borrows them.
    let mut payload: Vec<(String, Vec<usize>, Vec<u8>)> = Vec::with_capacity(tensors.len());
    for (name, shape, vals) in &tensors {
        let mut bytes = Vec::with_capacity(vals.len() * 4);
        for v in vals {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        payload.push((name.clone(), shape.clone(), bytes));
    }
    let views: HashMap<String, TensorView> = payload
        .iter()
        .map(|(name, shape, bytes)| {
            (
                name.clone(),
                TensorView::new(safetensors::Dtype::F32, shape.clone(), bytes)
                    .expect("view from shaped bytes"),
            )
        })
        .collect();
    let bytes = safetensors::serialize(views, &None).expect("serialize synthetic checkpoint");
    std::fs::write(dir.join("model.safetensors"), bytes).expect("write safetensors");

    let config = format!(
        "{{\"hidden_size\": {hidden}, \"num_hidden_layers\": {layers}, \"num_attention_heads\": {heads}, \"intermediate_size\": {inter}}}"
    );
    std::fs::write(dir.join("config.json"), config).expect("write config");
}

#[test]
fn test_minilm_missing_model_file() {
    let dir = TempDir::new().unwrap();
    let err = arags_embedding::embedder::MinilmEmbedder::new(
        dir.path(),
        arags_embedding::embedder::config::Quantization::None,
    )
    .unwrap_err();
    assert!(err.to_string().contains("model.safetensors"));
}

#[test]
fn test_minilm_missing_tokenizer() {
    let dir = TempDir::new().unwrap();
    toy_checkpoint(dir.path());
    let err = arags_embedding::embedder::MinilmEmbedder::new(
        dir.path(),
        arags_embedding::embedder::config::Quantization::None,
    )
    .unwrap_err();
    assert!(err.to_string().contains("tokenizer"));
}

#[test]
fn test_quantization_parse() {
    use arags_embedding::embedder::config::Quantization;
    assert_eq!(Quantization::parse(""), Quantization::Int8);
    assert_eq!(Quantization::parse("int8"), Quantization::Int8);
    assert_eq!(Quantization::parse(" NONE "), Quantization::None);
    assert_eq!(Quantization::default(), Quantization::Int8);
    assert!(Quantization::Int8.ggml_dtype().is_some());
    assert_eq!(Quantization::None.ggml_dtype(), None);
}

/// Full inference path against real `sentence-transformers/all-MiniLM-L6-v2`
/// weights. Run with:
/// `ARAGS_MINILM_DIR=/models/all-MiniLM-L6-v2 cargo test -- --ignored`
#[test]
#[ignore = "requires real all-MiniLM-L6-v2 weights"]
fn test_minilm_real_weights_semantics() {
    let Some(dir) = std::env::var_os("ARAGS_MINILM_DIR").map(std::path::PathBuf::from) else {
        panic!("ARAGS_MINILM_DIR not set");
    };
    let embedder = arags_embedding::embedder::MinilmEmbedder::new(
        &dir,
        arags_embedding::embedder::config::Quantization::Int8,
    )
    .unwrap();

    assert_eq!(embedder.dimensions(), 384);
    assert_eq!(embedder.name(), "all-MiniLM-L6-v2");

    let code = "fn main() { println!(\"hello world\"); }";
    let doc = "The Eiffel tower is in Paris, France.";
    let e_code = embedder.embed(code).unwrap();
    let e_doc = embedder.embed(doc).unwrap();
    assert_eq!(e_code.len(), 384);

    for e in [&e_code, &e_doc] {
        let norm: f32 = e.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "expected unit norm, got {norm}");
    }

    // Batch preserves order and matches single calls within tolerance.
    let batch = embedder.embed_batch(&[code, doc]).unwrap();
    assert_eq!(batch.len(), 2);
    let dot: f32 = e_code.iter().zip(&batch[0]).map(|(a, b)| a * b).sum();
    assert!(dot > 0.999, "batch[0] diverges from embed(): {dot}");

    // Semantics: same-topic pair beats cross-topic similarity.
    let sim = |a: &[f32], b: &[f32]| -> f32 { a.iter().zip(b).map(|(x, y)| x * y).sum() };
    let paris = embedder.embed("Paris is the capital of France.").unwrap();
    assert!(
        sim(&e_doc, &paris) > sim(&e_code, &paris),
        "semantic ordering violated"
    );
}
