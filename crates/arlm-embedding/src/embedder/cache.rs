use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::{Embedding, EmbeddingError, EmbeddingResult};

/// SQLite-backed embedding cache.
///
/// Stores computed embeddings keyed by content hash to avoid redundant
/// model inference. Thread-safe via internal mutex.
pub struct EmbeddingCache {
    conn: Arc<Mutex<Connection>>,
    dims: usize,
}

impl EmbeddingCache {
    /// Open or create an embedding cache database.
    ///
    /// # Arguments
    ///
    /// * `db_path` - Path to the `SQLite` database file. Use `:memory:` for in-memory.
    /// * `dims` - Expected embedding dimensionality (used for validation).
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or the schema cannot be created.
    pub fn open(db_path: &str, dims: usize) -> EmbeddingResult<Self> {
        let conn = Connection::open(db_path)?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS embedding_cache (
                hash TEXT PRIMARY KEY,
                embedding BLOB NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_embedding_cache_hash ON embedding_cache(hash);",
        )?;

        tracing::info!(db_path = db_path, dims = dims, "opened embedding cache");

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            dims,
        })
    }

    /// Create an in-memory cache for testing.
    ///
    /// # Errors
    ///
    /// Returns an error if the in-memory database cannot be opened.
    pub fn in_memory(dims: usize) -> EmbeddingResult<Self> {
        Self::open(":memory:", dims)
    }

    /// Compute the SHA-256 hash of text content.
    #[must_use]
    pub fn content_hash(text: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }

    /// Look up a cached embedding by content hash.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get(&self, text: &str) -> EmbeddingResult<Option<Embedding>> {
        let hash = Self::content_hash(text);
        let conn = self.conn.lock();

        let mut stmt =
            conn.prepare("SELECT embedding FROM embedding_cache WHERE hash = ?1")?;

        let result = stmt.query_row(rusqlite::params![hash], |row| {
            let blob: Vec<u8> = row.get(0)?;
            Ok(blob)
        });

        match result {
            Ok(blob) => {
                let embedding = bytes_to_embedding(&blob, self.dims)?;
                Ok(Some(embedding))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(EmbeddingError::Sqlite(e)),
        }
    }

    /// Store an embedding in the cache.
    ///
    /// Uses `INSERT OR REPLACE` to handle collisions gracefully.
    ///
    /// # Errors
    ///
    /// Returns an error if the dimension mismatches or the database write fails.
    pub fn put(&self, text: &str, embedding: &Embedding) -> EmbeddingResult<()> {
        if embedding.len() != self.dims {
            return Err(EmbeddingError::DimensionMismatch {
                expected: self.dims,
                actual: embedding.len(),
            });
        }

        let hash = Self::content_hash(text);
        let blob = embedding_to_bytes(embedding);
        let conn = self.conn.lock();

        conn.execute(
            "INSERT OR REPLACE INTO embedding_cache (hash, embedding) VALUES (?1, ?2)",
            rusqlite::params![hash, blob],
        )?;

        Ok(())
    }

    /// Check if a text is already cached.
    #[must_use]
    pub fn contains(&self, text: &str) -> bool {
        let hash = Self::content_hash(text);
        let conn = self.conn.lock();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM embedding_cache WHERE hash = ?1",
                rusqlite::params![hash],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count > 0
    }

    /// Number of cached entries.
    #[must_use]
    pub fn len(&self) -> usize {
        let conn = self.conn.lock();
        conn.query_row("SELECT COUNT(*) FROM embedding_cache", [], |row| {
            row.get(0)
        })
        .unwrap_or(0)
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all cached embeddings.
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    pub fn clear(&self) -> EmbeddingResult<()> {
        let conn = self.conn.lock();
        conn.execute_batch("DELETE FROM embedding_cache")?;
        Ok(())
    }
}

/// Serialize an embedding to bytes (little-endian f32).
fn embedding_to_bytes(embedding: &Embedding) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// Deserialize bytes to an embedding (little-endian f32).
fn bytes_to_embedding(bytes: &[u8], expected_dims: usize) -> EmbeddingResult<Embedding> {
    if bytes.len() != expected_dims * 4 {
        return Err(EmbeddingError::DimensionMismatch {
            expected: expected_dims,
            actual: bytes.len() / 4,
        });
    }

    let mut embedding = Vec::with_capacity(expected_dims);
    for chunk in bytes.chunks_exact(4) {
        let val = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        embedding.push(val);
    }
    Ok(embedding)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_put_get() {
        let cache = EmbeddingCache::in_memory(4).expect("cache");
        let emb = vec![0.1, 0.2, 0.3, 0.4];
        cache.put("hello", &emb).expect("put");
        let got = cache.get("hello").expect("get").expect("found");
        assert_eq!(got, emb);
    }

    #[test]
    fn test_cache_miss() {
        let cache = EmbeddingCache::in_memory(4).expect("cache");
        let result = cache.get("nonexistent").expect("get");
        assert!(result.is_none());
    }

    #[test]
    fn test_cache_contains() {
        let cache = EmbeddingCache::in_memory(4).expect("cache");
        assert!(!cache.contains("hello"));
        cache
            .put("hello", &vec![1.0, 2.0, 3.0, 4.0])
            .expect("put");
        assert!(cache.contains("hello"));
    }

    #[test]
    fn test_cache_len() {
        let cache = EmbeddingCache::in_memory(4).expect("cache");
        assert_eq!(cache.len(), 0);
        cache
            .put("a", &vec![1.0, 2.0, 3.0, 4.0])
            .expect("put");
        cache
            .put("b", &vec![5.0, 6.0, 7.0, 8.0])
            .expect("put");
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_cache_clear() {
        let cache = EmbeddingCache::in_memory(4).expect("cache");
        cache
            .put("a", &vec![1.0, 2.0, 3.0, 4.0])
            .expect("put");
        cache.clear().expect("clear");
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_dimension_mismatch() {
        let cache = EmbeddingCache::in_memory(4).expect("cache");
        let result = cache.put("a", &vec![1.0, 2.0]);
        assert!(result.is_err());
    }

    #[test]
    fn test_content_hash_deterministic() {
        let h1 = EmbeddingCache::content_hash("hello");
        let h2 = EmbeddingCache::content_hash("hello");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_content_hash_different() {
        let h1 = EmbeddingCache::content_hash("hello");
        let h2 = EmbeddingCache::content_hash("world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_cache_overwrite() {
        let cache = EmbeddingCache::in_memory(4).expect("cache");
        cache
            .put("a", &vec![1.0, 2.0, 3.0, 4.0])
            .expect("put");
        cache
            .put("a", &vec![5.0, 6.0, 7.0, 8.0])
            .expect("overwrite");
        let got = cache.get("a").expect("get").expect("found");
        assert_eq!(got, vec![5.0, 6.0, 7.0, 8.0]);
    }
}
