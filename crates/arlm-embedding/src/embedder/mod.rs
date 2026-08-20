use std::fs::File;
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use thiserror::Error;

pub mod batch;
pub mod bge_m3;
pub mod cache;
pub mod config;
pub mod fallback;
pub mod lightweight;

pub use config::{build_embedder, EmbeddingConfig, EmbeddingModel, Quantization};
pub use lightweight::LightweightEmbedder;

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

/// Truncate (or zero-pad) an embedding to `dims` dimensions.
///
/// Implements Matryoshka representation truncation: keeps the first `dims`
/// components, or zero-pads if the input is shorter. This is a pure function
/// with no model dependency.
#[must_use]
pub fn matryoshka_truncate(emb: &[f32], dims: usize) -> Vec<f32> {
    if emb.len() >= dims {
        emb[..dims].to_vec()
    } else {
        let mut out = vec![0.0_f32; dims];
        out[..emb.len()].copy_from_slice(emb);
        out
    }
}

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

/// A file memory-mapped for zero-copy reading.
///
/// Uses `memmap2` to map the file into virtual memory without loading it
/// entirely into RAM. The OS manages paging — only accessed pages are
/// physically loaded. The `&str` returned by [`content()`](Self::content)
/// borrows directly from the mmap buffer.
pub struct OwnedFile {
    _mmap: Mmap,
    path: PathBuf,
    language: Option<String>,
    content: &'static str,
}

impl OwnedFile {
    /// Memory-map a file and detect its language.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened or is not valid UTF-8.
    #[allow(unsafe_code)]
    pub fn new(path: &Path) -> Result<Self, EmbeddingError> {
        let file = File::open(path)?;
        // SAFETY: we open the file read-only and the indexing pipeline never
        // writes to source files. The mmap is kept alive by OwnedFile.
        let mmap = unsafe { Mmap::map(&file)? };

        let content = std::str::from_utf8(&mmap).map_err(|_| EmbeddingError::NotUtf8(path.to_path_buf()))?;

        let language = crate::chunker::detect_language(path);

        tracing::info!(
            path = %path.display(),
            bytes = mmap.len(),
            language = language.as_deref().unwrap_or("unknown"),
            "loaded file (mmap)"
        );

        // SAFETY: `content` borrows from `mmap` which lives in `_mmap` field.
        // The `&'static str` is safe because the mmap outlives this struct,
        // and we never mutate the underlying mapping.
        let content_static: &'static str = unsafe { std::mem::transmute(content) };

        Ok(Self {
            _mmap: mmap,
            path: path.to_path_buf(),
            language,
            content: content_static,
        })
    }

    /// The file content as a string slice (zero-copy, borrows from mmap).
    #[must_use]
    pub fn content(&self) -> &str {
        self.content
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
#[allow(clippy::expect_used, clippy::unwrap_used)]
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
        std::fs::write(&file_path, [0xFF, 0xFE, 0x00]).expect("write");

        let result = OwnedFile::new(&file_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_embedder_trait_name() {
        let embedder = fallback::FallbackEmbedder::new(128);
        assert_eq!(embedder.name(), "fallback-hash");
    }
}
