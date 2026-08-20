//! Protobuf definitions and generated types for arlm gRPC communication.
//!
//! This crate contains the `.proto` files and generated Rust types used for
//! client-server communication via gRPC (tonic).

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/arlm.rs"));
}

pub use proto::*;
