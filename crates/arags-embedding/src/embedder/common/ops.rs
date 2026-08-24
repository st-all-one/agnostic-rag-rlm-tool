//! Low-level tensor math shared by the `MiniLM` transformer.
//!
//! These pure functions are exposed for testing but are not part of the
//! public embedder API surface in a meaningful sense.

use candle_core::{DType, Tensor};

use crate::embedder::{Embedding, EmbeddingError, EmbeddingResult, matryoshka_truncate};

/// Layer normalization: (x - mean) / sqrt(var + eps) * weight + bias
#[doc(hidden)]
pub fn layer_norm(x: &Tensor, w: &Tensor, b: &Tensor) -> EmbeddingResult<Tensor> {
    let x_f32 = x
        .to_dtype(DType::F32)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let mean = x_f32
        .mean(candle_core::D::Minus1)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let var = x_f32
        .broadcast_sub(
            &mean
                .unsqueeze(candle_core::D::Minus1)
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?,
        )
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .powf(2.0)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .mean(candle_core::D::Minus1)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let eps_tensor =
        Tensor::new(1e-5_f32, x.device()).map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let std = var
        .broadcast_add(
            &eps_tensor
                .broadcast_as(var.shape())
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?,
        )
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .sqrt()
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let normed = x_f32
        .broadcast_sub(
            &mean
                .unsqueeze(candle_core::D::Minus1)
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?,
        )
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .broadcast_div(
            &std.unsqueeze(candle_core::D::Minus1)
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?,
        )
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let w_f32 = w
        .to_dtype(DType::F32)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let b_f32 = b
        .to_dtype(DType::F32)
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
#[doc(hidden)]
pub fn gelu(x: &Tensor) -> EmbeddingResult<Tensor> {
    let x_f32 = x
        .to_dtype(DType::F32)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let sqrt_2_over_pi = Tensor::new(0.797_884_6_f32, x.device())
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let c = Tensor::new(0.044_715_f32, x.device())
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let half =
        Tensor::new(0.5_f32, x.device()).map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let one =
        Tensor::new(1.0_f32, x.device()).map_err(|e| EmbeddingError::Candle(e.to_string()))?;

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
        .broadcast_mul(
            &sqrt_2_over_pi
                .broadcast_as(x_f32.shape())
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?,
        )
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let tanh_val = inner
        .tanh()
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    let result = one
        .broadcast_add(&tanh_val)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .broadcast_mul(
            &half
                .broadcast_as(x_f32.shape())
                .map_err(|e| EmbeddingError::Candle(e.to_string()))?,
        )
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .broadcast_mul(&x_f32)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?;
    result
        .to_dtype(x.dtype())
        .map_err(|e| EmbeddingError::Candle(e.to_string()))
}

/// Masked fill: for each element, if mask == 0, use fill_val; else use x.
#[doc(hidden)]
pub fn masked_fill(x: &Tensor, mask: &Tensor, fill_val: &Tensor) -> EmbeddingResult<Tensor> {
    mask.gt(0.0)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))?
        .where_cond(x, fill_val)
        .map_err(|e| EmbeddingError::Candle(e.to_string()))
}

/// Convert f16 to f32 (software conversion).
#[doc(hidden)]
#[must_use]
pub fn half_to_f32(h: u16) -> f32 {
    let sign = u32::from(h >> 15);
    let exp = u32::from((h >> 10) & 0x1F);
    let mantissa = u32::from(h & 0x3FF);

    if exp == 0 {
        if mantissa == 0 {
            f32::from_bits(sign << 31)
        } else {
            let mut m = mantissa;
            let mut e = 1u32;
            while (m & 0x400) == 0 {
                m <<= 1;
                e += 1;
            }

            f32::from_bits((sign << 31) | ((127 - 15 + 1 - e) << 23) | ((m & 0x3FF) << 13))
        }
    } else if exp == 31 {
        f32::from_bits((sign << 31) | 0x7F80_0000 | (mantissa << 13))
    } else {
        f32::from_bits((sign << 31) | ((exp + 127 - 15) << 23) | (mantissa << 13))
    }
}

/// Apply Matryoshka truncation if `dims` is `Some`, otherwise pass through.
#[doc(hidden)]
#[must_use]
pub fn apply_matryoshka(emb: Embedding, dims: Option<usize>) -> Embedding {
    match dims {
        Some(d) => matryoshka_truncate(&emb, d),
        None => emb,
    }
}
