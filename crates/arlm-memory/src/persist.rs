use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::ScopedTimer;

/// The wiki directory name inside a project.
const WIKI_DIR: &str = ".arlm/wiki";

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

fn default_salience() -> f64 {
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
    fn dir_name(self) -> &'static str {
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
    pub path: PathBuf,
    /// Relative path inside the wiki.
    pub relative_path: String,
}

/// Engine for persisting wiki markdown pages.
pub struct PersistEngine {
    wiki_root: PathBuf,
}

impl PersistEngine {
    /// Create a new `PersistEngine` rooted at `<project>/.arlm/wiki/`.
    ///
    /// # Errors
    ///
    /// Returns an error if the wiki directory cannot be created.
    pub fn new(project_path: &Path) -> Result<Self> {
        let wiki_root = project_path.join(WIKI_DIR);
        Self::ensure_dirs(&wiki_root)?;
        Ok(Self { wiki_root })
    }

    /// Create a `PersistEngine` with an explicit wiki root (for testing).
    #[must_use]
    pub fn with_wiki_root(wiki_root: PathBuf) -> Self {
        Self { wiki_root }
    }

    /// Get the wiki root directory.
    #[must_use]
    pub fn wiki_root(&self) -> &Path {
        &self.wiki_root
    }

    /// Persist a search result.
    ///
    /// # Errors
    ///
    /// Returns an error if file creation or writing fails.
    pub fn persist_search(&self, options: &SearchPersistOptions) -> Result<PersistResult> {
        let _timer = ScopedTimer::new("persist_search");

        let now = Utc::now();
        let date_prefix = now.format("%Y-%m-%d").to_string();
        let slug = sanitize_slug(&options.query);
        let filename = format!("{date_prefix}_{slug}.md");

        let frontmatter = Frontmatter {
            title: format!("{} - search", options.query),
            created: now.to_rfc3339(),
            updated: now.to_rfc3339(),
            query: Some(options.query.clone()),
            tier: Some(options.tier.clone()),
            project: Some(options.project.clone()),
            entities: options.entities.clone(),
            tags: options.tags.clone(),
            pinned: false,
            expires_at: None,
            salience: default_salience(),
            access_count: 0,
            supersedes: None,
        };

        let content = render_markdown(&frontmatter, &options.body);
        let dir = self.wiki_root.join(WikiScope::Searches.dir_name());
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create directory: {}", dir.display()))?;

        let file_path = dir.join(&filename);
        std::fs::write(&file_path, &content)
            .with_context(|| format!("failed to write file: {}", file_path.display()))?;

        let relative_path = format!("{}/{}", WikiScope::Searches.dir_name(), filename);

        tracing::info!(
            path = %file_path.display(),
            query = %options.query,
            "search persisted"
        );

        Ok(PersistResult {
            path: file_path,
            relative_path,
        })
    }

    /// Persist an analysis page.
    ///
    /// # Errors
    ///
    /// Returns an error if file creation or writing fails.
    pub fn persist_analysis(&self, options: &AnalysisPersistOptions) -> Result<PersistResult> {
        let _timer = ScopedTimer::new("persist_analysis");

        let now = Utc::now();
        let seq = self.next_sequence(WikiScope::Analyses)?;
        let slug = sanitize_slug(&options.title);
        let filename = format!("{seq:03}-{slug}.md");

        let frontmatter = Frontmatter {
            title: options.title.clone(),
            created: now.to_rfc3339(),
            updated: now.to_rfc3339(),
            query: None,
            tier: None,
            project: Some(options.project.clone()),
            entities: Vec::new(),
            tags: options.tags.clone(),
            pinned: false,
            expires_at: None,
            salience: default_salience(),
            access_count: 0,
            supersedes: None,
        };

        let content = render_markdown(&frontmatter, &options.body);
        let dir = self.wiki_root.join(WikiScope::Analyses.dir_name());
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create directory: {}", dir.display()))?;

        let file_path = dir.join(&filename);
        std::fs::write(&file_path, &content)
            .with_context(|| format!("failed to write file: {}", file_path.display()))?;

        let relative_path = format!("{}/{}", WikiScope::Analyses.dir_name(), filename);

        tracing::info!(path = %file_path.display(), "analysis persisted");

        Ok(PersistResult {
            path: file_path,
            relative_path,
        })
    }

    /// Persist a decision page.
    ///
    /// # Errors
    ///
    /// Returns an error if file creation or writing fails.
    pub fn persist_decision(&self, options: &DecisionPersistOptions) -> Result<PersistResult> {
        let _timer = ScopedTimer::new("persist_decision");

        let now = Utc::now();
        let seq = self.next_sequence(WikiScope::Decisions)?;
        let slug = sanitize_slug(&options.title);
        let filename = format!("{seq:03}-{slug}.md");

        let frontmatter = Frontmatter {
            title: options.title.clone(),
            created: now.to_rfc3339(),
            updated: now.to_rfc3339(),
            query: None,
            tier: None,
            project: None,
            entities: Vec::new(),
            tags: options.tags.clone(),
            pinned: false,
            expires_at: None,
            salience: default_salience(),
            access_count: 0,
            supersedes: options.supersedes.clone(),
        };

        let content = render_markdown(&frontmatter, &options.body);
        let dir = self.wiki_root.join(WikiScope::Decisions.dir_name());
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create directory: {}", dir.display()))?;

        let file_path = dir.join(&filename);
        std::fs::write(&file_path, &content)
            .with_context(|| format!("failed to write file: {}", file_path.display()))?;

        let relative_path = format!("{}/{}", WikiScope::Decisions.dir_name(), filename);

        tracing::info!(path = %file_path.display(), "decision persisted");

        Ok(PersistResult {
            path: file_path,
            relative_path,
        })
    }

    /// Persist a session page.
    ///
    /// # Errors
    ///
    /// Returns an error if file creation or writing fails.
    pub fn persist_session(&self, options: &SessionPersistOptions) -> Result<PersistResult> {
        let _timer = ScopedTimer::new("persist_session");

        let now = Utc::now();
        let filename = format!("{}.md", sanitize_identifier(&options.session_id));

        let frontmatter = Frontmatter {
            title: format!("session {}", options.session_id),
            created: now.to_rfc3339(),
            updated: now.to_rfc3339(),
            query: None,
            tier: None,
            project: Some(options.project.clone()),
            entities: Vec::new(),
            tags: Vec::new(),
            pinned: false,
            expires_at: None,
            salience: default_salience(),
            access_count: 0,
            supersedes: None,
        };

        let content = render_markdown(&frontmatter, &options.body);
        let dir = self.wiki_root.join(WikiScope::Sessions.dir_name());
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create directory: {}", dir.display()))?;

        let file_path = dir.join(&filename);
        std::fs::write(&file_path, &content)
            .with_context(|| format!("failed to write file: {}", file_path.display()))?;

        let relative_path = format!("{}/{}", WikiScope::Sessions.dir_name(), filename);

        tracing::info!(path = %file_path.display(), session = %options.session_id, "session persisted");

        Ok(PersistResult {
            path: file_path,
            relative_path,
        })
    }

    /// Persist a trajectory page.
    ///
    /// # Errors
    ///
    /// Returns an error if file creation or writing fails.
    pub fn persist_trajectory(&self, options: &TrajectoryPersistOptions) -> Result<PersistResult> {
        let _timer = ScopedTimer::new("persist_trajectory");

        let now = Utc::now();
        let filename = format!("{}.md", sanitize_identifier(&options.run_id));

        let frontmatter = Frontmatter {
            title: format!("run {}", options.run_id),
            created: now.to_rfc3339(),
            updated: now.to_rfc3339(),
            query: None,
            tier: None,
            project: Some(options.project.clone()),
            entities: Vec::new(),
            tags: Vec::new(),
            pinned: false,
            expires_at: None,
            salience: default_salience(),
            access_count: 0,
            supersedes: None,
        };

        let content = render_markdown(&frontmatter, &options.body);
        let dir = self.wiki_root.join(WikiScope::Trajectories.dir_name());
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create directory: {}", dir.display()))?;

        let file_path = dir.join(&filename);
        std::fs::write(&file_path, &content)
            .with_context(|| format!("failed to write file: {}", file_path.display()))?;

        let relative_path = format!("{}/{}", WikiScope::Trajectories.dir_name(), filename);

        tracing::info!(path = %file_path.display(), run = %options.run_id, "trajectory persisted");

        Ok(PersistResult {
            path: file_path,
            relative_path,
        })
    }

    /// Persist a raw markdown body at an arbitrary wiki path.
    ///
    /// The `wiki_path` is relative to the wiki root (e.g. `rules/no-unwrap.md`).
    ///
    /// # Errors
    ///
    /// Returns an error if file creation or writing fails.
    pub fn persist_raw(&self, wiki_path: &str, body: &str) -> Result<PersistResult> {
        let _timer = ScopedTimer::new("persist_raw");

        let file_path = self.wiki_root.join(wiki_path);

        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory: {}", parent.display()))?;
        }

        std::fs::write(&file_path, body)
            .with_context(|| format!("failed to write file: {}", file_path.display()))?;

        tracing::info!(path = %file_path.display(), "raw page persisted");

        Ok(PersistResult {
            path: file_path,
            relative_path: wiki_path.to_string(),
        })
    }

    /// Read and parse a persisted wiki page.
    ///
    /// Returns the frontmatter and the body (everything after the frontmatter block).
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn read_page(&self, wiki_path: &str) -> Result<(Frontmatter, String)> {
        let file_path = self.wiki_root.join(wiki_path);
        let content = std::fs::read_to_string(&file_path)
            .with_context(|| format!("failed to read file: {}", file_path.display()))?;

        parse_markdown(&content)
    }

    /// List all pages under a scope.
    ///
    /// # Errors
    ///
    /// Returns an error if directory reading fails.
    pub fn list_pages(&self, scope: WikiScope) -> Result<Vec<String>> {
        let dir = self.wiki_root.join(scope.dir_name());
        if !dir.is_dir() {
            return Ok(Vec::new());
        }

        let mut pages = Vec::new();
        let entries = std::fs::read_dir(&dir)
            .with_context(|| format!("failed to read directory: {}", dir.display()))?;

        for entry in entries {
            let entry = entry.context("failed to read dir entry")?;
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    pages.push(format!("{}/{}", scope.dir_name(), name));
                }
            }
        }

        pages.sort();
        Ok(pages)
    }

    fn ensure_dirs(wiki_root: &Path) -> Result<()> {
        for scope in &[
            WikiScope::Searches,
            WikiScope::Analyses,
            WikiScope::Decisions,
            WikiScope::Sessions,
            WikiScope::Trajectories,
            WikiScope::Global,
        ] {
            let dir = wiki_root.join(scope.dir_name());
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("failed to create directory: {}", dir.display()))?;
        }
        Ok(())
    }

    fn next_sequence(&self, scope: WikiScope) -> Result<u32> {
        let dir = self.wiki_root.join(scope.dir_name());
        if !dir.is_dir() {
            return Ok(1);
        }

        let mut max_seq: u32 = 0;
        let entries = std::fs::read_dir(&dir)
            .with_context(|| format!("failed to read directory: {}", dir.display()))?;

        for entry in entries {
            let entry = entry.context("failed to read dir entry")?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some(seq_part) = name_str.split('-').next() {
                if let Ok(seq) = seq_part.parse::<u32>() {
                    max_seq = max_seq.max(seq);
                }
            }
        }

        Ok(max_seq + 1)
    }
}

/// Render a markdown page with YAML frontmatter.
fn render_markdown(frontmatter: &Frontmatter, body: &str) -> String {
    let yaml = serde_yaml_ng::to_string(frontmatter).unwrap_or_default();
    format!("---\n{yaml}---\n\n{body}")
}

/// Parse a markdown page with YAML frontmatter.
///
/// Returns `(frontmatter, body)`.
fn parse_markdown(content: &str) -> Result<(Frontmatter, String)> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let rest = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
        .context("missing opening frontmatter delimiter")?;

    let (yaml_part, body) = rest
        .split_once("\n---\n")
        .or_else(|| rest.split_once("\n---\r\n"))
        .or_else(|| {
            // Handle case where closing --- is at end of content
            rest.rsplit_once("\n---")
                .filter(|(_, after)| after.trim().is_empty())
                .map(|(before, _)| (before, ""))
        })
        .context("missing closing frontmatter delimiter")?;

    let frontmatter: Frontmatter =
        serde_yaml_ng::from_str(yaml_part).context("failed to parse frontmatter YAML")?;

    // Strip leading blank line after frontmatter
    let body = body.strip_prefix('\n').unwrap_or(body);

    Ok((frontmatter, body.to_string()))
}

/// Sanitize a string into a filesystem-safe slug.
///
/// Lowercases, replaces non-alphanumeric characters with hyphens,
/// collapses consecutive hyphens, and trims leading/trailing hyphens.
fn sanitize_slug(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut prev_hyphen = false;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_hyphen = false;
        } else if ((ch == '_' || ch == '-') || (ch.is_whitespace() || ch == '/' || ch == '\\'))
            && !prev_hyphen
            && !slug.is_empty()
        {
            slug.push('-');
            prev_hyphen = true;
        }
        // Skip other characters (punctuation, etc.)
    }

    slug.trim_matches('-').to_string()
}

/// Sanitize an identifier for use as a filename.
///
/// Unlike `sanitize_slug`, this preserves underscores and alphanumeric characters,
/// only replacing truly unsafe characters. Designed for IDs like `s_abc123`.
fn sanitize_identifier(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (PersistEngine, TempDir) {
        let tmp = TempDir::new().unwrap();
        let engine = PersistEngine::new(tmp.path()).unwrap();
        (engine, tmp)
    }

    #[test]
    fn test_new_creates_wiki_dirs() {
        let tmp = TempDir::new().unwrap();
        let engine = PersistEngine::new(tmp.path()).unwrap();

        assert!(engine.wiki_root().is_dir());
        assert!(engine.wiki_root().join("searches").is_dir());
        assert!(engine.wiki_root().join("analyses").is_dir());
        assert!(engine.wiki_root().join("decisions").is_dir());
        assert!(engine.wiki_root().join("sessions").is_dir());
        assert!(engine.wiki_root().join("trajectories").is_dir());
        assert!(engine.wiki_root().join("_global").is_dir());
    }

    #[test]
    fn test_persist_search() {
        let (engine, _tmp) = setup();

        let result = engine
            .persist_search(&SearchPersistOptions {
                query: "bug de login".to_string(),
                tier: "entity".to_string(),
                project: "my-project".to_string(),
                entities: vec!["jwt".to_string(), "session".to_string()],
                tags: vec!["auth".to_string()],
                body: "## Resultado\n\nSome content here.".to_string(),
            })
            .unwrap();

        assert!(result.path.exists());
        assert!(result.relative_path.starts_with("searches/"));

        let content = std::fs::read_to_string(&result.path).unwrap();
        assert!(content.starts_with("---\n"));
        assert!(content.contains("title:"));
        assert!(content.contains("query: bug de login"));
        assert!(content.contains("tier: entity"));
        assert!(content.contains("project: my-project"));
        assert!(content.contains("entities:"));
        assert!(content.contains("- jwt"));
        assert!(content.contains("- session"));
        assert!(content.contains("## Resultado"));
    }

    #[test]
    fn test_persist_analysis() {
        let (engine, _tmp) = setup();

        let result = engine
            .persist_analysis(&AnalysisPersistOptions {
                title: "Auth Architecture".to_string(),
                project: "proj".to_string(),
                tags: vec!["architecture".to_string()],
                body: "# Analysis\n\nDetailed analysis.".to_string(),
            })
            .unwrap();

        assert!(result.path.exists());
        assert!(result.relative_path.starts_with("analyses/"));
        assert!(result.relative_path.contains("001-auth-architecture"));

        let content = std::fs::read_to_string(&result.path).unwrap();
        assert!(content.contains("title: Auth Architecture"));
    }

    #[test]
    fn test_persist_analysis_auto_sequence() {
        let (engine, _tmp) = setup();

        let r1 = engine
            .persist_analysis(&AnalysisPersistOptions {
                title: "First".to_string(),
                project: "proj".to_string(),
                tags: Vec::new(),
                body: "body".to_string(),
            })
            .unwrap();

        let r2 = engine
            .persist_analysis(&AnalysisPersistOptions {
                title: "Second".to_string(),
                project: "proj".to_string(),
                tags: Vec::new(),
                body: "body".to_string(),
            })
            .unwrap();

        assert!(r1.relative_path.contains("001-"));
        assert!(r2.relative_path.contains("002-"));
    }

    #[test]
    fn test_persist_decision() {
        let (engine, _tmp) = setup();

        let result = engine
            .persist_decision(&DecisionPersistOptions {
                title: "Use Postgres".to_string(),
                tags: vec!["database".to_string()],
                body: "# Decision\n\nWe chose Postgres.".to_string(),
                supersedes: None,
            })
            .unwrap();

        assert!(result.path.exists());
        assert!(result.relative_path.starts_with("decisions/"));
        assert!(result.relative_path.contains("001-use-postgres"));

        let content = std::fs::read_to_string(&result.path).unwrap();
        assert!(content.contains("title: Use Postgres"));
    }

    #[test]
    fn test_persist_decision_with_supersedes() {
        let (engine, _tmp) = setup();

        let result = engine
            .persist_decision(&DecisionPersistOptions {
                title: "Use Postgres v2".to_string(),
                tags: Vec::new(),
                body: "Updated decision.".to_string(),
                supersedes: Some("decisions/001-use-postgres.md".to_string()),
            })
            .unwrap();

        let content = std::fs::read_to_string(&result.path).unwrap();
        assert!(content.contains("supersedes: decisions/001-use-postgres.md"));
    }

    #[test]
    fn test_persist_session() {
        let (engine, _tmp) = setup();

        let result = engine
            .persist_session(&SessionPersistOptions {
                session_id: "s_abc123".to_string(),
                project: "proj".to_string(),
                body: "# Session\n\nSession content.".to_string(),
            })
            .unwrap();

        assert!(result.path.exists());
        assert!(result.relative_path.contains("sessions/s_abc123.md"));
    }

    #[test]
    fn test_persist_trajectory() {
        let (engine, _tmp) = setup();

        let result = engine
            .persist_trajectory(&TrajectoryPersistOptions {
                run_id: "run_xyz".to_string(),
                project: "proj".to_string(),
                body: "# Run\n\nTrajectory content.".to_string(),
            })
            .unwrap();

        assert!(result.path.exists());
        assert!(result.relative_path.contains("trajectories/run_xyz.md"));
    }

    #[test]
    fn test_persist_raw() {
        let (engine, _tmp) = setup();

        let result = engine
            .persist_raw("rules/no-unwrap.md", "# Rule\n\nNo unwrap allowed.")
            .unwrap();

        assert!(result.path.exists());
        assert_eq!(result.relative_path, "rules/no-unwrap.md");

        let content = std::fs::read_to_string(&result.path).unwrap();
        assert_eq!(content, "# Rule\n\nNo unwrap allowed.");
    }

    #[test]
    fn test_read_page() {
        let (engine, _tmp) = setup();

        let content = "---\ntitle: Hello\ncreated: '2024-01-15T10:30:00Z'\nupdated: '2024-01-15T10:30:00Z'\nsalience: 1.0\n---\n\nWorld.";
        engine.persist_raw("test.md", content).unwrap();

        let (fm, body) = engine.read_page("test.md").unwrap();
        assert_eq!(fm.title, "Hello");
        assert_eq!(body, "World.");
    }

    #[test]
    fn test_list_pages_empty() {
        let (engine, _tmp) = setup();
        let pages = engine.list_pages(WikiScope::Analyses).unwrap();
        assert!(pages.is_empty());
    }

    #[test]
    fn test_list_pages() {
        let (engine, _tmp) = setup();

        engine
            .persist_analysis(&AnalysisPersistOptions {
                title: "A".to_string(),
                project: "proj".to_string(),
                tags: Vec::new(),
                body: "body".to_string(),
            })
            .unwrap();

        engine
            .persist_analysis(&AnalysisPersistOptions {
                title: "B".to_string(),
                project: "proj".to_string(),
                tags: Vec::new(),
                body: "body".to_string(),
            })
            .unwrap();

        let pages = engine.list_pages(WikiScope::Analyses).unwrap();
        assert_eq!(pages.len(), 2);
        assert!(pages[0].contains("001-a"));
        assert!(pages[1].contains("002-b"));
    }

    #[test]
    fn test_sanitize_slug() {
        assert_eq!(sanitize_slug("Hello World"), "hello-world");
        assert_eq!(sanitize_slug("bug de login"), "bug-de-login");
        assert_eq!(sanitize_slug("  spaces  "), "spaces");
        assert_eq!(sanitize_slug("special/chars\\here"), "special-chars-here");
        assert_eq!(sanitize_slug("already-ok"), "already-ok");
        assert_eq!(sanitize_slug("UPPER"), "upper");
        assert_eq!(sanitize_slug("a__b--c"), "a-b-c");
        assert_eq!(sanitize_slug(""), "");
    }

    #[test]
    fn test_frontmatter_roundtrip() {
        let fm = Frontmatter {
            title: "Test Page".to_string(),
            created: "2024-01-15T10:30:00Z".to_string(),
            updated: "2024-01-15T10:30:00Z".to_string(),
            query: Some("test query".to_string()),
            tier: Some("entity".to_string()),
            project: Some("proj".to_string()),
            entities: vec!["e1".to_string()],
            tags: vec!["t1".to_string()],
            pinned: true,
            expires_at: None,
            salience: 0.8,
            access_count: 5,
            supersedes: Some("old/path.md".to_string()),
        };

        let rendered = render_markdown(&fm, "body content");
        assert!(rendered.starts_with("---\n"));

        let (parsed_fm, body) = parse_markdown(&rendered).unwrap();
        assert_eq!(parsed_fm.title, "Test Page");
        assert_eq!(parsed_fm.query, Some("test query".to_string()));
        assert!(parsed_fm.pinned);
        assert_eq!(parsed_fm.salience, 0.8);
        assert_eq!(parsed_fm.access_count, 5);
        assert_eq!(parsed_fm.supersedes, Some("old/path.md".to_string()));
        assert_eq!(body, "body content");
    }

    #[test]
    fn test_pinned_survives_in_frontmatter() {
        let (engine, _tmp) = setup();

        engine
            .persist_decision(&DecisionPersistOptions {
                title: "Important".to_string(),
                tags: Vec::new(),
                body: "Body".to_string(),
                supersedes: None,
            })
            .unwrap();

        let pages = engine.list_pages(WikiScope::Decisions).unwrap();
        let (fm, _) = engine.read_page(&pages[0]).unwrap();
        assert!(!fm.pinned);
        assert_eq!(fm.salience, 1.0);
    }
}
