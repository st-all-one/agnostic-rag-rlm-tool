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
// Pre-existing pervasive pedantic style lints in this crate's legacy code.
// Enforced as `warn` by the workspace; kept as allows so the crate builds
// clean under `cargo clippy -- -D warnings` without masking real issues.
#![allow(
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::too_many_arguments,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]
pub mod fts;
pub mod lance;
pub mod qa_vectors;
pub mod sqlite;

pub use lance::{SearchResult, VectorEntry, VectorStore};
pub use qa_vectors::QuestionVectorStore;
pub use sqlite::Role;
pub use sqlite::Storage;
pub use sqlite::cache;
pub use sqlite::qa_cache;
pub use sqlite::tokens;

#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
