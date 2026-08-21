//! Complete ingestion pipeline: file → chunks → embeddings.

pub mod files;

pub use files::{
    compress_text, compute_hash, discover_files, glob_match, is_text_file, path_force_matches,
};

use std::path::PathBuf;
use std::sync::Arc;

use rayon::prelude::*;

use crate::chunker::code::CodeChunker;
use crate::chunker::markdown::MarkdownChunker;
use crate::chunker::recursive::RecursiveChunker;
use crate::chunker::text::TextChunker;
use crate::chunker::{ChunkingStrategy, RawChunk};
use crate::embedder::cache::EmbeddingCache;
use crate::embedder::config::EmbeddingConfig;
use crate::embedder::{
    Embedder, Embedding, EmbeddingError, EmbeddingResult, OwnedFile, build_embedder,
};

/// A chunk ready for embedding, with owned data.
pub struct ChunkedText {
    pub file_path: PathBuf,
    pub offset_start: usize,
    pub offset_end: usize,
    pub line_start: usize,
    pub line_end: usize,
    pub content: String,
    /// zstd-compressed `content` when `IngestOptions::compress` is set.
    pub compressed: Option<Vec<u8>>,
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
    /// Whether to use zstd compression for stored text.
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
    code: Box<dyn ChunkingStrategy>,
    text: Box<dyn ChunkingStrategy>,
    markdown: Box<dyn ChunkingStrategy>,
    recursive: Box<dyn ChunkingStrategy>,
    embedder: Arc<dyn Embedder>,
    cache: Option<Arc<EmbeddingCache>>,
    batch_size: usize,
}

impl IngestionPipeline {
    /// Create a new pipeline with the given embedder and default strategies.
    #[must_use]
    pub fn new(embedder: Arc<dyn Embedder>, cache: Option<EmbeddingCache>) -> Self {
        let _timer = crate::Timer::new("pipeline_new");
        Self {
            code: Box::new(CodeChunker::new(512, 64)),
            text: Box::new(TextChunker::new(512, 64)),
            markdown: Box::new(MarkdownChunker::new(512)),
            recursive: Box::new(RecursiveChunker::new(512, 64)),
            embedder,
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

    /// The configured batch size for embedding inference.
    #[must_use]
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Create a pipeline from an [`EmbeddingConfig`], building the embedder
    /// (BGE-M3 or lightweight) accordingly.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured embedder cannot be built.
    pub fn from_config(
        config: &EmbeddingConfig,
        cache: Option<EmbeddingCache>,
    ) -> anyhow::Result<Self> {
        let embedder = build_embedder(config)?;
        Ok(Self::new(embedder, cache))
    }

    /// Process a list of files through the chunking and embedding pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error if file reading, chunking, or embedding fails.
    pub fn ingest(
        &self,
        files: &[PathBuf],
        options: &IngestOptions,
    ) -> EmbeddingResult<IngestResult> {
        let _timer = crate::Timer::new("pipeline_ingest");
        let total_files = files.len();

        tracing::info!(total_files = total_files, "starting ingestion pipeline");

        // Phase 1: Read files (parallel via rayon).
        // A single unreadable file (e.g. invalid UTF-8) must not abort the
        // whole index — skip it with a warning instead.
        let owned_files: Vec<OwnedFile> = files
            .par_iter()
            .map(|p| match OwnedFile::new(p) {
                Ok(f) => Some(f),
                Err(e) => {
                    tracing::warn!(
                        path = %p.display(),
                        error = %e,
                        "skipping file that could not be read"
                    );
                    None
                }
            })
            .flatten()
            .collect();

        tracing::info!(files_read = owned_files.len(), "finished reading files");

        // Phase 2: Chunk files (parallel via rayon)
        let all_chunks: Vec<ChunkedText> = owned_files
            .par_iter()
            .flat_map(|of| {
                let strategy = self.select_strategy(of.language_hint());
                let raw_chunks = strategy.chunk(of.content(), of.path());
                to_chunked_texts(of, raw_chunks, options)
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

    /// Select the appropriate chunking strategy based on the language hint.
    ///
    /// All four strategies are always present (inserted in `new`), so this
    /// returns a reference without fallible lookup.
    fn select_strategy(&self, language: &str) -> &dyn ChunkingStrategy {
        match language {
            "rust" | "python" | "javascript" | "typescript" | "go" | "java" | "cpp" | "c"
            | "ruby" | "php" => self.code.as_ref(),
            "markdown" => self.markdown.as_ref(),
            "text" => self.text.as_ref(),
            _ => self.recursive.as_ref(),
        }
    }

    /// Embed texts using cache when available.
    fn embed_batch_cached(&self, texts: &[&str]) -> EmbeddingResult<Vec<Embedding>> {
        if let Some(ref cache) = self.cache {
            let mut results = Vec::with_capacity(texts.len());
            let mut uncached = Vec::new();
            let mut uncached_indices = Vec::new();

            for (i, text) in texts.iter().enumerate() {
                if let Some(emb) = cache.get(text)? {
                    results.push(Some(emb));
                } else {
                    results.push(None);
                    uncached_indices.push(i);
                    uncached.push(*text);
                }
            }

            if !uncached.is_empty() {
                let _timer = crate::Timer::new("batch_embed_uncached");
                let mut embeddings = Vec::with_capacity(uncached.len());
                for chunk in uncached.chunks(self.batch_size) {
                    let embs = self.embedder.embed_batch(chunk)?;
                    embeddings.extend(embs);
                }

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

/// Convert raw chunks to owned `ChunkedText` with hashes (and optional
/// zstd compression when `options.compress` is set).
fn to_chunked_texts(
    owned_file: &OwnedFile,
    raw_chunks: Vec<RawChunk<'_>>,
    options: &IngestOptions,
) -> Vec<ChunkedText> {
    raw_chunks
        .into_iter()
        .map(|rc| {
            let content = rc.content.into_owned();
            let compressed = if options.compress {
                Some(compress_text(&content))
            } else {
                None
            };
            let hash = compute_hash(&content);
            ChunkedText {
                file_path: owned_file.path().to_path_buf(),
                offset_start: rc.offset_start,
                offset_end: rc.offset_end,
                line_start: rc.line_start,
                line_end: rc.line_end,
                content,
                compressed,
                language: rc.language,
                chunk_type: rc.chunk_type,
                hash,
            }
        })
        .collect()
}
