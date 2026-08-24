//! `all-MiniLM-L6-v2` transformer: canonical BERT encoder with optional INT8.
//!
//! Architecture constants mirror `sentence-transformers/all-MiniLM-L6-v2`
//! (6 layers, 12 heads, hidden 384). When the checkpoint ships a `config.json`
//! the values there win — this keeps the loader honest for tests with tiny
//! synthetic models without exposing any runtime knob.

use std::path::Path;

use candle_core::quantized::GgmlDType;
use candle_core::{Device, Tensor};
use candle_nn::ops;

use crate::embedder::common::ops::{gelu, layer_norm, masked_fill};
use crate::embedder::common::weights::{Projection, build_projection, load_tensor};
use crate::embedder::{EmbeddingError, EmbeddingResult};

/// Hidden size of `all-MiniLM-L6-v2` (fallback when `config.json` is absent).
pub const HIDDEN_SIZE: usize = 384;
/// Encoder depth of `all-MiniLM-L6-v2`.
pub const NUM_LAYERS: usize = 6;
/// Attention heads of `all-MiniLM-L6-v2`.
pub const NUM_HEADS: usize = 12;

/// A single transformer layer: pre-norm self-attention + pre-norm FFN.
#[derive(Debug)]
pub(crate) struct TransformerLayer {
    attn_q: Projection,
    attn_k: Projection,
    attn_v: Projection,
    attn_o: Projection,
    attn_norm_w: Tensor,
    attn_norm_b: Tensor,
    ffn_in: Projection,
    ffn_out: Projection,
    ffn_norm_w: Tensor,
    ffn_norm_b: Tensor,
    num_heads: usize,
    head_dim: usize,
}

/// The full BERT encoder used by `all-MiniLM-L6-v2`.
#[derive(Debug)]
pub struct MiniLmModel {
    word_embeddings: Tensor,
    position_embeddings: Tensor,
    token_type_row0: Option<Tensor>,
    embed_norm_w: Tensor,
    embed_norm_b: Tensor,
    layers: Vec<TransformerLayer>,
    hidden_size: usize,
}

impl TransformerLayer {
    /// Forward pass over `[B, S, H]` hidden states with an `[B, S]` mask.
    fn forward(&self, hidden: &Tensor, attention_mask: &Tensor) -> EmbeddingResult<Tensor> {
        let (batch, seq_len) = {
            let d = hidden.dims();
            (
                *d.first()
                    .ok_or_else(|| EmbeddingError::ModelNotLoaded("empty hidden".into()))?,
                *d.get(1).ok_or_else(|| {
                    EmbeddingError::ModelNotLoaded("rank-2 hidden expected".into())
                })?,
            )
        };
        let normed = layer_norm(hidden, &self.attn_norm_w, &self.attn_norm_b)?;

        let split = |x: &Tensor| -> EmbeddingResult<Tensor> {
            x.reshape((batch, seq_len, self.num_heads, self.head_dim))
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?
                .permute((0, 2, 1, 3))
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?
                .contiguous()
                .map_err(|e| EmbeddingError::Candle(e.to_string()))
        };

        let q = split(&self.attn_q.forward(&normed)?)?;
        let k = split(&self.attn_k.forward(&normed)?)?;
        let v = split(&self.attn_v.forward(&normed)?)?;

        #[allow(clippy::cast_precision_loss)]
        let scale = (self.head_dim as f64).sqrt();
        let k_t = k
            .permute((0, 1, 3, 2))
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let scores = q
            .matmul(&k_t)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .affine(0.0, 1.0 / scale)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        // Extended mask: `[B, 1, 1, S]` broadcasting over heads and queries so
        // padded keys are masked out before softmax.
        let mask_f32 = attention_mask
            .to_dtype(candle_core::DType::F32)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let mask_4d = mask_f32
            .unsqueeze(1)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .unsqueeze(1)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .broadcast_as(scores.shape())
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let neg_inf = Tensor::new(f32::NEG_INFINITY, scores.device())
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .broadcast_as(scores.shape())
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let scores = masked_fill(&scores, &mask_4d, &neg_inf)?;

        let attn = ops::softmax(&scores, candle_core::D::Minus1)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .matmul(&v)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let attn = attn
            .permute((0, 2, 1, 3))
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .contiguous()
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .reshape((batch, seq_len, self.num_heads * self.head_dim))
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let attn_out = self.attn_o.forward(&attn)?;
        let hidden = hidden
            .broadcast_add(&attn_out)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let normed = layer_norm(&hidden, &self.ffn_norm_w, &self.ffn_norm_b)?;
        let ffn = self
            .ffn_out
            .forward(&gelu(&self.ffn_in.forward(&normed)?)?)?;
        hidden
            .broadcast_add(&ffn)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))
    }
}

impl MiniLmModel {
    /// Load model weights from a safetensors file using canonical
    /// `transformers` BERT tensor names (exactly what `all-MiniLM-L6-v2` ships).
    ///
    /// # Errors
    ///
    /// Returns an error if files are missing or tensors are malformed.
    pub fn load(
        model_dir: &Path,
        device: &Device,
        quant: Option<GgmlDType>,
    ) -> EmbeddingResult<Self> {
        let model_path = model_dir.join("model.safetensors");
        if !model_path.exists() {
            return Err(EmbeddingError::ModelNotLoaded(format!(
                "model.safetensors not found at {}",
                model_path.display()
            )));
        }

        let buffer = std::fs::read(&model_path)
            .map_err(|e| EmbeddingError::ModelNotLoaded(format!("read safetensors: {e}")))?;
        let tensors = safetensors::SafeTensors::deserialize(&buffer)
            .map_err(|e| EmbeddingError::ModelNotLoaded(format!("deserialize safetensors: {e}")))?;

        let (hidden_size, num_layers, num_heads) = read_arch(model_dir);
        let head_dim = hidden_size / num_heads;

        let proj = |prefix: &str| -> EmbeddingResult<Projection> {
            build_projection(prefix, &tensors, device, quant)
        };
        let load = |name: &str| -> EmbeddingResult<Tensor> { load_tensor(&tensors, name, device) };

        let word_embeddings = load("embeddings.word_embeddings.weight")?;
        let position_embeddings = load("embeddings.position_embeddings.weight")?;
        let embed_norm_w = load("embeddings.LayerNorm.weight")?;
        let embed_norm_b = load("embeddings.LayerNorm.bias")?;
        // Canonical BERT checkpoints carry token-type embeddings; sentence
        // embeddings only ever use type id 0, so keep just that row.
        let token_type_row0 = if tensors
            .tensor("embeddings.token_type_embeddings.weight")
            .is_ok()
        {
            let tt = load_tensor(&tensors, "embeddings.token_type_embeddings.weight", device)?;
            Some(
                tt.get(0)
                    .map_err(|e| EmbeddingError::Candle(format!("token_type row 0: {e}")))?,
            )
        } else {
            None
        };

        let mut layers = Vec::with_capacity(num_layers);
        for i in 0..num_layers {
            let p = format!("encoder.layer.{i}");
            layers.push(TransformerLayer {
                attn_q: proj(&format!("{p}.attention.self.query"))?,
                attn_k: proj(&format!("{p}.attention.self.key"))?,
                attn_v: proj(&format!("{p}.attention.self.value"))?,
                attn_o: proj(&format!("{p}.attention.output.dense"))?,
                attn_norm_w: load(&format!("{p}.attention.output.LayerNorm.weight"))?,
                attn_norm_b: load(&format!("{p}.attention.output.LayerNorm.bias"))?,
                ffn_in: proj(&format!("{p}.intermediate.dense"))?,
                ffn_out: proj(&format!("{p}.output.dense"))?,
                ffn_norm_w: load(&format!("{p}.output.LayerNorm.weight"))?,
                ffn_norm_b: load(&format!("{p}.output.LayerNorm.bias"))?,
                num_heads,
                head_dim,
            });
        }

        tracing::info!(
            dims = hidden_size,
            layers = num_layers,
            num_heads,
            quantized = quant.is_some(),
            "loaded MiniLM encoder weights"
        );

        Ok(Self {
            word_embeddings,
            position_embeddings,
            token_type_row0,
            embed_norm_w,
            embed_norm_b,
            layers,
            hidden_size,
        })
    }

    /// Base embedding dimensionality of this checkpoint.
    #[must_use]
    pub fn hidden_size(&self) -> usize {
        self.hidden_size
    }

    /// Forward pass: `[B, S]` ids + `[B, S]` mask → `[B, S, H]` states.
    ///
    /// # Errors
    ///
    /// Returns an error if tensor ops fail.
    pub fn forward(&self, input_ids: &Tensor, attention_mask: &Tensor) -> EmbeddingResult<Tensor> {
        let ids_2d = input_ids
            .to_vec2::<u32>()
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let batch = ids_2d.len();
        let seq_len = ids_2d.first().map_or(0, Vec::len);

        // Per-row lookup then stack: candle's index_select needs 1-D indices.
        let mut rows = Vec::with_capacity(batch);
        for row in &ids_2d {
            let idx = Tensor::from_vec(row.clone(), (row.len(),), input_ids.device())
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
            rows.push(
                self.word_embeddings
                    .index_select(&idx, 0)
                    .map_err(|e| EmbeddingError::Candle(e.to_string()))?,
            );
        }
        let word_emb = if batch == 1 {
            rows.remove(0)
        } else {
            Tensor::stack(&rows, 0).map_err(|e| EmbeddingError::Candle(e.to_string()))?
        };

        // BERT positions start at padding_idx + 1 (= 1).
        #[allow(clippy::cast_possible_truncation)]
        let position_ids = Tensor::arange(1u32, seq_len as u32 + 1, input_ids.device())
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let pos_emb = self
            .position_embeddings
            .index_select(&position_ids, 0)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let pos_emb = pos_emb
            .unsqueeze(0)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let mut hidden = word_emb
            .broadcast_add(&pos_emb)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        if let Some(tt0) = &self.token_type_row0 {
            hidden = hidden
                .broadcast_add(tt0)
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        }

        hidden = layer_norm(&hidden, &self.embed_norm_w, &self.embed_norm_b)?;
        for layer in &self.layers {
            hidden = layer.forward(&hidden, attention_mask)?;
        }
        Ok(hidden)
    }
}

/// Read `(hidden_size, num_layers, num_heads)` from `config.json`, falling
/// back to the `all-MiniLM-L6-v2` constants.
#[allow(clippy::type_complexity)]
fn read_arch(model_dir: &Path) -> (usize, usize, usize) {
    let defaults = (HIDDEN_SIZE, NUM_LAYERS, NUM_HEADS);
    let Ok(text) = std::fs::read_to_string(model_dir.join("config.json")) else {
        return defaults;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return defaults;
    };
    let get = |key: &str, fallback: usize| -> usize {
        json.get(key)
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| usize::try_from(v).ok())
            .filter(|v| *v > 0)
            .unwrap_or(fallback)
    };
    (
        get("hidden_size", defaults.0),
        get("num_hidden_layers", defaults.1),
        get("num_attention_heads", defaults.2),
    )
}
