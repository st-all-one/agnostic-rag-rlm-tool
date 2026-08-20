//! High-level persist operations for each wiki scope.

use anyhow::{Context, Result};
use chrono::Utc;

use crate::persist::{
    AnalysisPersistOptions, DecisionPersistOptions, Frontmatter, PersistEngine, PersistResult,
    SearchPersistOptions, SessionPersistOptions, TrajectoryPersistOptions, WikiScope,
    default_salience,
};
use crate::persist::render_markdown;
use crate::ScopedTimer;

impl PersistEngine {
    /// Persist a search result.
    ///
    /// # Errors
    ///
    /// Returns an error if file creation or writing fails.
    pub fn persist_search(&self, options: &SearchPersistOptions) -> Result<PersistResult> {
        let _timer = ScopedTimer::new("persist_search");

        let now = Utc::now();
        let date_prefix = now.format("%Y-%m-%d").to_string();
        let slug = crate::persist::sanitize_slug(&options.query);
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
        let slug = crate::persist::sanitize_slug(&options.title);
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
        let slug = crate::persist::sanitize_slug(&options.title);
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
        let filename = format!("{}.md", crate::persist::sanitize_identifier(&options.session_id));

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
        let filename = format!("{}.md", crate::persist::sanitize_identifier(&options.run_id));

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
}
