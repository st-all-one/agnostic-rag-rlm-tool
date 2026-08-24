//! Shared helpers for mapping crate-internal errors to tonic [`Status`].

use tonic::Status;

/// Convert any error into an internal gRPC error, logging it first.
#[must_use]
pub fn internal<E: std::fmt::Display>(err: E) -> Status {
    tracing::error!(error = %err, "grpc handler error");
    Status::internal(err.to_string())
}

/// A not-found status.
#[must_use]
pub fn not_found(msg: &str) -> Status {
    tracing::debug!(message = msg, "grpc not found");
    Status::not_found(msg)
}

/// An invalid-argument status, logging the offending value.
#[must_use]
pub fn invalid_arg(msg: &str) -> Status {
    tracing::warn!(message = msg, "grpc invalid argument");
    Status::invalid_argument(msg)
}

/// A stream error status produced when a broadcast channel lags or closes.
#[must_use]
pub fn stream_error<E: std::fmt::Display>(err: E) -> Status {
    tracing::warn!(error = %err, "stream ended");
    Status::internal(err.to_string())
}
