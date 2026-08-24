use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::{Embedder, Embedding, EmbeddingError, EmbeddingResult};

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

        let mut stmt = conn.prepare("SELECT embedding FROM embedding_cache WHERE hash = ?1")?;

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
        conn.query_row("SELECT COUNT(*) FROM embedding_cache", [], |row| row.get(0))
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

/// An [`Embedder`] wrapped with an SQLite-backed content-hash cache.
///
/// Cache hits skip model inference entirely; misses are computed by the
/// inner embedder and stored. Cache I/O failures degrade gracefully to a
/// plain pass-through (logged, never fatal). Enabled via
/// `server.toml [embedder] cache = true` (plan 020).
pub struct CachedEmbedder {
    inner: Arc<dyn Embedder + Send + Sync>,
    cache: EmbeddingCache,
}

impl CachedEmbedder {
    /// Wrap `inner` with `cache`.
    #[must_use]
    pub fn new(inner: Arc<dyn Embedder + Send + Sync>, cache: EmbeddingCache) -> Self {
        Self { inner, cache }
    }

    /// The backing cache (for stats/inspection).
    #[must_use]
    pub fn cache(&self) -> &EmbeddingCache {
        &self.cache
    }
}

impl Embedder for CachedEmbedder {
    fn embed(&self, text: &str) -> EmbeddingResult<Embedding> {
        if let Ok(Some(emb)) = self.cache.get(text) {
            return Ok(emb);
        }
        let emb = self.inner.embed(text)?;
        if let Err(e) = self.cache.put(text, &emb) {
            tracing::warn!(error = %e, "embedding cache put failed");
        }
        Ok(emb)
    }

    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Embedding>> {
        let mut out: Vec<Embedding> = Vec::with_capacity(texts.len());
        let mut miss_idx: Vec<usize> = Vec::new();
        for (i, t) in texts.iter().enumerate() {
            if let Ok(Some(emb)) = self.cache.get(t) {
                out.push(emb);
            } else {
                out.push(Vec::new());
                miss_idx.push(i);
            }
        }
        if !miss_idx.is_empty() {
            let missed: Vec<&str> = miss_idx.iter().map(|&i| texts[i]).collect();
            let computed = self.inner.embed_batch(&missed)?;
            for (j, emb) in computed.into_iter().enumerate() {
                let i = miss_idx[j];
                if let Err(e) = self.cache.put(texts[i], &emb) {
                    tracing::warn!(error = %e, "embedding cache put failed");
                }
                out[i] = emb;
            }
        }
        Ok(out)
    }

    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }
}

#[cfg(test)]
mod cached_tests {
    use super::*;
    use crate::embedder::lightweight::LightweightEmbedder;

    #[test]
    fn test_cached_embedder_hit_skips_inner_and_reuses() {
        let cache = EmbeddingCache::in_memory(8).unwrap();
        let wrapped = Arc::new(LightweightEmbedder::new(8));
        let emb = CachedEmbedder::new(wrapped.clone(), cache);

        let first = emb.embed("hello world").unwrap();
        assert_eq!(first.len(), 8);
        assert!(emb.cache().contains("hello world"));
        // Second call is served from the cache and must be identical.
        let second = emb.embed("hello world").unwrap();
        assert_eq!(first, second);
        assert_eq!(emb.name(), wrapped.name());
    }

    #[test]
    fn test_cached_embedder_batch_mixed_hits() {
        let cache = EmbeddingCache::in_memory(8).unwrap();
        let emb = CachedEmbedder::new(Arc::new(LightweightEmbedder::new(8)), cache);

        let warm = emb.embed("a").unwrap();
        let batch = emb.embed_batch(&["a", "b"]).unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0], warm); // hit
        assert_eq!(batch[1].len(), 8); // computed then stored
        assert!(emb.cache().contains("b"));
    }
}
