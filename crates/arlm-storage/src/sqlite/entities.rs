use std::sync::LazyLock;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::params;

use super::conn::Storage;

const MAX_ENTITIES_PER_CHUNK: usize = 10;

// SAFETY: All regex patterns below are compile-time constants validated at
// first access. The patterns use standard regex syntax and are known to be
// valid. If a pattern were invalid, the `LazyLock` initialization would
// panic on first access, which is the desired behavior for programmer errors.
#[allow(clippy::unwrap_used)]
fn compile_regex(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap()
}

static FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    compile_regex(r"(?i)\b(?:fn|def|function|func|pub\s+fn|async\s+fn)\s+([a-zA-Z_][a-zA-Z0-9_]*)")
});

static IMPORT_RE: LazyLock<Regex> =
    LazyLock::new(|| compile_regex(r"(?i)\b(?:use|from|import)\s+([a-zA-Z_][a-zA-Z0-9_:./]*)"));

static IDENTIFIER_RE: LazyLock<Regex> =
    LazyLock::new(|| compile_regex(r"\b([A-Z][a-zA-Z0-9]+|[a-z]+(?:_[a-z]+)+)\b"));

impl Storage {
    /// Ensure the `entities_fts` table exists (called on connection open).
    ///
    /// # Errors
    ///
    /// Returns an error if the FTS5 table cannot be created.
    pub fn ensure_entities_fts(&self) -> Result<()> {
        let conn = self.conn();
        let conn = conn.lock();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS entities_fts USING fts5(\
                entity, \
                tokenize='porter unicode61'\
            );",
        )
        .context("failed to create entities_fts table")?;
        Ok(())
    }

    /// Extract entities from chunk content using deterministic regex rules.
    #[must_use]
    pub fn extract_entities(content: &str, file_path: &str) -> Vec<String> {
        let mut entities = Vec::new();

        // 1. Function/method names
        for mat in FUNCTION_RE.captures_iter(content) {
            if let Some(m) = mat.get(1) {
                entities.push(m.as_str().to_lowercase());
            }
        }

        // 2. Import paths (extract last segment of path)
        for mat in IMPORT_RE.captures_iter(content) {
            if let Some(m) = mat.get(1) {
                let path = m.as_str();
                // Extract last segment: "crate::auth::validate_token" → "validate_token"
                if let Some(last) = path.split([':', '/', '.']).next_back() {
                    if !last.is_empty() {
                        entities.push(last.to_lowercase());
                    }
                }
            }
        }

        // 3. Identifiers (PascalCase types, snake_case names)
        for mat in IDENTIFIER_RE.captures_iter(content) {
            if let Some(m) = mat.get(1) {
                let val = m.as_str().to_lowercase();
                if val.len() >= 3 {
                    entities.push(val);
                }
            }
        }

        // 4. File stem as entity
        if let Some(stem) = std::path::Path::new(file_path).file_stem() {
            entities.push(stem.to_string_lossy().to_lowercase());
        }

        // Dedup + truncate
        entities.sort();
        entities.dedup();
        entities.truncate(MAX_ENTITIES_PER_CHUNK);
        entities
    }

    /// Insert entities for a chunk into `chunk_entities` and `entities_fts`.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub fn insert_chunk_entities(&self, chunk_id: i64, entities: &[String]) -> Result<()> {
        let conn = self.conn();
        let conn = conn.lock();

        for entity in entities {
            conn.execute(
                "INSERT OR IGNORE INTO chunk_entities (chunk_id, entity) VALUES (?1, ?2)",
                params![chunk_id, entity],
            )
            .context("failed to insert chunk entity")?;

            conn.execute(
                "INSERT INTO entities_fts (entity) VALUES (?1)",
                params![entity],
            )
            .context("failed to insert into entities_fts")?;
        }

        Ok(())
    }

    /// Get entities for a chunk.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_chunk_entities(&self, chunk_id: i64) -> Result<Option<Vec<String>>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare("SELECT entity FROM chunk_entities WHERE chunk_id = ?1")
            .context("failed to prepare get_chunk_entities query")?;

        let entities: Vec<String> = stmt
            .query_map(params![chunk_id], |row| row.get(0))
            .context("failed to query chunk entities")?
            .filter_map(std::result::Result::ok)
            .collect();

        if entities.is_empty() {
            Ok(None)
        } else {
            Ok(Some(entities))
        }
    }

    /// Search entities via FTS5, returning matching chunk IDs with BM25 scores.
    ///
    /// # Errors
    ///
    /// Returns an error if the FTS5 query fails.
    pub fn search_entities(
        &self,
        query_entities: &[String],
        buffer_id: i64,
        top_k: usize,
    ) -> Result<Vec<EntityHit>> {
        let conn = self.conn();
        let conn = conn.lock();
        let limit = i64::try_from(top_k).context("top_k overflow")?;

        let mut results: Vec<EntityHit> = Vec::new();

        for entity in query_entities {
            // FTS5 prefix match: "entity*" matches "entity_name"
            let fts_query = format!("{entity}*");
            let mut stmt = conn
                .prepare(
                    "SELECT ce.chunk_id, bm25(entities_fts) as score \
                     FROM entities_fts \
                     JOIN chunk_entities ce ON ce.entity = entities_fts.entity \
                     JOIN chunks c ON c.id = ce.chunk_id \
                     WHERE entities_fts.entity MATCH ?1 \
                       AND c.buffer_id = ?2 \
                     ORDER BY score \
                     LIMIT ?3",
                )
                .context("failed to prepare entity search query")?;

            let rows = stmt
                .query_map(params![fts_query, buffer_id, limit], |row| {
                    Ok(EntityHit {
                        chunk_id: row.get(0)?,
                        score: row.get(1)?,
                    })
                })?
                .filter_map(std::result::Result::ok);

            results.extend(rows);
        }

        Ok(results)
    }

    /// Search entities across all buffers.
    ///
    /// # Errors
    ///
    /// Returns an error if the FTS5 query fails.
    pub fn search_entities_all(
        &self,
        query_entities: &[String],
        top_k: usize,
    ) -> Result<Vec<EntityHit>> {
        let conn = self.conn();
        let conn = conn.lock();
        let limit = i64::try_from(top_k).context("top_k overflow")?;

        let mut results: Vec<EntityHit> = Vec::new();

        for entity in query_entities {
            let fts_query = format!("{entity}*");
            let mut stmt = conn
                .prepare(
                    "SELECT ce.chunk_id, bm25(entities_fts) as score \
                     FROM entities_fts \
                     JOIN chunk_entities ce ON ce.entity = entities_fts.entity \
                     WHERE entities_fts.entity MATCH ?1 \
                     ORDER BY score \
                     LIMIT ?2",
                )
                .context("failed to prepare entity search_all query")?;

            let rows = stmt
                .query_map(params![fts_query, limit], |row| {
                    Ok(EntityHit {
                        chunk_id: row.get(0)?,
                        score: row.get(1)?,
                    })
                })?
                .filter_map(std::result::Result::ok);

            results.extend(rows);
        }

        Ok(results)
    }
}

/// A single entity search hit.
#[derive(Debug, Clone)]
pub struct EntityHit {
    pub chunk_id: i64,
    pub score: f64,
}
