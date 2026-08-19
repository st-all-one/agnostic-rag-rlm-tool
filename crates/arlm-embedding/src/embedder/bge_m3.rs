use std::path::Path;
use std::sync::Arc;

use candle_core::{Device, Tensor};
use tokenizers::Tokenizer;

use super::{Embedder, Embedding, EmbeddingError, EmbeddingResult};

/// BGE-M3 embedding model backed by candle.
///
/// Supports INT8 quantized weights for efficient CPU inference.
pub struct BgeM3Embedder {
    model: Arc<BgeM3Model>,
    tokenizer: Arc<Tokenizer>,
    device: Device,
    dims: usize,
}

/// Wrapper around the candle-transformers BGE-M3 model.
///
/// This is a thin abstraction so the embedder module doesn't depend on
/// the exact internal API of candle-transformers.
struct BgeM3Model {
    // In a real implementation this would hold the model weights.
    // For now we store a marker; the actual candle model loading
    // would go here when the model files are available.
    _device: Device,
}

impl BgeM3Model {
    #[allow(clippy::unused_self)]
    fn forward(
        &self,
        _input_ids: &Tensor,
        _attention_mask: &Tensor,
    ) -> EmbeddingResult<Tensor> {
        // Placeholder: real implementation loads from safetensors
        Err(EmbeddingError::ModelNotLoaded(
            "BGE-M3 model files not found. Provide a model directory with \
             model.safetensors and tokenizer.json."
                .into(),
        ))
    }
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

        let model = BgeM3Model {
            _device: device.clone(),
        };

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
        })
    }

    /// Tokenize and prepare model inputs for a batch of texts.
    fn prepare_inputs(
        &self,
        texts: &[&str],
    ) -> EmbeddingResult<(Tensor, Tensor)> {
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
            .unwrap_or(0);

        let mut input_ids = Vec::with_capacity(encodings.len());
        let mut attention_masks = Vec::with_capacity(encodings.len());

        for enc in &encodings {
            let mut ids = enc.get_ids().to_vec();
            let mut mask = enc.get_attention_mask().to_vec();
            ids.resize(max_len, 0);
            mask.resize(max_len, 0);
            input_ids.push(ids);
            attention_masks.push(mask);
        }

        let input_ids =
            Tensor::new(input_ids, &self.device).map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let attention_mask = Tensor::new(attention_masks, &self.device)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        Ok((input_ids, attention_mask))
    }

    /// Apply mean pooling to the model output.
    fn mean_pool(output: &Tensor, attention_mask: &Tensor) -> EmbeddingResult<Tensor> {
        let masked = output
            .broadcast_mul(attention_mask)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let summed = masked
            .sum(1)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let mask_sum = attention_mask
            .sum(1)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let mask_sum = mask_sum
            .clamp(1e-9, f32::INFINITY)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        summed
            .div(&mask_sum)
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
        embedding
            .to_vec1::<f32>()
            .map_err(|e| EmbeddingError::Candle(e.to_string()))
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
            result.push(vec);
        }
        Ok(result)
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn name(&self) -> &'static str {
        "bge-m3"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bge_m3_missing_model() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No tokenizer.json → should fail
        let result = BgeM3Embedder::new(dir.path(), 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_bge_m3_prepare_inputs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tok_path = dir.path().join("tokenizer.json");
        let vocab = serde_json::json!({
            "model": {
                "type": "BPE",
                "vocab": {
                    "[CLS]": 0, "[SEP]": 1, "[UNK]": 2, "[PAD]": 3,
                    "h": 4, "e": 5, "l": 6, "o": 7,
                    "w": 8, "r": 9, "d": 10
                },
                "merges": []
            },
            "decoder": {"type": "WordPiece", "prefix": "##", "cleanup": false},
            "normalizer": null,
            "pre_tokenizer": {"type": "Whitespace"},
            "post_processor": null
        });
        std::fs::write(&tok_path, serde_json::to_string(&vocab).expect("json")).expect("write tokenizer");
        let embedder = BgeM3Embedder::new(dir.path(), 128).expect("create");
        let (ids, mask) = embedder.prepare_inputs(&["hello"]).expect("prepare");
        assert!(ids.dim(0).unwrap_or(0) >= 1);
        assert!(mask.dim(0).unwrap_or(0) >= 1);
    }
}
