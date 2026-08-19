use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use arlm_storage::Storage;
use arlm_storage::sqlite::chunks::NewChunk;

use crate::ScopedTimer;

/// Options for indexing knowledge into a project.
#[derive(Debug, Clone)]
pub struct IndexOptions {
    /// Maximum bytes per chunk.
    pub max_chunk_bytes: usize,
    /// Embedding model name.
    pub embedding_model: String,
    /// Embedding dimensions.
    pub embedding_dims: i64,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            max_chunk_bytes: 1500,
            embedding_model: "bge-m3".to_string(),
            embedding_dims: 1024,
        }
    }
}

/// Result of an indexing operation.
#[derive(Debug, Clone)]
pub struct IndexResult {
    pub files_processed: u64,
    pub chunks_created: u64,
    pub duration_ms: u128,
}

/// The knowledge engine handles indexing files and retrieving content.
pub struct KnowledgeEngine {
    storage: Storage,
}

impl KnowledgeEngine {
    /// Create a new `KnowledgeEngine`.
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    /// Get the underlying storage.
    #[must_use]
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// Index a directory into a project buffer.
    ///
    /// Reads files, chunks them by byte size, and stores metadata in `SQLite`.
    /// Does NOT compute embeddings (that requires the embedding crate).
    ///
    /// # Errors
    ///
    /// Returns an error if directory reading or database operations fail.
    pub fn index_directory(
        &self,
        project_name: &str,
        dir_path: &Path,
        options: &IndexOptions,
    ) -> Result<IndexResult> {
        let _timer = ScopedTimer::new("knowledge_index_directory");
        let start = std::time::Instant::now();

        let buffer = self
            .storage
            .get_buffer_by_name(project_name)
            .context("failed to find project")?
            .context("project not found")?;

        let mut files_processed: u64 = 0;
        let mut chunks_created: u64 = 0;

        let entries = std::fs::read_dir(dir_path)
            .with_context(|| format!("failed to read directory: {}", dir_path.display()))?;

        for entry in entries {
            let entry = entry.context("failed to read dir entry")?;
            let path = entry.path();

            // Skip non-files, hidden files, and SQLite internals
            if !path.is_file() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') || name.ends_with("-shm") || name.ends_with("-wal") {
                    continue;
                }
            }

            self.index_file(buffer.id, &path, options)?;
            files_processed += 1;

            let chunks = Self::count_file_chunks(&path, options.max_chunk_bytes);
            chunks_created += chunks;
        }

        self.storage
            .update_buffer_counts(
                buffer.id,
                i64::try_from(chunks_created).unwrap_or(i64::MAX),
                i64::try_from(files_processed).unwrap_or(i64::MAX),
            )
            .context("failed to update buffer counts")?;

        let duration = start.elapsed();
        tracing::info!(
            project = project_name,
            files = files_processed,
            chunks = chunks_created,
            elapsed_ms = duration.as_millis(),
            "directory indexed"
        );

        Ok(IndexResult {
            files_processed,
            chunks_created,
            duration_ms: duration.as_millis(),
        })
    }

    /// Index a single file into chunks.
    ///
    /// # Errors
    ///
    /// Returns an error if file reading or database operations fail.
    pub fn index_file(
        &self,
        buffer_id: i64,
        file_path: &Path,
        options: &IndexOptions,
    ) -> Result<Vec<i64>> {
        let content = std::fs::read_to_string(file_path)
            .with_context(|| format!("failed to read file: {}", file_path.display()))?;

        let path_str = file_path.to_str().context("file path is not valid UTF-8")?;

        let bytes = content.as_bytes();
        let chunk_size = options.max_chunk_bytes;
        let mut chunk_ids = Vec::new();
        let mut offset = 0;
        let mut line_start = 1;

        while offset < bytes.len() {
            let end = (offset + chunk_size).min(bytes.len());

            // Find a line boundary near the end to avoid splitting mid-line
            let chunk_end = if end < bytes.len() {
                find_line_boundary(bytes, end)
            } else {
                bytes.len()
            };

            let chunk_bytes = &bytes[offset..chunk_end];
            let chunk_text =
                std::str::from_utf8(chunk_bytes).context("chunk content is not valid UTF-8")?;

            let line_count = chunk_text.bytes().filter(|&b| b == b'\n').count();
            #[allow(clippy::cast_possible_wrap)]
            let line_end = line_start + line_count as i64;

            let hash = compute_hash(chunk_bytes);

            #[allow(clippy::cast_possible_wrap)]
            let chunk = NewChunk {
                buffer_id,
                file_path: path_str.to_string(),
                offset_start: offset as i64,
                offset_end: chunk_end as i64,
                line_start,
                line_end,
                hash,
                language: detect_language(file_path),
                chunk_type: None,
                token_count: Some(estimate_tokens(chunk_text)),
            };

            let chunk_id = self
                .storage
                .insert_chunk(&chunk)
                .context("failed to insert chunk")?;

            self.storage
                .insert_chunk_content(chunk_id, chunk_text)
                .context("failed to insert chunk content")?;

            chunk_ids.push(chunk_id);

            offset = chunk_end;
            line_start = line_end + 1;
        }

        tracing::debug!(file = path_str, chunks = chunk_ids.len(), "file indexed");

        Ok(chunk_ids)
    }

    /// Retrieve chunk content by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_chunk_content(&self, chunk_id: i64) -> Result<Option<String>> {
        self.storage
            .get_chunk_content(chunk_id)
            .context("failed to get chunk content")
    }

    /// List all chunks for a project.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn list_chunks(&self, buffer_id: i64) -> Result<Vec<arlm_storage::sqlite::chunks::Chunk>> {
        self.storage
            .list_chunks(buffer_id)
            .context("failed to list chunks")
    }

    fn count_file_chunks(path: &Path, max_chunk_bytes: usize) -> u64 {
        std::fs::read(path).map_or(0, |bytes| {
            let len = bytes.len();
            if len == 0 {
                1
            } else {
                ((len - 1) / max_chunk_bytes + 1) as u64
            }
        })
    }
}

fn find_line_boundary(bytes: &[u8], near: usize) -> usize {
    // Search backwards from `near` for a newline
    let search_start = near.saturating_sub(100);
    for i in (search_start..near).rev() {
        if bytes[i] == b'\n' {
            return i + 1;
        }
    }
    near
}

fn compute_hash(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

fn detect_language(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| match ext {
            "rs" => "rust",
            "py" => "python",
            "js" => "javascript",
            "ts" => "typescript",
            "tsx" => "typescriptreact",
            "jsx" => "javascriptreact",
            "go" => "go",
            "java" => "java",
            "c" | "h" | "hpp" => "c",
            "cpp" | "cc" | "cxx" => "cpp",
            "rb" => "ruby",
            "sh" | "bash" => "shell",
            "sql" => "sql",
            "md" => "markdown",
            "json" => "json",
            "yaml" | "yml" => "yaml",
            "toml" => "toml",
            "html" => "html",
            "css" => "css",
            _ => "",
        })
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn estimate_tokens(text: &str) -> i64 {
    // Rough estimate: ~4 chars per token
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    {
        ((text.len() as f64 / 4.0).ceil() as i64).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlm_storage::sqlite::buffers::NewBuffer;
    use tempfile::TempDir;

    fn setup() -> (KnowledgeEngine, TempDir) {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::open(tmp.path()).unwrap();
        (KnowledgeEngine::new(storage), tmp)
    }

    fn create_project(engine: &KnowledgeEngine, name: &str) -> i64 {
        engine
            .storage()
            .insert_buffer(&NewBuffer {
                name: name.to_string(),
                path: "/tmp/test".to_string(),
            })
            .unwrap()
    }

    #[test]
    fn test_index_file() {
        let (engine, tmp) = setup();
        let buffer_id = create_project(&engine, "test");

        let file_path = tmp.path().join("test.rs");
        std::fs::write(&file_path, "fn main() {\n    println!(\"hello\");\n}").unwrap();

        let opts = IndexOptions::default();
        let ids = engine.index_file(buffer_id, &file_path, &opts).unwrap();
        assert!(!ids.is_empty());

        let content = engine.get_chunk_content(ids[0]).unwrap().unwrap();
        assert!(content.contains("fn main"));
    }

    #[test]
    fn test_index_directory() {
        let (engine, tmp) = setup();
        create_project(&engine, "test");

        // Use a subdirectory for test files to avoid SQLite internals
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();

        let file1 = src_dir.join("a.rs");
        let file2 = src_dir.join("b.py");
        std::fs::write(&file1, "fn main() {}").unwrap();
        std::fs::write(&file2, "print('hello')").unwrap();

        let opts = IndexOptions::default();
        let result = engine.index_directory("test", &src_dir, &opts).unwrap();

        assert_eq!(result.files_processed, 2);
        assert!(result.chunks_created >= 2);
    }

    #[test]
    fn test_detect_language() {
        assert_eq!(
            detect_language(Path::new("main.rs")),
            Some("rust".to_string())
        );
        assert_eq!(
            detect_language(Path::new("app.py")),
            Some("python".to_string())
        );
        assert_eq!(
            detect_language(Path::new("index.html")),
            Some("html".to_string())
        );
        assert_eq!(detect_language(Path::new("unknown")), None);
    }

    #[test]
    fn test_compute_hash() {
        let h1 = compute_hash(b"hello");
        let h2 = compute_hash(b"hello");
        let h3 = compute_hash(b"world");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_estimate_tokens() {
        assert!(estimate_tokens("hello world") > 0);
        assert!(estimate_tokens("") >= 1);
    }
}
