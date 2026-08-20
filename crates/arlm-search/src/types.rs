use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchTier {
    Fts,
    Entity,
    Vector,
    LlmRerank,
}

impl fmt::Display for SearchTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fts => write!(f, "fts"),
            Self::Entity => write!(f, "entity"),
            Self::Vector => write!(f, "vector"),
            Self::LlmRerank => write!(f, "llm_rerank"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Bm25Result {
    pub chunk_id: i64,
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct SemanticResult {
    pub chunk_id: u64,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct EntityResult {
    pub chunk_id: i64,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct HybridResult {
    pub chunk_id: i64,
    pub score: f32,
    /// `true` when `chunk_id` refers to a row in the `summaries` table
    /// (dual-layer search) rather than the `chunks` table.
    pub is_summary: bool,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk_id: i64,
    pub score: f32,
    pub file_path: String,
    pub line_start: i64,
    pub line_end: i64,
    pub content: String,
    pub language: Option<String>,
    /// `true` when this result comes from the `summaries` table.
    pub is_summary: bool,
    /// Scope of the source summary (`file`/`module`/`project`) when `is_summary`.
    pub summary_scope: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub tier: SearchTier,
    pub top_k: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            tier: SearchTier::Entity,
            top_k: 10,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChunkWithText {
    pub id: i64,
    pub buffer_id: i64,
    pub file_path: String,
    pub line_start: i64,
    pub line_end: i64,
    pub content: String,
    pub language: Option<String>,
    /// `true` when this record was resolved from the `summaries` table.
    pub is_summary: bool,
    /// Scope of the source summary when `is_summary`.
    pub summary_scope: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Prompt,
    Json,
    Markdown,
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prompt => write!(f, "prompt"),
            Self::Json => write!(f, "json"),
            Self::Markdown => write!(f, "markdown"),
        }
    }
}
