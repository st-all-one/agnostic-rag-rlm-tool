#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::float_cmp
)]

use candle_core::{Device, Tensor};

use arlm_embedding::embedder::bge_m3::{
    BgeM3Embedder, apply_matryoshka, gelu, half_to_f32, layer_norm, masked_fill,
};
use arlm_embedding::embedder::matryoshka_truncate;

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
    let val = half_to_f32(0x3C00); // f16 1.0
    assert!((val - 1.0).abs() < 0.01);
    let val = half_to_f32(0x4000); // f16 2.0
    assert!((val - 2.0).abs() < 0.01);
}

#[test]
fn test_apply_matryoshka_none_passthrough() {
    let emb = vec![1.0_f32, 2.0, 3.0];
    assert_eq!(apply_matryoshka(emb.clone(), None), emb);
}

#[test]
fn test_matryoshka_truncate_via_bge_m3() {
    let emb = vec![1.0_f32, 2.0, 3.0];
    assert_eq!(matryoshka_truncate(&emb, 3), emb);
    assert_eq!(matryoshka_truncate(&emb, 2), vec![1.0, 2.0]);
}
