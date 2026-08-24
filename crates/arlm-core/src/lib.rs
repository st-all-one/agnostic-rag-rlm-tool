#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::needless_borrow,
        clippy::unnecessary_literal_bound,
        clippy::float_cmp,
        clippy::duration_suboptimal_units,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )
)]
pub mod logging;
pub mod memory;
pub mod qa_cache;
pub mod types;

pub use memory::MemoryProvider;
pub use types::*;

#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
