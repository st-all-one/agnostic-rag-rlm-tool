//! Types for wiki page persistence.

use serde::{Deserialize, Serialize};

/// YAML frontmatter for persisted wiki pages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter {
    /// Human-readable title.
    pub title: String,
    /// ISO 8601 creation timestamp.
    pub created: String,
    /// ISO 8601 last-updated timestamp.
    pub updated: String,
    /// Original search query, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Search tier used (fts, entity, vector, llm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    /// Project name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Extracted entities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<String>,
    /// User-supplied tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// If true, survives decay eviction.
    #[serde(default)]
    pub pinned: bool,
    /// Optional TTL (ISO 8601).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Retention score (0.0–1.0).
    #[serde(default = "default_salience")]
    pub salience: f64,
    /// Times this page has been accessed.
    #[serde(default)]
    pub access_count: u64,
    /// Path of the previous version, if superseded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
}

/// Default salience value for newly created wiki pages.
#[must_use]
pub fn default_salience() -> f64 {
    1.0
}

/// The category of wiki page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WikiScope {
    /// Searches: .arlm/wiki/searches/
    Searches,
    /// Analyses: .arlm/wiki/analyses/
    Analyses,
    /// Decisions: .arlm/wiki/decisions/
    Decisions,
    /// Sessions: .arlm/wiki/sessions/
    Sessions,
    /// Trajectories: .arlm/wiki/trajectories/
    Trajectories,
    /// Global rules: .arlm/wiki/_global/
    Global,
}

impl WikiScope {
    /// Directory name inside the wiki.
    #[must_use]
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Searches => "searches",
            Self::Analyses => "analyses",
            Self::Decisions => "decisions",
            Self::Sessions => "sessions",
            Self::Trajectories => "trajectories",
            Self::Global => "_global",
        }
    }
}

/// Options for persisting a search result.
#[derive(Debug, Clone)]
pub struct SearchPersistOptions {
    /// The original query.
    pub query: String,
    /// Search tier used.
    pub tier: String,
    /// Project name.
    pub project: String,
    /// Extracted entities.
    pub entities: Vec<String>,
    /// Tags.
    pub tags: Vec<String>,
    /// Formatted markdown body.
    pub body: String,
}

/// Options for persisting an analysis.
#[derive(Debug, Clone)]
pub struct AnalysisPersistOptions {
    /// Title for the analysis page.
    pub title: String,
    /// Project name.
    pub project: String,
    /// Tags.
    pub tags: Vec<String>,
    /// Markdown body content.
    pub body: String,
}

/// Options for persisting a decision.
#[derive(Debug, Clone)]
pub struct DecisionPersistOptions {
    /// Title for the decision page.
    pub title: String,
    /// Tags.
    pub tags: Vec<String>,
    /// Markdown body content.
    pub body: String,
    /// Path of the previous version, if superseding.
    pub supersedes: Option<String>,
}

/// Options for persisting a session.
#[derive(Debug, Clone)]
pub struct SessionPersistOptions {
    /// Session ID.
    pub session_id: String,
    /// Project name.
    pub project: String,
    /// Markdown body content.
    pub body: String,
}

/// Options for persisting a trajectory.
#[derive(Debug, Clone)]
pub struct TrajectoryPersistOptions {
    /// Run ID.
    pub run_id: String,
    /// Project name.
    pub project: String,
    /// Markdown body content.
    pub body: String,
}

/// Result of a persist operation.
#[derive(Debug, Clone)]
pub struct PersistResult {
    /// Absolute path to the created file.
    pub path: std::path::PathBuf,
    /// Relative path inside the wiki.
    pub relative_path: String,
}
