//! Public `MiniLM` embedder: tokenization, batched inference, mean pooling,
//! L2 normalization.

use std::path::Path;
use std::sync::Arc;

use candle_core::{Device, Tensor};
use tokenizers::Tokenizer;

use super::model::MiniLmModel;
use crate::embedder::config::Quantization;
use crate::embedder::{Embedder, Embedding, EmbeddingError, EmbeddingResult};

/// Maximum sequence length of all-`MiniLM`-L6-v2 (BERT position embeddings).
const MAX_SEQ: usize = 512;

/// all-`MiniLM`-L6-v2 embedder backed by candle (in-process CPU inference).
///
/// Loads canonical `transformers` weights from a local directory and runs the
/// standard sentence-transformers recipe: mean pooling over token states with
/// the attention mask, then L2 normalization.
#[derive(Debug)]
pub struct MinilmEmbedder {
    model: Arc<MiniLmModel>,
    tokenizer: Arc<Tokenizer>,
}

impl MinilmEmbedder {
    /// Create a new `MiniLM` embedder from a checkpoint directory.
    ///
    /// The directory must contain `model.safetensors` + `tokenizer.json`
    /// (the files shipped by `sentence-transformers/all-MiniLM-L6-v2`).
    ///
    /// # Errors
    ///
    /// Returns an error if the model or tokenizer cannot be loaded.
    pub fn new(model_dir: &Path, quantization: Quantization) -> EmbeddingResult<Self> {
        let device = Device::Cpu;

        // The weights are the primary artifact; surface their absence first.
        let model_path = model_dir.join("model.safetensors");
        if !model_path.exists() {
            return Err(EmbeddingError::ModelNotLoaded(format!(
                "model.safetensors not found at {}",
                model_path.display()
            )));
        }

        let tokenizer = Tokenizer::from_file(model_dir.join("tokenizer.json"))
            .map_err(|e| EmbeddingError::Tokenizer(format!("failed to load tokenizer: {e}")))?;

        let model = MiniLmModel::load(model_dir, &device, quantization.ggml_dtype())?;

        tracing::info!(
            model_dir = %model_dir.display(),
            dims = model.hidden_size(),
            ?quantization,
            "loaded minilm embedder"
        );

        Ok(Self {
            model: Arc::new(model),
            tokenizer: Arc::new(tokenizer),
        })
    }

    /// Hidden size of the loaded checkpoint (384 for all-`MiniLM`-L6-v2).
    #[must_use]
    pub fn hidden_size(&self) -> usize {
        self.model.hidden_size()
    }

    /// Tokenize texts and build padded `[B, S]` id/mask tensors.
    ///
    /// Sequences are padded to the longest length in the batch (capped at
    /// [`MAX_SEQ`]); long inputs are truncated.
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
            .max()
            .unwrap_or(1)
            .clamp(1, MAX_SEQ);

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

        let device = Device::Cpu;
        let input_ids =
            Tensor::new(input_ids, &device).map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let attention_mask = Tensor::new(attention_masks, &device)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        Ok((input_ids, attention_mask))
    }

    /// Mean-pool `[B, S, H]` token states over the sequence using the mask,
    /// then L2-normalize each row.
    fn mean_pool(output: &Tensor, attention_mask: &Tensor) -> EmbeddingResult<Tensor> {
        let err = |e: candle_core::Error| EmbeddingError::Candle(e.to_string());
        let mask_f32 = attention_mask
            .to_dtype(candle_core::DType::F32)
            .map_err(err)?;
        let mask_3d = mask_f32.unsqueeze(2).map_err(err)?;
        let summed = output
            .broadcast_mul(&mask_3d)
            .map_err(err)?
            .sum(candle_core::D::Minus2)
            .map_err(err)?;
        let mask_sum = mask_f32
            .sum(candle_core::D::Minus1)
            .map_err(err)?
            .clamp(1e-9, f32::INFINITY)
            .map_err(err)?;
        let pooled = summed
            .broadcast_div(&mask_sum.unsqueeze(1).map_err(err)?)
            .map_err(err)?;
        let norm = pooled
            .sqr()
            .map_err(err)?
            .sum(candle_core::D::Minus1)
            .map_err(err)?
            .sqrt()
            .map_err(err)?
            .clamp(1e-12, f32::INFINITY)
            .map_err(err)?;
        pooled
            .broadcast_div(&norm.unsqueeze(1).map_err(err)?)
            .map_err(err)
    }
}

impl Embedder for MinilmEmbedder {
    /// # Errors
    ///
    /// Returns an error if tokenization or model inference fails.
    fn embed(&self, text: &str) -> EmbeddingResult<Embedding> {
        let (input_ids, attention_mask) = self.prepare_inputs(std::slice::from_ref(&text))?;
        let output = self.model.forward(&input_ids, &attention_mask)?;
        let embedding = Self::mean_pool(&output, &attention_mask)?;
        embedding
            .squeeze(0)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .to_vec1::<f32>()
            .map_err(|e| EmbeddingError::Candle(e.to_string()))
    }

    /// # Errors
    ///
    /// Returns an error if tokenization or model inference fails.
    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Embedding>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let _timer = crate::Timer::new("minilm_embed_batch");
        let (input_ids, attention_mask) = self.prepare_inputs(texts)?;
        let output = self.model.forward(&input_ids, &attention_mask)?;
        let embeddings = Self::mean_pool(&output, &attention_mask)?;
        embeddings
            .to_vec2::<f32>()
            .map_err(|e| EmbeddingError::Candle(e.to_string()))
    }

    fn dimensions(&self) -> usize {
        self.model.hidden_size()
    }

    fn name(&self) -> &'static str {
        "all-MiniLM-L6-v2"
    }
}
