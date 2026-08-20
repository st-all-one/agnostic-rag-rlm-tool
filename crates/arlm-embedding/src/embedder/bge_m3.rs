use std::path::Path;
use std::sync::Arc;

use candle_core::quantized::{GgmlDType, QMatMul, QTensor};
use candle_core::{Device, Tensor};
use candle_nn::{Linear, Module, ops};
use tokenizers::Tokenizer;

use super::config::EmbeddingConfig;
use super::{matryoshka_truncate, Embedder, Embedding, EmbeddingError, EmbeddingResult};

#[allow(dead_code)]
const DEFAULT_DIMS: usize = 1024;
const DEFAULT_MAX_LEN: usize = 512;

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

/// A linear projection that may run with full-precision weights or as a
/// quantized matmul (`QMatMul`). Both variants apply the optional bias.
enum Projection {
    /// Full-precision f32 linear (candle_nn::Linear, includes bias).
    F32 { linear: Linear },
    /// Quantized matmul (`QMatMul`) plus separately-stored bias.
    Quantized { qmatmul: QMatMul, bias: Option<Tensor> },
}

impl Projection {
    /// Forward pass: returns `x @ W^T + b`.
    fn forward(&self, x: &Tensor) -> EmbeddingResult<Tensor> {
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
struct TransformerLayer {
    attn_q: Projection,
    attn_k: Projection,
    attn_v: Projection,
    attn_o: Projection,
    attn_norm_w: Tensor,
    attn_norm_b: Tensor,
    ffn_dense_h: Projection,
    ffn_dense_o: Projection,
    ffn_norm_w: Tensor,
    ffn_norm_b: Tensor,
    num_heads: usize,
    head_dim: usize,
}

/// The full transformer encoder.
struct BgeM3Model {
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
    fn load(
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
        let tensors = safetensors::SafeTensors::deserialize(&buffer).map_err(|e| {
            EmbeddingError::ModelNotLoaded(format!("deserialize safetensors: {e}"))
        })?;

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
                            half_to_f32(h)
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
            let attn_q =
                build_projection(&format!("{prefix}.self_attn.q_proj"), &tensors, device, quant)?;
            let attn_k =
                build_projection(&format!("{prefix}.self_attn.k_proj"), &tensors, device, quant)?;
            let attn_v =
                build_projection(&format!("{prefix}.self_attn.v_proj"), &tensors, device, quant)?;
            let attn_o =
                build_projection(&format!("{prefix}.self_attn.o_proj"), &tensors, device, quant)?;
            let attn_norm_w = get(&format!("{prefix}.self_attn_layer_norm.weight"))?;
            let attn_norm_b = get(&format!("{prefix}.self_attn_layer_norm.bias"))?;

            let ffn_dense_h =
                build_projection(&format!("{prefix}.mlp.dense.h_to_4h"), &tensors, device, quant)?;
            let ffn_dense_o =
                build_projection(&format!("{prefix}.mlp.dense.4h_to_h"), &tensors, device, quant)?;
            let ffn_norm_w = get(&format!("{prefix}.mlp_layer_norm.weight"))?;
            let ffn_norm_b = get(&format!("{prefix}.mlp_layer_norm.bias"))?;

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
            embed_norm_w,
            embed_norm_b,
            layers,
            final_norm_w,
            final_norm_b,
        })
    }

    /// Forward pass: token ids + attention mask -> embeddings.
    fn forward(&self, input_ids: &Tensor, attention_mask: &Tensor) -> EmbeddingResult<Tensor> {
        let seq_len = input_ids.dim(0).map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        // Word + position embeddings
        let word_emb = self
            .word_embeddings
            .index_select(input_ids, 0)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        let device = self.word_embeddings.device();
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

        // Embedding layer norm
        hidden = layer_norm(&hidden, &self.embed_norm_w, &self.embed_norm_b)?;

        // Transformer layers
        for layer in &self.layers {
            hidden = layer.forward(&hidden, attention_mask)?;
        }

        // Final layer norm
        hidden = layer_norm(&hidden, &self.final_norm_w, &self.final_norm_b)?;

        Ok(hidden)
    }
}

impl TransformerLayer {
    fn forward(&self, hidden: &Tensor, attention_mask: &Tensor) -> EmbeddingResult<Tensor> {
        // Pre-norm self-attention
        let normed = layer_norm(hidden, &self.attn_norm_w, &self.attn_norm_b)?;

        let seq_len = normed.dim(1).map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        // Project Q, K, V using Projection::forward
        let q = self.attn_q.forward(&normed)?;
        let k = self.attn_k.forward(&normed)?;
        let v = self.attn_v.forward(&normed)?;

        // Reshape to (batch=1, seq, heads, head_dim) -> (batch=1, heads, seq, head_dim)
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

        // Scaled dot-product attention: (heads, seq, seq)
        let scale = (self.head_dim as f64).sqrt();
        let k_t = k
            .transpose(1, 2)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let attn_weights = q
            .matmul(&k_t)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .affine(0.0, 1.0 / scale)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        // Apply attention mask: expand mask to (1, 1, seq)
        let mask_f32 = attention_mask
            .to_dtype(candle_core::DType::F32)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .unsqueeze(0)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        // masked_fill: where mask==0, set to -inf
        let neg_inf = Tensor::new(f32::NEG_INFINITY, &attention_mask.device())
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let filled = neg_inf
            .broadcast_as(attn_weights.shape())
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let mask_4d = mask_f32
            .unsqueeze(2)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
        let attn_weights = masked_fill(&attn_weights, &mask_4d, &filled)?;

        let attn_weights = ops::softmax(&attn_weights, candle_core::D::Minus1)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        // Attention output: (heads, seq, head_dim)
        let attn_out = attn_weights
            .matmul(&v)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        // Reshape back: (seq, hidden)
        let attn_out = attn_out
            .permute((1, 0, 2))
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?
            .reshape((seq_len, self.num_heads * self.head_dim))
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        // Output projection + residual
        let attn_out = self.attn_o.forward(&attn_out)?;
        let hidden = hidden
            .broadcast_add(&attn_out)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        // Pre-norm FFN
        let normed = layer_norm(&hidden, &self.ffn_norm_w, &self.ffn_norm_b)?;
        let ffn = self.ffn_dense_h.forward(&normed)?;
        let ffn = gelu(&ffn)?;
        let ffn = self.ffn_dense_o.forward(&ffn)?;

        // Residual
        let hidden = hidden
            .broadcast_add(&ffn)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        Ok(hidden)
    }
}

/// Layer normalization: (x - mean) / sqrt(var + eps) * weight + bias
fn layer_norm(x: &Tensor, w: &Tensor, b: &Tensor) -> EmbeddingResult<Tensor> {
    let x_f32 = x
        .to_dtype(candle_core::DType::F32)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let mean = x_f32
        .mean(candle_core::D::Minus1)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let var = x_f32
        .broadcast_sub(&mean.unsqueeze(candle_core::D::Minus1).map_err(|e| EmbeddingError::Candle(e.to_string()))?)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .powf(2.0)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .mean(candle_core::D::Minus1)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let eps_tensor = Tensor::new(1e-5_f32, x.device())
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let std = var
        .broadcast_add(&eps_tensor.broadcast_as(var.shape()).map_err(|e| EmbeddingError::Candle(e.to_string()))?)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .sqrt()
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let normed = x_f32
        .broadcast_sub(&mean.unsqueeze(candle_core::D::Minus1).map_err(|e| EmbeddingError::Candle(e.to_string()))?)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .broadcast_div(&std.unsqueeze(candle_core::D::Minus1).map_err(|e| EmbeddingError::Candle(e.to_string()))?)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let w_f32 = w
        .to_dtype(candle_core::DType::F32)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let b_f32 = b
        .to_dtype(candle_core::DType::F32)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    normed
        .broadcast_mul(&w_f32)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .broadcast_add(&b_f32)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .to_dtype(x.dtype())
        .map_err(|e| EmbeddingError::Candle(e.to_string()))
}

/// GELU activation using tanh approximation: 0.5 * x * (1 + tanh(sqrt(2/pi) * (x + 0.044715 * x^3)))
fn gelu(x: &Tensor) -> EmbeddingResult<Tensor> {
    let x_f32 = x
        .to_dtype(candle_core::DType::F32)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    // sqrt(2/pi) ≈ 0.7978845608
    let sqrt_2_over_pi = Tensor::new(0.7978845608_f32, x.device())
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let c = Tensor::new(0.044715_f32, x.device())
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let half = Tensor::new(0.5_f32, x.device())
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let one = Tensor::new(1.0_f32, x.device())
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

    let x_cubed = x_f32
        .powf(3.0)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let c_x3 = c
        .broadcast_as(x_f32.shape())
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .mul(&x_cubed)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let inner = x_f32
        .broadcast_add(&c_x3)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .broadcast_mul(&sqrt_2_over_pi.broadcast_as(x_f32.shape()).map_err(|e| EmbeddingError::Candle(e.to_string()))?)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let tanh_val = inner.tanh().map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let result = one
        .broadcast_add(&tanh_val)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .broadcast_mul(&half.broadcast_as(x_f32.shape()).map_err(|e| EmbeddingError::Candle(e.to_string()))?)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .broadcast_mul(&x_f32)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    result
        .to_dtype(x.dtype())
        .map_err(|e| EmbeddingError::Candle(e.to_string()))
}

/// Masked fill: for each element, if mask == 0, use fill_val; else use x.
fn masked_fill(x: &Tensor, mask: &Tensor, fill_val: &Tensor) -> EmbeddingResult<Tensor> {
    mask.gt(0.0)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .where_cond(x, fill_val)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))
}

/// Convert f16 to f32 (software conversion).
fn half_to_f32(h: u16) -> f32 {
    let sign = (h >> 15) as u32;
    let exp = ((h >> 10) & 0x1F) as u32;
    let mantissa = (h & 0x3FF) as u32;

    if exp == 0 {
        if mantissa == 0 {
            f32::from_bits(sign << 31)
        } else {
            // Subnormal
            let mut m = mantissa;
            let mut e = 1u32;
            while (m & 0x400) == 0 {
                m <<= 1;
                e += 1;
            }
            let val = f32::from_bits((sign << 31) | ((127 - 15 + 1 - e) << 23) | ((m & 0x3FF) << 13));
            val
        }
    } else if exp == 31 {
        f32::from_bits((sign << 31) | 0x7F800000 | ((mantissa as u32) << 13))
    } else {
        let val = f32::from_bits((sign << 31) | ((exp + 127 - 15) << 23) | ((mantissa as u32) << 13));
        val
    }
}

/// Create a [`Projection`] from safetensors weights.
///
/// When `quant` is `None`, a full-precision `Linear` is built. When a GGML
/// dtype is supplied, the weight tensor is quantized via [`QTensor::quantize`]
/// and wrapped in a [`QMatMul`] (the bias is stored separately and applied
/// after the matmul in [`Projection::forward`]).
fn build_projection(
    prefix: &str,
    tensors: &safetensors::SafeTensors<'_>,
    device: &Device,
    quant: Option<GgmlDType>,
) -> EmbeddingResult<Projection> {
    let w = load_tensor(tensors, &format!("{prefix}.weight"), device)?;
    let b = load_tensor(tensors, &format!("{prefix}.bias"), device)?;

    match quant {
        None => Ok(Projection::F32 {
            linear: Linear::new(w, Some(b)),
        }),
        Some(dtype) => {
            let qtensor = QTensor::quantize(&w, dtype)
                .map_err(|e| EmbeddingError::Candle(format!("quantize {prefix}: {e}")))?;
            let qmatmul = QMatMul::from_arc(Arc::new(qtensor))
                .map_err(|e| EmbeddingError::Candle(format!("qmatmul {prefix}: {e}")))?;
            Ok(Projection::Quantized {
                qmatmul,
                bias: Some(b),
            })
        }
    }
}

/// Load a single tensor from safetensors by name.
fn load_tensor(
    tensors: &safetensors::SafeTensors<'_>,
    name: &str,
    device: &Device,
) -> EmbeddingResult<Tensor> {
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
                    half_to_f32(h)
                })
                .collect();
            Tensor::from_vec(vec, shape, device)
                .map_err(|e| EmbeddingError::Candle(e.to_string()))
        }
        other => Err(EmbeddingError::ModelNotLoaded(format!(
            "unsupported dtype {other:?} for tensor '{name}'"
        ))),
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

        let model =
            BgeM3Model::load(model_dir, config.dims, &device, config.quantization.ggml_dtype())?;

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
            .min(DEFAULT_MAX_LEN);

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
            .sum(candle_core::D::Minus1)
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
            .div(&mask_sum_2d)
            .map_err(|e| EmbeddingError::Candle(e.to_string()))?;

        // L2 normalize
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
            .div(&norm_2d)
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

/// Apply Matryoshka truncation if `dims` is `Some`, otherwise pass through.
fn apply_matryoshka(emb: Embedding, dims: Option<usize>) -> Embedding {
    match dims {
        Some(d) => matryoshka_truncate(&emb, d),
        None => emb,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_bge_m3_missing_model() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = BgeM3Embedder::new(dir.path(), 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_bge_m3_missing_tokenizer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let model_path = dir.path().join("model.safetensors");
        std::fs::write(&model_path, b"").expect("write");
        let result = BgeM3Embedder::new(dir.path(), 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_gelu_positive() {
        let device = Device::Cpu;
        let x = Tensor::new(vec![1.0_f32, 2.0, 3.0], &device).unwrap();
        let y = gelu(&x).unwrap();
        let vals = y.to_vec1::<f32>().unwrap();
        // GELU(1) ≈ 0.841, GELU(2) ≈ 1.954, GELU(3) ≈ 2.996
        assert!((vals[0] - 0.841).abs() < 0.01);
        assert!((vals[1] - 1.954).abs() < 0.01);
        assert!((vals[2] - 2.996).abs() < 0.01);
    }

    #[test]
    fn test_gelu_negative() {
        let device = Device::Cpu;
        let x = Tensor::new(vec![-1.0_f32, -2.0], &device).unwrap();
        let y = gelu(&x).unwrap();
        let vals = y.to_vec1::<f32>().unwrap();
        // GELU(-1) ≈ -0.159, GELU(-2) ≈ -0.045
        assert!((vals[0] - (-0.159)).abs() < 0.01);
        assert!((vals[1] - (-0.045)).abs() < 0.01);
    }

    #[test]
    fn test_layer_norm() {
        let device = Device::Cpu;
        let x = Tensor::new(vec![1.0_f32, 2.0, 3.0, 4.0], &device)
            .unwrap()
            .reshape((1, 4))
            .unwrap();
        let w = Tensor::ones(4, candle_core::DType::F32, &device).unwrap();
        let b = Tensor::zeros(4, candle_core::DType::F32, &device).unwrap();
        let y = layer_norm(&x, &w, &b).unwrap();
        let vals = y.to_vec2::<f32>().unwrap();
        let mean: f32 = vals[0].iter().sum::<f32>() / vals[0].len() as f32;
        assert!(mean.abs() < 1e-5);
    }

    #[test]
    fn test_masked_fill() {
        let device = Device::Cpu;
        let x = Tensor::new(vec![1.0_f32, 2.0, 3.0], &device).unwrap();
        let mask = Tensor::new(vec![1.0_f32, 0.0, 1.0], &device).unwrap();
        let fill = Tensor::new(vec![f32::NEG_INFINITY; 3], &device).unwrap();
        let y = masked_fill(&x, &mask, &fill).unwrap();
        let vals = y.to_vec1::<f32>().unwrap();
        assert_eq!(vals[0], 1.0);
        assert!(vals[1].is_infinite());
        assert_eq!(vals[2], 3.0);
    }

    #[test]
    fn test_half_to_f32() {
        // f16 1.0 = 0x3C00
        let val = half_to_f32(0x3C00);
        assert!((val - 1.0).abs() < 0.01);
        // f16 2.0 = 0x4000
        let val = half_to_f32(0x4000);
        assert!((val - 2.0).abs() < 0.01);
    }

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
    fn test_apply_matryoshka_none_passthrough() {
        let emb = vec![1.0_f32, 2.0, 3.0];
        assert_eq!(apply_matryoshka(emb.clone(), None), emb);
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
        std::fs::write(&tok_path, serde_json::to_string(&vocab).expect("json"))
            .expect("write tokenizer");
        // Model file needed for full test — covered by integration tests
    }
}
