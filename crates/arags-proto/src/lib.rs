//! Protobuf definitions and generated types for arags gRPC communication.
//!
//! This crate is the single source of truth for client-server gRPC contract.
//! The `.proto` files under `proto/` define the schema; `build.rs` compiles
//! them via `tonic_build` into Rust types at build time. The generated code
//! lives in the `proto` module, re-exported at the crate root.
//!
//! The schema is versioned via the protobuf `package` declaration
//! (`arags.v1`). The generated file name tracks the package
//! (`arags.v1.rs`); downstream code keeps referencing `arags_proto::proto::*`
//! and the `arags_service_{client,server}` modules, which this crate preserves.

pub mod proto {
    // The generated module is external (prost/tonic output) and intentionally
    // excluded from workspace pedantic lints via this module-level allow.
    #![allow(
        clippy::all,
        clippy::pedantic,
        clippy::cargo,
        clippy::nursery,
        dead_code,
        missing_docs
    )]

    include!(concat!(env!("OUT_DIR"), "/arags.v1.rs"));
}

pub use proto::*;
