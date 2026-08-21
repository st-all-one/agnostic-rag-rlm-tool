//! Public BGE-M3 embedder: tokenization, mean pooling, L2 normalization.

use std::path::Path;
use std::sync::Arc;

use candle_core::{Device, Tensor};
use tokenizers::Tokenizer;

use super::super::config::EmbeddingConfig;
use super::super::{Embedder, Embedding, EmbeddingError, EmbeddingResult};
use super::model::BgeM3Model;
use super::ops::apply_matryoshka;

/// BGE-M3 embedding model backed by candle.
///
/// Loads FP32 weights from a safetensors file and runs a
/// transformer encoder with mean pooling + L2 normalization.
pub struct BgeM3Embedder {
    model: Arc<BgeM3Model>,
    tokenizer: Arc<Tokenizer>,
    device: Device,
    dims: usize,
    matryoshka_dims: Option<usize>,
}

impl BgeM3Embedder {
    /// Create a new BGE-M3 embedder.
    ///
    /// # Arguments
    ///
    /// * `model_dir` - Directory containing `model.safetensors` and `tokenizer.json`.
    /// * `dims` - Embedding dimensionality (BGE-M3 default: 1024).
    ///
    /// # Errors
    ///
    /// Returns an error if the model or tokenizer cannot be loaded.
    pub fn new(model_dir: &Path, dims: usize) -> EmbeddingResult<Self> {
        let device = Device::Cpu;

        let tokenizer_path = model_dir.join("tokenizer.json");
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| EmbeddingError::Tokenizer(format!("failed to load tokenizer: {e}")))?;

        let model = BgeM3Model::load(model_dir, dims, &device, None)?;

        tracing::info!(
            model_dir = %model_dir.display(),
            dims = dims,
            "loaded BGE-M3 embedder"
        );

        Ok(Self {
            model: Arc::new(model),
            tokenizer: Arc::new(tokenizer),
            device,
            dims,
            matryoshka_dims: None,
        })
    }

    /// Create a BGE-M3 embedder driven by an [`EmbeddingConfig`].
    ///
    /// Applies the configured quantization (INT8/INT4 via `QMatMul`, or f32 by
    /// default) and Matryoshka truncation (`matryoshka_dims`).
    ///
    /// # Errors
    ///
    /// Returns an error if the model or tokenizer cannot be loaded.
    pub fn new_with_config(model_dir: &Path, config: &EmbeddingConfig) -> EmbeddingResult<Self> {
        let device = Device::Cpu;

        let tokenizer_path = model_dir.join("tokenizer.json");
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| EmbeddingError::Tokenizer(format!("failed to load tokenizer: {e}")))?;

        let model = BgeM3Model::load(
            model_dir,
            config.dims,
            &device,
            config.quantization.ggml_dtype(),
        )?;

        tracing::info!(
            model_dir = %model_dir.display(),
            dims = config.dims,
            quantization = ?config.quantization,
            matryoshka_dims = ?config.matryoshka_dims,
            "loaded BGE-M3 embedder (config)"
        );

        Ok(Self {
            model: Arc::new(model),
            tokenizer: Arc::new(tokenizer),
            device,
            dims: config.dims,
            matryoshka_dims: config.matryoshka_dims,
        })
    }

    /// Tokenize and prepare model inputs for a batch of texts.
    fn prepare_inputs(&self, texts: &[&str]) -> EmbeddingResult<(Tensor, Tensor)> {
        let encodings: Vec<_> = texts
            .iter()
            .map(|t| {
                self.tokenizer
                    .encode(*t, true)
                    .map_err(|e| EmbeddingError::Tokenizer(format!("encode failed: {e}")))
            })
            .collect::<EmbeddingResult<Vec<_>>>()?;

        let max_len = encodings
            .iter()
            .map(|e| e.get_ids().len())
            .min()
            .unwrap_or(1)
            .min(512);

        let mut input_ids = Vec::with_capacity(encodings.len());
        let mut attention_masks = Vec::with_capacity(encodings.len());

        for enc in &encodings {
            let mut ids = enc.get_ids().to_vec();
            let mut mask = enc.get_attention_mask().to_vec();
            ids.truncate(max_len);
            mask.truncate(max_len);
            ids.resize(max_len, 0);
            mask.resize(max_len, 0);
            input_ids.push(ids);
            attention_masks.push(mask);
        }

        let input_ids = Tensor::new(input_ids, &self.device)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let attention_mask = Tensor::new(attention_masks, &self.device)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        Ok((input_ids, attention_mask))
    }

    /// Apply mean pooling to the model output, then L2-normalize.
    fn mean_pool(output: &Tensor, attention_mask: &Tensor) -> EmbeddingResult<Tensor> {
        let mask_f32 = attention_mask
            .to_dtype(candle_core::DType::F32)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let mask_3d = mask_f32
            .unsqueeze(2)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let masked = output
            .broadcast_mul(&mask_3d)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let summed = masked
            .sum(candle_core::D::Minus2)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let mask_sum = mask_f32
            .sum(candle_core::D::Minus1)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let mask_sum = mask_sum
            .clamp(1e-9, f32::INFINITY)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let mask_sum_2d = mask_sum
            .unsqueeze(1)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let pooled = summed
            .broadcast_div(&mask_sum_2d)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let norm = pooled
            .sqr()
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .sum(candle_core::D::Minus1)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .sqrt()
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .clamp(1e-12, f32::INFINITY)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let norm_2d = norm
            .unsqueeze(1)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        pooled
            .broadcast_div(&norm_2d)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))
    }
}

impl Embedder for BgeM3Embedder {
    /// # Errors
    ///
    /// Returns an error if tokenization or model inference fails.
    fn embed(&self, text: &str) -> EmbeddingResult<Embedding> {
        let (input_ids, attention_mask) = self.prepare_inputs(&[text])?;
        let output = self.model.forward(&input_ids, &attention_mask)?;
        let embedding = Self::mean_pool(&output, &attention_mask)?;
        let vec = embedding
            .get(0)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .to_vec1::<f32>()
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        Ok(apply_matryoshka(vec, self.matryoshka_dims))
    }

    /// # Errors
    ///
    /// Returns an error if tokenization or model inference fails.
    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Embedding>> {
        let _timer = crate::Timer::new("bge_m3_embed_batch");
        let (input_ids, attention_mask) = self.prepare_inputs(texts)?;
        let output = self.model.forward(&input_ids, &attention_mask)?;
        let embeddings = Self::mean_pool(&output, &attention_mask)?;

        let mut result = Vec::with_capacity(texts.len());
        for i in 0..texts.len() {
            let emb = embeddings
                .get(i)
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
            let vec = emb
                .to_vec1::<f32>()
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
            result.push(apply_matryoshka(vec, self.matryoshka_dims));
        }
        Ok(result)
    }

    fn dimensions(&self) -> usize {
        self.matryoshka_dims.unwrap_or(self.dims)
    }

    fn name(&self) -> &'static str {
        "bge-m3"
    }
}
