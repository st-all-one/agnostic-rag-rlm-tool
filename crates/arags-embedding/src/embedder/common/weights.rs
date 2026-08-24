//! `SafeTensors` weight loading and quantized projection construction.

use std::sync::Arc;

use candle_core::quantized::{GgmlDType, QMatMul, QTensor};
use candle_core::{Device, Tensor};
use candle_nn::{Linear, Module};

use crate::embedder::common::ops::half_to_f32;
use crate::embedder::{EmbeddingError, EmbeddingResult};

/// A linear projection that may run with full-precision weights or as a
/// quantized matmul (`QMatMul`). Both variants apply the optional bias.
#[derive(Debug)]
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

/// Create a [`Projection`] from safetensors weights.
///
/// When `quant` is `None`, a full-precision `Linear` is built. When a GGML
/// dtype is supplied, the weight tensor is quantized via [`QTensor::quantize`]
/// and wrapped in a [`QMatMul`] (the bias is stored separately and applied
/// after the matmul).
pub(crate) fn build_projection(
    prefix: &str,
    tensors: &safetensors::SafeTensors<'_>,
    device: &Device,
    quant: Option<GgmlDType>,
) -> EmbeddingResult<Projection> {
    let w = load_tensor(tensors, &format!("{prefix}.weight"), device)?;
    let b = load_tensor(tensors, &format!("{prefix}.bias"), device)?;

    match quant {
        None => Ok(Projection::F32 {
            linear: candle_nn::Linear::new(w, Some(b)),
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
pub(crate) fn load_tensor(
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
            Tensor::from_vec(vec, shape, device).map_err(|e| EmbeddingError::Candle(e.to_string()))
        }
        safetensors::Dtype::F16 => {
            let vec: Vec<f32> = data
                .chunks_exact(2)
                .map(|b| {
                    let h = u16::from_le_bytes([b[0], b[1]]);
                    half_to_f32(h)
                })
                .collect();
            Tensor::from_vec(vec, shape, device).map_err(|e| EmbeddingError::Candle(e.to_string()))
        }
        other => Err(EmbeddingError::ModelNotLoaded(format!(
            "unsupported dtype {other:?} for tensor '{name}'"
        ))),
    }
}
