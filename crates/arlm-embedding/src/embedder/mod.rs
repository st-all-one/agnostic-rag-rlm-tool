use std::path::{Path, PathBuf};

use thiserror::Error;

pub mod batch;
pub mod bge_m3;
pub mod cache;
pub mod fallback;

/// Errors specific to the embedding subsystem.
#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("failed to open file: {0}")]
    FileOpen(#[from] std::io::Error),

    #[error("file is not valid UTF-8: {0}")]
    NotUtf8(PathBuf),

    #[error("candle error: {0}")]
    Candle(String),

    #[error("tokenizer error: {0}")]
    Tokenizer(String),

    #[error("model not loaded: {0}")]
    ModelNotLoaded(String),

    #[error("dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("embedding cache miss")]
    CacheMiss,
}

/// Result type for embedding operations.
pub type EmbeddingResult<T> = Result<T, EmbeddingError>;

/// An embedding vector.
pub type Embedding = Vec<f32>;

/// Trait for text embedding models.
pub trait Embedder: Send + Sync {
    /// Embed a single text string, returning a normalized embedding vector.
    ///
    /// # Errors
    ///
    /// Returns an error if tokenization or model inference fails.
    fn embed(&self, text: &str) -> EmbeddingResult<Embedding>;

    /// Embed multiple texts in a batch, returning one embedding per input.
    ///
    /// # Errors
    ///
    /// Returns an error if tokenization or model inference fails.
    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Embedding>>;

    /// The dimensionality of the embedding vectors.
    fn dimensions(&self) -> usize;

    /// A human-readable name for this embedder.
    fn name(&self) -> &'static str;
}

/// A file loaded into owned memory.
///
/// Reads the file content into a `String`. For production use with very large
/// files, consider enabling `unsafe_code` at the crate level and using `memmap2`
/// for zero-copy memory-mapped I/O.
pub struct OwnedFile {
    path: PathBuf,
    language: Option<String>,
    content: String,
}

impl OwnedFile {
    /// Load a file and detect its language.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or is not valid UTF-8.
    pub fn new(path: &Path) -> Result<Self, EmbeddingError> {
        let content = std::fs::read_to_string(path)?;

        let language = crate::chunker::detect_language(path);

        tracing::info!(
            path = %path.display(),
            bytes = content.len(),
            language = language.as_deref().unwrap_or("unknown"),
            "loaded file"
        );

        Ok(Self {
            path: path.to_path_buf(),
            language,
            content,
        })
    }

    /// The file content as a string slice.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// The file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The detected language hint (e.g., "rust", "python").
    #[must_use]
    pub fn language_hint(&self) -> &str {
        self.language.as_deref().unwrap_or("text")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_owned_file_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, "fn main() {}").expect("write");

        let owned = OwnedFile::new(&file_path).expect("OwnedFile::new");
        assert_eq!(owned.content(), "fn main() {}");
        assert_eq!(owned.language_hint(), "rust");
        assert_eq!(owned.path(), file_path.as_path());
    }

    #[test]
    fn test_owned_file_not_utf8() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("binary.bin");
        std::fs::write(&file_path, &[0xFF, 0xFE, 0x00]).expect("write");

        let result = OwnedFile::new(&file_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_embedder_trait_name() {
        let embedder = fallback::FallbackEmbedder::new(128);
        assert_eq!(embedder.name(), "fallback-hash");
    }
}
