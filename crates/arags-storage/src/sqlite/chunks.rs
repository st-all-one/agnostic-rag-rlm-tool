//! Chunk metadata + content storage. The [`Storage`] impl methods are split by
//! concern across the `basic` / `time_travel` / `access` / `content` / `vector`
//! sibling modules; the shared [`Chunk`] type, column projection, and row mapper
//! live here.

pub(crate) mod access;
pub(crate) mod basic;
pub(crate) mod content;
pub(crate) mod time_travel;
pub(crate) mod vector;

use rusqlite::Row;

/// Metadata for a chunk (without content).
#[derive(Debug, Clone)]
pub struct Chunk {
    pub id: i64,
    pub buffer_id: i64,
    pub file_path: String,
    pub offset_start: i64,
    pub offset_end: i64,
    pub line_start: i64,
    pub line_end: i64,
    pub hash: Vec<u8>,
    pub language: Option<String>,
    pub chunk_type: Option<String>,
    pub token_count: Option<i64>,
    pub status: String,
    pub created_at: i64,
    pub last_accessed_at: i64,
    /// Project epoch at write time (drift / time-travel, plan 021).
    pub epoch: i64,
    /// Agent username that produced the chunk (audit/provenance).
    pub created_by: Option<String>,
    /// LLM model that produced the chunk (metadata).
    pub model: Option<String>,
    /// Revision counter; starts at 1, bumped on supersede (plan 021).
    pub version: i64,
    /// Whether this is the live revision (plan 021).
    pub is_active: bool,
    /// Rowid of the newer revision that superseded this one (`is_active = 0`
    /// rows only); `None` for the live row (plan 021).
    pub superseded_by: Option<i64>,
}

/// Column projection shared by all chunk row queries (order fixed; see
/// [`chunk_mapper`]). The trailing temporal columns let callers surface
/// authorship and walk the supersede chain for time-travel (plan 021).
pub(crate) const CHUNK_COLS: &str = "id, buffer_id, file_path, offset_start, offset_end, line_start, \
     line_end, hash, language, chunk_type, token_count, status, created_at, \
     last_accessed_at, epoch, created_by, model, version, is_active, superseded_by";

/// Map a chunk row into [`Chunk`] (column order matches [`CHUNK_COLS`]).
pub(crate) fn chunk_mapper(r: &Row<'_>) -> rusqlite::Result<Chunk> {
    Ok(Chunk {
        id: r.get(0)?,
        buffer_id: r.get(1)?,
        file_path: r.get(2)?,
        offset_start: r.get(3)?,
        offset_end: r.get(4)?,
        line_start: r.get(5)?,
        line_end: r.get(6)?,
        hash: r.get(7)?,
        language: r.get(8)?,
        chunk_type: r.get(9)?,
        token_count: r.get(10)?,
        status: r.get(11)?,
        created_at: r.get(12)?,
        last_accessed_at: r.get(13)?,
        epoch: r.get(14)?,
        created_by: r.get(15)?,
        model: r.get(16)?,
        version: r.get(17)?,
        is_active: r.get::<_, i64>(18)? != 0,
        superseded_by: r.get(19)?,
    })
}

/// New chunk to insert.
#[derive(Debug)]
pub struct NewChunk {
    pub buffer_id: i64,
    pub file_path: String,
    pub offset_start: i64,
    pub offset_end: i64,
    pub line_start: i64,
    pub line_end: i64,
    pub hash: Vec<u8>,
    pub language: Option<String>,
    pub chunk_type: Option<String>,
    pub token_count: Option<i64>,
}
