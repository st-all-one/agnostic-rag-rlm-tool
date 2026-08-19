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
