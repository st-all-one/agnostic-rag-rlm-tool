//! Self-attention + FFN forward pass for a single transformer layer.

use candle_core::Tensor;
use candle_nn::ops;

use super::super::{EmbeddingError, EmbeddingResult};
use super::model::TransformerLayer;
use super::ops::{gelu, layer_norm, masked_fill};

impl TransformerLayer {
    pub(crate) fn forward(
        &self,
        hidden: &Tensor,
        attention_mask: &Tensor,
    ) -> EmbeddingResult<Tensor> {
        let normed = layer_norm(hidden, &self.attn_norm_w, &self.attn_norm_b)?;

        let seq_len = normed
            .dim(1)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let q = self.attn_q.forward(&normed)?;
        let k = self.attn_k.forward(&normed)?;
        let v = self.attn_v.forward(&normed)?;

        let q = q
            .reshape((seq_len, self.num_heads, self.head_dim))
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .permute((1, 0, 2))
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let k = k
            .reshape((seq_len, self.num_heads, self.head_dim))
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .permute((1, 0, 2))
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let v = v
            .reshape((seq_len, self.num_heads, self.head_dim))
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .permute((1, 0, 2))
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        #[allow(clippy::cast_precision_loss)]
        let scale = (self.head_dim as f64).sqrt();
        let k_t = k
            .transpose(1, 2)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let attn_weights = q
            .matmul(&k_t)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .affine(0.0, 1.0 / scale)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let mask_f32 = attention_mask
            .to_dtype(candle_core::DType::F32)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let neg_inf = Tensor::new(f32::NEG_INFINITY, attention_mask.device())
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let filled = neg_inf
            .broadcast_as(attn_weights.shape())
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        // Mask the *key* dimension: `attn_weights` is `[heads, seq, seq]`, so the
        // broadcastable mask is `[1, 1, seq, seq]` where column `j` is the
        // key's attention mask (padded positions attend to -inf).
        let mask_4d = mask_f32
            .unsqueeze(1)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .broadcast_as(&[self.num_heads, seq_len, seq_len])
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let attn_weights = masked_fill(&attn_weights, &mask_4d, &filled)?;

        let attn_weights = ops::softmax(&attn_weights, candle_core::D::Minus1)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let attn_out = attn_weights
            .matmul(&v)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let attn_out = attn_out
            .permute((1, 0, 2))
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .reshape((seq_len, self.num_heads * self.head_dim))
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let attn_out = self.attn_o.forward(&attn_out)?;
        let hidden = hidden
            .broadcast_add(&attn_out)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let normed = layer_norm(&hidden, &self.ffn_norm_w, &self.ffn_norm_b)?;
        let ffn = self.ffn_dense_h.forward(&normed)?;
        let ffn = gelu(&ffn)?;
        let ffn = self.ffn_dense_o.forward(&ffn)?;

        let hidden = hidden
            .broadcast_add(&ffn)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        Ok(hidden)
    }
}
