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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_tier_display() {
        assert_eq!(SearchTier::Fts.to_string(), "fts");
        assert_eq!(SearchTier::Entity.to_string(), "entity");
        assert_eq!(SearchTier::Vector.to_string(), "vector");
        assert_eq!(SearchTier::LlmRerank.to_string(), "llm_rerank");
    }

    #[test]
    fn test_search_options_default() {
        let opts = SearchOptions::default();
        assert_eq!(opts.tier, SearchTier::Entity);
        assert_eq!(opts.top_k, 10);
    }

    #[test]
    fn test_hybrid_result_clone() {
        let r = HybridResult {
            chunk_id: 1,
            score: 0.5,
        };
        let r2 = r.clone();
        assert_eq!(r.chunk_id, r2.chunk_id);
        assert_eq!(r.score, r2.score);
    }

    #[test]
    fn test_output_format_variants() {
        let _ = OutputFormat::Prompt;
        let _ = OutputFormat::Json;
        let _ = OutputFormat::Markdown;
    }
}
