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
pub mod lance;
pub mod sqlite;

pub use lance::SearchResult;
pub use lance::VectorStore;
pub use sqlite::Storage;

#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_storage_open() {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::open(tmp.path()).unwrap();
        assert!(storage.path().exists());
    }

    #[tokio::test]
    async fn test_vector_store_open() {
        let tmp = TempDir::new().unwrap();
        let store = VectorStore::open(tmp.path()).await.unwrap();
        assert!(store.table.count_rows(None).await.unwrap() == 0);
    }
}
