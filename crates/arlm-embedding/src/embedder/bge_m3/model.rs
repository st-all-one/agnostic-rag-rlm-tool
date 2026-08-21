//! BGE-M3 transformer model: types, weight loading, encoder forward pass.
//!
//! The model architecture is a standard pre-norm BERT encoder (BGE-M3). Public
//! BGE-M3 weights (e.g. `seansitter/bge-m3-safetensors`) use the canonical
//! `transformers` tensor naming (`embeddings.word_embeddings.weight`,
//! `encoder.layer.N.attention.self.query.weight`, …), which differs from the
//! internal naming this loader was written against. [`remap_name`] translates
//! the canonical names to the internal ones so any standard checkpoint loads.

use std::path::Path;

use candle_core::quantized::{GgmlDType, QMatMul};
use candle_core::{Device, Tensor};
use candle_nn::{Linear, Module};

use super::super::{EmbeddingError, EmbeddingResult};
use super::ops::layer_norm;
use super::weights::build_projection;
use super::weights::load_tensor;

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
    token_type_embeddings: Option<Tensor>,
    embed_norm_w: Tensor,
    embed_norm_b: Tensor,
    layers: Vec<TransformerLayer>,
}

/// Translate a canonical `transformers` BGE-M3 tensor name into the internal
/// name this loader expects. Canonical checkpoints (e.g.
/// `seansitter/bge-m3-safetensors`) expose the standard BERT naming, while the
/// internal forward pass uses a compact naming scheme.
fn remap_name(candle: &str) -> String {
    match candle {
        "embeddings.word.weight" => "embeddings.word_embeddings.weight".to_string(),
        "embeddings.position.weight" => "embeddings.position_embeddings.weight".to_string(),
        "embeddings.layer_norm.weight" => "embeddings.LayerNorm.weight".to_string(),
        "embeddings.layer_norm.bias" => "embeddings.LayerNorm.bias".to_string(),
        // The canonical final encoder norm is the last layer's post-FFN norm.
        "encoder.final_layer_norm.weight" => "encoder.layer.23.output.LayerNorm.weight".to_string(),
        "encoder.final_layer_norm.bias" => "encoder.layer.23.output.LayerNorm.bias".to_string(),
        _ => {
            if let Some(rest) = candle.strip_prefix("encoder.layers.") {
                let Some((idx, tail)) = rest.split_once('.') else {
                    return candle.to_string();
                };
                let std_tail = match tail {
                    "self_attn.q_proj" => "attention.self.query",
                    "self_attn.k_proj" => "attention.self.key",
                    "self_attn.v_proj" => "attention.self.value",
                    "self_attn.o_proj" => "attention.output.dense",
                    "self_attn_layer_norm.weight" => "attention.output.LayerNorm.weight",
                    "self_attn_layer_norm.bias" => "attention.output.LayerNorm.bias",
                    "mlp.dense.h_to_4h" => "intermediate.dense",
                    "mlp.dense.4h_to_h" => "output.dense",
                    "mlp_layer_norm.weight" => "output.LayerNorm.weight",
                    "mlp_layer_norm.bias" => "output.LayerNorm.bias",
                    _ => tail,
                };
                return format!("encoder.layer.{idx}.{std_tail}");
            }
            candle.to_string()
        }
    }
}

impl BgeM3Model {
    /// Load model weights from a safetensors file.
    ///
    /// Accepts standard `transformers` BGE-M3 checkpoints via [`remap_name`].
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

        let has = |name: &str| tensors.tensor(name).is_ok();

        // Load a tensor by its *internal* name, remapping from canonical names.
        let load = |candle: &str| -> EmbeddingResult<Tensor> {
            let canonical = remap_name(candle);
            load_tensor(&tensors, &canonical, device)
        };
        // Build a projection from its *internal* prefix, remapping to canonical.
        let proj = |candle_prefix: &str| -> EmbeddingResult<Projection> {
            let canonical = remap_name(candle_prefix);
            build_projection(&canonical, &tensors, device, quant)
        };

        let word_embeddings = load("embeddings.word.weight")?;
        let position_embeddings = load("embeddings.position.weight")?;
        let embed_norm_w = load("embeddings.layer_norm.weight")?;
        let embed_norm_b = load("embeddings.layer_norm.bias")?;
        let token_type_embeddings = if has("embeddings.token_type_embeddings.weight") {
            Some(load_tensor(
                &tensors,
                "embeddings.token_type_embeddings.weight",
                device,
            )?)
        } else {
            None
        };

        let num_heads = 16;
        let head_dim = dims / num_heads;

        let mut layers = Vec::new();
        for i in 0..24 {
            let prefix = format!("encoder.layers.{i}");
            let attn_q = proj(&format!("{prefix}.self_attn.q_proj"))?;
            let attn_k = proj(&format!("{prefix}.self_attn.k_proj"))?;
            let attn_v = proj(&format!("{prefix}.self_attn.v_proj"))?;
            let attn_o = proj(&format!("{prefix}.self_attn.o_proj"))?;
            let attn_norm_w = load(&format!("{prefix}.self_attn_layer_norm.weight"))?;
            let attn_norm_b = load(&format!("{prefix}.self_attn_layer_norm.bias"))?;

            let ffn_dense_h = proj(&format!("{prefix}.mlp.dense.h_to_4h"))?;
            let ffn_dense_o = proj(&format!("{prefix}.mlp.dense.4h_to_h"))?;
            let ffn_norm_w = load(&format!("{prefix}.mlp_layer_norm.weight"))?;
            let ffn_norm_b = load(&format!("{prefix}.mlp_layer_norm.bias"))?;

            layers.push(TransformerLayer {
                attn_q,
                attn_k,
                attn_v,
                attn_o,
                attn_norm_w,
                attn_norm_b,
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
            token_type_embeddings,
            embed_norm_w,
            embed_norm_b,
            layers,
        })
    }

    /// Forward pass: token ids + attention mask -> embeddings.
    pub(crate) fn forward(
        &self,
        input_ids: &Tensor,
        attention_mask: &Tensor,
    ) -> EmbeddingResult<Tensor> {
        // `prepare_inputs` emits a batched `[1, seq]` tensor for single-text
        // embedding; collapse the leading batch dim so `index_select` receives a
        // 1-D index vector.
        let input_ids = if input_ids.dims().len() == 2 {
            input_ids
                .squeeze(0)
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        } else {
            input_ids.clone()
        };
        let seq_len = input_ids
            .dim(0)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let word_emb = self
            .word_embeddings
            .index_select(&input_ids, 0)
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

        if let Some(tt) = &self.token_type_embeddings {
            hidden = hidden
                .broadcast_add(tt)
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        }

        hidden = layer_norm(&hidden, &self.embed_norm_w, &self.embed_norm_b)?;

        for layer in &self.layers {
            hidden = layer.forward(&hidden, attention_mask)?;
        }

        Ok(hidden)
    }
}
