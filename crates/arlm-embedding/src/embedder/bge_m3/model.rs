//! BGE-M3 transformer model: types, weight loading, encoder forward pass.

use std::path::Path;

use candle_core::quantized::{GgmlDType, QMatMul};
use candle_core::{Device, Tensor};
use candle_nn::{Linear, Module};

use super::super::{EmbeddingError, EmbeddingResult};
use super::ops::layer_norm;
use super::weights::build_projection;

/// A linear projection that may run with full-precision weights or as a
/// quantized matmul (`QMatMul`). Both variants apply the optional bias.
pub(crate) enum Projection {
    /// Full-precision f32 linear (`candle_nn::Linear`, includes bias).
    F32 { linear: Linear },
    /// Quantized matmul (`QMatMul`) plus separately-stored bias.
    Quantized {
        qmatmul: QMatMul,
        bias: Option<Tensor>,
    },
}

impl Projection {
    /// Forward pass: returns `x @ W^T + b`.
    pub(crate) fn forward(&self, x: &Tensor) -> EmbeddingResult<Tensor> {
        match self {
            Projection::F32 { linear } => linear
                .forward(x)
                .map_err(|e| EmbeddingError::Candle(e.to_string())),
            Projection::Quantized { qmatmul, bias } => {
                let out = qmatmul
                    .forward(x)
                    .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
                match bias {
                    Some(b) => out
                        .broadcast_add(b)
                        .map_err(|e| EmbeddingError::Candle(e.to_string())),
                    None => Ok(out),
                }
            }
        }
    }
}

/// A single transformer layer (self-attention + FFN with pre-norm).
pub(crate) struct TransformerLayer {
    pub(crate) attn_q: Projection,
    pub(crate) attn_k: Projection,
    pub(crate) attn_v: Projection,
    pub(crate) attn_o: Projection,
    pub(crate) attn_norm_w: Tensor,
    pub(crate) attn_norm_b: Tensor,
    pub(crate) ffn_dense_h: Projection,
    pub(crate) ffn_dense_o: Projection,
    pub(crate) ffn_norm_w: Tensor,
    pub(crate) ffn_norm_b: Tensor,
    pub(crate) num_heads: usize,
    pub(crate) head_dim: usize,
}

/// The full transformer encoder.
pub(crate) struct BgeM3Model {
    word_embeddings: Tensor,
    position_embeddings: Tensor,
    embed_norm_w: Tensor,
    embed_norm_b: Tensor,
    layers: Vec<TransformerLayer>,
    final_norm_w: Tensor,
    final_norm_b: Tensor,
}

impl BgeM3Model {
    /// Load model weights from a safetensors file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or tensors are missing/malformed.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn load(
        model_dir: &Path,
        dims: usize,
        device: &Device,
        quant: Option<GgmlDType>,
    ) -> EmbeddingResult<Self> {
        let model_path = model_dir.join("model.safetensors");
        let tokenizer_path = model_dir.join("tokenizer.json");

        if !model_path.exists() {
            return Err(EmbeddingError::ModelNotLoaded(format!(
                "model.safetensors not found at {}",
                model_path.display()
            )));
        }
        if !tokenizer_path.exists() {
            return Err(EmbeddingError::ModelNotLoaded(format!(
                "tokenizer.json not found at {}",
                tokenizer_path.display()
            )));
        }

        let buffer = std::fs::read(&model_path)
            .map_err(|e| EmbeddingError::ModelNotLoaded(format!("read safetensors: {e}")))?;
        let tensors = safetensors::SafeTensors::deserialize(&buffer)
            .map_err(|e| EmbeddingError::ModelNotLoaded(format!("deserialize safetensors: {e}")))?;

        let get = |name: &str| -> EmbeddingResult<Tensor> {
            let info = tensors
                .tensor(name)
                .map_err(|e| EmbeddingError::ModelNotLoaded(format!("tensor '{name}': {e}")))?;
            let dtype = info.dtype();
            let shape = info.shape();
            let data = info.data();
            match dtype {
                safetensors::Dtype::F32 => {
                    let vec: Vec<f32> = data
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                        .collect();
                    Tensor::from_vec(vec, shape, device)
                        .map_err(|e| EmbeddingError::Candle(e.to_string()))
                }
                safetensors::Dtype::F16 => {
                    let vec: Vec<f32> = data
                        .chunks_exact(2)
                        .map(|b| {
                            let h = u16::from_le_bytes([b[0], b[1]]);
                            super::ops::half_to_f32(h)
                        })
                        .collect();
                    Tensor::from_vec(vec, shape, device)
                        .map_err(|e| EmbeddingError::Candle(e.to_string()))
                }
                other => Err(EmbeddingError::ModelNotLoaded(format!(
                    "unsupported dtype {other:?} for tensor '{name}'"
                ))),
            }
        };

        let word_embeddings = get("embeddings.word.weight")?;
        let position_embeddings = get("embeddings.position.weight")?;
        let embed_norm_w = get("embeddings.layer_norm.weight")?;
        let embed_norm_b = get("embeddings.layer_norm.bias")?;
        let final_norm_w = get("encoder.final_layer_norm.weight")?;
        let final_norm_b = get("encoder.final_layer_norm.bias")?;

        let num_heads = 16;
        let head_dim = dims / num_heads;

        let mut layers = Vec::new();
        for i in 0..24 {
            let prefix = format!("encoder.layers.{i}");
            let attn_q = build_projection(
                &format!("{prefix}.self_attn.q_proj"),
                &tensors,
                device,
                quant,
            )?;
            let attn_k = build_projection(
                &format!("{prefix}.self_attn.k_proj"),
                &tensors,
                device,
                quant,
            )?;
            let attn_v = build_projection(
                &format!("{prefix}.self_attn.v_proj"),
                &tensors,
                device,
                quant,
            )?;
            let attn_o = build_projection(
                &format!("{prefix}.self_attn.o_proj"),
                &tensors,
                device,
                quant,
            )?;
            let attn_norm_w = get(&format!("{prefix}.self_attn_layer_norm.weight"))?;
            let attn_norm_b_placeholder = get(&format!("{prefix}.self_attn_layer_norm.bias"))?;

            let ffn_dense_h = build_projection(
                &format!("{prefix}.mlp.dense.h_to_4h"),
                &tensors,
                device,
                quant,
            )?;
            let ffn_dense_o = build_projection(
                &format!("{prefix}.mlp.dense.4h_to_h"),
                &tensors,
                device,
                quant,
            )?;
            let ffn_norm_w = get(&format!("{prefix}.mlp_layer_norm.weight"))?;
            let ffn_norm_b = get(&format!("{prefix}.mlp_layer_norm.bias"))?;

            layers.push(TransformerLayer {
                attn_q,
                attn_k,
                attn_v,
                attn_o,
                attn_norm_w,
                attn_norm_b: attn_norm_b_placeholder,
                ffn_dense_h,
                ffn_dense_o,
                ffn_norm_w,
                ffn_norm_b,
                num_heads,
                head_dim,
            });
        }

        tracing::info!(
            dims,
            layers = layers.len(),
            num_heads,
            "loaded BGE-M3 model weights"
        );

        Ok(Self {
            word_embeddings,
            position_embeddings,
            embed_norm_w,
            embed_norm_b,
            layers,
            final_norm_w,
            final_norm_b,
        })
    }

    /// Forward pass: token ids + attention mask -> embeddings.
    pub(crate) fn forward(
        &self,
        input_ids: &Tensor,
        attention_mask: &Tensor,
    ) -> EmbeddingResult<Tensor> {
        let seq_len = input_ids
            .dim(0)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let word_emb = self
            .word_embeddings
            .index_select(input_ids, 0)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let device = self.word_embeddings.device();
        #[allow(clippy::cast_possible_truncation)]
        let position_ids = Tensor::arange(0u32, seq_len as u32, device)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let pos_emb = self
            .position_embeddings
            .index_select(&position_ids, 0)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let pos_emb_2d = pos_emb
            .unsqueeze(0)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let mut hidden = word_emb
            .broadcast_add(&pos_emb_2d)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        hidden = layer_norm(&hidden, &self.embed_norm_w, &self.embed_norm_b)?;

        for layer in &self.layers {
            hidden = layer.forward(&hidden, attention_mask)?;
        }

        hidden = layer_norm(&hidden, &self.final_norm_w, &self.final_norm_b)?;

        Ok(hidden)
    }
}
