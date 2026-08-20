use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use sha2::{Digest, Sha256};
use zstd::stream::encode_all;

use crate::chunker::code::CodeChunker;
use crate::chunker::markdown::MarkdownChunker;
use crate::chunker::recursive::RecursiveChunker;
use crate::chunker::text::TextChunker;
use crate::chunker::{ChunkingStrategy, RawChunk};
use crate::embedder::cache::EmbeddingCache;
use crate::embedder::config::EmbeddingConfig;
use crate::embedder::{build_embedder, Embedder, Embedding, EmbeddingError, EmbeddingResult, OwnedFile};

/// A chunk ready for embedding, with owned data.
pub struct ChunkedText {
    pub file_path: PathBuf,
    pub offset_start: usize,
    pub offset_end: usize,
    pub line_start: usize,
    pub line_end: usize,
    pub content: String,
    pub language: Option<String>,
    pub chunk_type: Option<String>,
    pub hash: String,
}

/// A chunk with its computed embedding.
pub struct EmbeddedChunk {
    pub chunk: ChunkedText,
    pub embedding: Embedding,
}

/// Options for the ingestion pipeline.
pub struct IngestOptions {
    /// Maximum tokens per chunk.
    pub max_tokens: usize,
    /// Overlap tokens for chunking strategies.
    pub overlap_tokens: usize,
    /// Batch size for embedding inference.
    pub batch_size: usize,
    /// Whether to use compression for stored text.
    pub compress: bool,
}

impl Default for IngestOptions {
    fn default() -> Self {
        Self {
            max_tokens: 512,
            overlap_tokens: 64,
            batch_size: 64,
            compress: true,
        }
    }
}

/// Result of a pipeline ingestion run.
pub struct IngestResult {
    pub total_files: usize,
    pub total_chunks: usize,
    pub total_embedded: usize,
}

/// Complete ingestion pipeline: file → chunks → embeddings.
pub struct IngestionPipeline {
    strategies: HashMap<&'static str, Box<dyn ChunkingStrategy>>,
    embedder: Arc<dyn Embedder>,
    cache: Option<Arc<EmbeddingCache>>,
    batch_size: usize,
}

impl IngestionPipeline {
    /// Create a new pipeline with the given embedder and default strategies.
    #[must_use]
    pub fn new(embedder: Arc<dyn Embedder>, cache: Option<EmbeddingCache>) -> Self {
        let mut strategies: HashMap<&'static str, Box<dyn ChunkingStrategy>> = HashMap::new();
        strategies.insert("code", Box::new(CodeChunker::new(512, 64)));
        strategies.insert("text", Box::new(TextChunker::new(512, 64)));
        strategies.insert("markdown", Box::new(MarkdownChunker::new(512)));
        strategies.insert("recursive", Box::new(RecursiveChunker::new(512, 64)));

        Self {
            embedder,
            strategies,
            cache: cache.map(Arc::new),
            batch_size: 64,
        }
    }

    /// Set the batch size for embedding inference.
    #[must_use]
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Create a pipeline from an [`EmbeddingConfig`], building the embedder
    /// (BGE-M3 or lightweight) accordingly.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured embedder cannot be built.
    pub fn from_config(config: &EmbeddingConfig, cache: Option<EmbeddingCache>) -> anyhow::Result<Self> {
        let embedder = build_embedder(config)?;
        Ok(Self::new(embedder, cache))
    }

    /// Process a list of files through the chunking and embedding pipeline.
    ///
    /// # Arguments
    ///
    /// * `files` - Paths to files to process.
    /// * `_options` - Ingestion options.
    ///
    /// # Errors
    ///
    /// Returns an error if file reading, chunking, or embedding fails.
    pub fn ingest(
        &self,
        files: &[PathBuf],
        _options: &IngestOptions,
    ) -> EmbeddingResult<IngestResult> {
        let _timer = crate::Timer::new("pipeline_ingest");
        let total_files = files.len();

        tracing::info!(total_files = total_files, "starting ingestion pipeline");

        // Phase 1: Read files (parallel via rayon)
        let owned_files: Vec<OwnedFile> = files
            .par_iter()
            .map(|p| OwnedFile::new(p))
            .collect::<Result<Vec<_>, _>>()?;

        tracing::info!(files_read = owned_files.len(), "finished reading files");

        // Phase 2: Chunk files (parallel via rayon)
        let all_chunks: Vec<ChunkedText> = owned_files
            .par_iter()
            .flat_map(|of| {
                let strategy = Self::select_strategy_static(&self.strategies, of.language_hint());
                let raw_chunks = strategy.chunk(of.content(), of.path());
                to_chunked_texts(of, raw_chunks)
            })
            .collect();

        let total_chunks = all_chunks.len();
        tracing::info!(total_chunks = total_chunks, "finished chunking");

        // Phase 3: Embed chunks (batch inference)
        let chunk_refs: Vec<&str> = all_chunks.iter().map(|c| &*c.content).collect();
        let embeddings = self.embed_batch_cached(&chunk_refs)?;

        let total_embedded = embeddings.len();
        tracing::info!(total_embedded = total_embedded, "finished embedding");

        let _embedded: Vec<EmbeddedChunk> = all_chunks
            .into_iter()
            .zip(embeddings)
            .map(|(chunk, embedding)| EmbeddedChunk { chunk, embedding })
            .collect();

        Ok(IngestResult {
            total_files,
            total_chunks,
            total_embedded,
        })
    }

    /// Select the appropriate chunking strategy based on language hint.
    ///
    /// All strategies are guaranteed to be present (inserted in `new`).
    fn select_strategy_static<'a>(
        strategies: &'a HashMap<&'static str, Box<dyn ChunkingStrategy>>,
        language: &str,
    ) -> &'a dyn ChunkingStrategy {
        // SAFETY: all keys are inserted in `IngestionPipeline::new`
        #[allow(clippy::unwrap_used)]
        match language {
            "rust" | "python" | "javascript" | "typescript" | "go" | "java" | "cpp" | "c"
            | "ruby" | "php" => strategies.get("code").unwrap().as_ref(),
            "markdown" => strategies.get("markdown").unwrap().as_ref(),
            "text" => strategies.get("text").unwrap().as_ref(),
            _ => strategies.get("recursive").unwrap().as_ref(),
        }
    }

    /// Embed texts using cache when available.
    fn embed_batch_cached(&self, texts: &[&str]) -> EmbeddingResult<Vec<Embedding>> {
        if let Some(ref cache) = self.cache {
            let mut results = Vec::with_capacity(texts.len());
            let mut uncached = Vec::new();
            let mut uncached_indices = Vec::new();

            // Phase 1: check cache
            for (i, text) in texts.iter().enumerate() {
                if let Some(emb) = cache.get(text)? {
                    results.push(Some(emb));
                } else {
                    results.push(None);
                    uncached_indices.push(i);
                    uncached.push(*text);
                }
            }

            // Phase 2: batch embed uncached texts
            if !uncached.is_empty() {
                let _timer = crate::Timer::new("batch_embed_uncached");
                let mut embeddings = Vec::with_capacity(uncached.len());
                for chunk in uncached.chunks(self.batch_size) {
                    let embs = self.embedder.embed_batch(chunk)?;
                    embeddings.extend(embs);
                }

                // Phase 3: fill results and store in cache
                for (idx_in_uncached, &orig_idx) in uncached_indices.iter().enumerate() {
                    let emb = &embeddings[idx_in_uncached];
                    results[orig_idx] = Some(emb.clone());
                    cache.put(texts[orig_idx], emb)?;
                }
            }

            results
                .into_iter()
                .map(|opt| {
                    opt.ok_or_else(|| {
                        EmbeddingError::ModelNotLoaded("result slot not filled".into())
                    })
                })
                .collect::<EmbeddingResult<Vec<_>>>()
        } else {
            let mut all = Vec::with_capacity(texts.len());
            for chunk in texts.chunks(self.batch_size) {
                let embs = self.embedder.embed_batch(chunk)?;
                all.extend(embs);
            }
            Ok(all)
        }
    }
}

/// Convert raw chunks to owned `ChunkedText` with hashes.
fn to_chunked_texts(owned_file: &OwnedFile, raw_chunks: Vec<RawChunk<'_>>) -> Vec<ChunkedText> {
    raw_chunks
        .into_iter()
        .map(|rc| {
            let content = rc.content.into_owned();
            let hash = compute_hash(&content);
            ChunkedText {
                file_path: owned_file.path().to_path_buf(),
                offset_start: rc.offset_start,
                offset_end: rc.offset_end,
                line_start: rc.line_start,
                line_end: rc.line_end,
                content,
                language: rc.language,
                chunk_type: rc.chunk_type,
                hash,
            }
        })
        .collect()
}

/// Compress text using zstd.
///
/// Returns the compressed bytes.
#[must_use]
pub fn compress_text(text: &str) -> Vec<u8> {
    encode_all(text.as_bytes(), 0).unwrap_or_else(|_| text.as_bytes().to_vec())
}

/// Compute SHA-256 hash of text content.
#[must_use]
pub fn compute_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

/// Discover files in a directory recursively, respecting common ignore patterns.
///
/// # Arguments
///
/// * `root` - Root directory to search.
/// * `extra_ignores` - Additional glob patterns to ignore (e.g., `["*.log", "dist/"]`).
///
/// # Errors
///
/// Returns an error if the directory cannot be read.
pub fn discover_files(root: &Path, extra_ignores: &[String]) -> EmbeddingResult<Vec<PathBuf>> {
    let mut files = Vec::with_capacity(256);
    discover_files_recursive(root, extra_ignores, &mut files)?;
    Ok(files)
}

/// Default ignore patterns applied to all projects.
const DEFAULT_IGNORES: &[&str] = &[
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "*.jks",
];

fn discover_files_recursive(
    dir: &Path,
    extra_ignores: &[String],
    files: &mut Vec<PathBuf>,
) -> EmbeddingResult<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    let entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();

    for entry in &entries {
        let path = entry.path();

        // Skip hidden directories and common ignore patterns
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.')
                || name == "node_modules"
                || name == "target"
                || name == "vendor"
                || name == "__pycache__"
                || name == ".git"
            {
                continue;
            }

            // Check default ignore patterns
            let dominated = DEFAULT_IGNORES.iter().any(|pat| glob_match(pat, name));
            if dominated {
                continue;
            }

            // Check user-specified ignore patterns
            let dominated = extra_ignores.iter().any(|pat| glob_match(pat, name));
            if dominated {
                continue;
            }
        }

        if path.is_dir() {
            discover_files_recursive(&path, extra_ignores, files)?;
        } else if path.is_file() && is_text_file(&path) {
            files.push(path);
        }
    }

    Ok(())
}

/// Simple glob matching for single-component patterns.
///
/// Supports `*` wildcard matching against a filename.
#[must_use]
fn glob_match(pattern: &str, name: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.") {
        return name.ends_with(suffix) && name.len() > suffix.len() + 1;
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return name.starts_with(prefix) && name.len() > prefix.len() + 1;
    }
    name == pattern
}

/// Check if a file is likely text-based (by extension).
#[must_use]
pub fn is_text_file(path: &Path) -> bool {
    let text_extensions = [
        "rs", "py", "js", "jsx", "ts", "tsx", "go", "java", "cpp", "cc", "cxx", "c", "h", "rb",
        "php", "md", "txt", "log", "json", "yaml", "yml", "toml", "xml", "html", "css", "scss",
        "sql", "sh", "bash", "zsh", "fish", "vim", "el", "lisp", "r", "R", "jl", "swift", "kt",
        "scala", "ex", "exs", "erl", "hs", "ml", "clj", "lua", "pl", "pm",
    ];

    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| text_extensions.contains(&ext))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_and_hash() {
        let text = "hello world, this is a test of compression";
        let compressed = compress_text(text);
        assert!(compressed.len() <= text.len() + 10);

        let hash = compute_hash(text);
        assert_eq!(hash.len(), 64); // SHA-256 hex
    }

    #[test]
    fn test_compute_hash_deterministic() {
        let h1 = compute_hash("test");
        let h2 = compute_hash("test");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_is_text_file() {
        assert!(is_text_file(Path::new("main.rs")));
        assert!(is_text_file(Path::new("README.md")));
        assert!(is_text_file(Path::new("config.json")));
        assert!(!is_text_file(Path::new("image.png")));
        assert!(!is_text_file(Path::new("binary")));
    }

    #[test]
    fn test_discover_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.rs"), "fn main() {}").expect("write");
        std::fs::write(dir.path().join("b.py"), "print('hello')").expect("write");
        std::fs::write(dir.path().join("c.png"), b"binary").expect("write");
        std::fs::write(dir.path().join(".env"), "SECRET=1").expect("write");
        std::fs::write(dir.path().join("key.pem"), "-----").expect("write");

        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).expect("mkdir");
        std::fs::write(sub.join("d.txt"), "text").expect("write");

        let files = discover_files(dir.path(), &[]).expect("discover");
        assert_eq!(files.len(), 3); // a.rs, b.py, sub/d.txt (filtered: .env, key.pem, c.png)
    }

    #[test]
    fn test_discover_files_custom_ignore() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.rs"), "fn main() {}").expect("write");
        std::fs::write(dir.path().join("b.log"), "log entry").expect("write");
        std::fs::write(dir.path().join("c.rs"), "fn foo() {}").expect("write");

        let ignores = vec!["*.log".to_string()];
        let files = discover_files(dir.path(), &ignores).expect("discover");
        assert_eq!(files.len(), 2); // a.rs, c.rs (filtered: b.log)
    }

    #[test]
    fn test_glob_match() {
        assert!(glob_match("*.pem", "server.pem"));
        assert!(glob_match("*.pem", "key.pem"));
        assert!(!glob_match("*.pem", "pem.txt"));
        assert!(glob_match(".env.*", ".env.local"));
        assert!(!glob_match(".env.*", ".env"));
        assert!(glob_match(".env", ".env"));
        assert!(!glob_match(".env", ".env.local"));
    }

    #[test]
    fn test_pipeline_new() {
        use crate::embedder::fallback::FallbackEmbedder;
        let embedder = Arc::new(FallbackEmbedder::new(128));
        let pipeline = IngestionPipeline::new(embedder, None);
        assert_eq!(pipeline.batch_size, 64);
    }

    #[test]
    fn test_pipeline_from_config_lightweight() {
        use crate::embedder::config::EmbeddingConfig;
        let config = EmbeddingConfig::for_tests();
        let pipeline = IngestionPipeline::from_config(&config, None).expect("pipeline");
        assert_eq!(pipeline.batch_size, 64);
    }

    #[test]
    fn test_pipeline_ingest() {
        use crate::embedder::fallback::FallbackEmbedder;

        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, "fn main() {\n    println!(\"hello\");\n}").expect("write");

        let embedder = Arc::new(FallbackEmbedder::new(128));
        let pipeline = IngestionPipeline::new(embedder, None);
        let options = IngestOptions::default();
        let result = pipeline.ingest(&[file_path], &options).expect("ingest");

        assert_eq!(result.total_files, 1);
        assert!(result.total_chunks >= 1);
        assert!(result.total_embedded >= 1);
    }
}
