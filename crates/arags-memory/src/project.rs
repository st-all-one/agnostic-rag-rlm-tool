use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use arags_storage::Storage;

use crate::ScopedTimer;

/// Project metadata and state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub id: i64,
    pub name: String,
    pub path: PathBuf,
    pub created_at: DateTime<Utc>,
    pub last_indexed_at: Option<DateTime<Utc>>,
    pub total_chunks: i64,
    pub total_files: i64,
}

/// Options for creating a project.
#[derive(Debug, Clone)]
pub struct CreateProjectOptions {
    pub name: String,
    pub path: PathBuf,
}

/// Manages project lifecycle (create, list, get, forget).
pub struct ProjectManager {
    storage: Storage,
}

impl ProjectManager {
    /// Create a new `ProjectManager` backed by the given storage.
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    /// Register a new project.
    ///
    /// # Errors
    ///
    /// Returns an error if the project name already exists or storage fails.
    pub fn create(&self, options: &CreateProjectOptions) -> Result<ProjectInfo> {
        let _timer = ScopedTimer::new("project_create");

        let existing = self
            .storage
            .get_buffer_by_name(&options.name)
            .context("failed to check existing project")?;

        if existing.is_some() {
            anyhow::bail!("project '{}' already exists", options.name);
        }

        let path_str = options
            .path
            .to_str()
            .context("project path is not valid UTF-8")?;

        let id = self
            .storage
            .insert_buffer(&arags_storage::sqlite::buffers::NewBuffer {
                name: options.name.clone(),
                path: path_str.to_string(),
            })
            .context("failed to insert project")?;

        let buffer = self
            .storage
            .get_buffer(id)
            .context("failed to get created project")?
            .context("project not found after creation")?;

        tracing::info!(project_id = id, name = %options.name, "project created");

        Ok(ProjectInfo::from_buffer(&buffer))
    }

    /// List all registered projects.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage query fails.
    pub fn list(&self) -> Result<Vec<ProjectInfo>> {
        let _timer = ScopedTimer::new("project_list");

        let buffers = self
            .storage
            .list_buffers()
            .context("failed to list projects")?;

        Ok(buffers.iter().map(ProjectInfo::from_buffer).collect())
    }

    /// Get a project by name.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage query fails.
    pub fn get(&self, name: &str) -> Result<Option<ProjectInfo>> {
        let buffer = self
            .storage
            .get_buffer_by_name(name)
            .context("failed to get project")?;

        Ok(buffer.map(|b| ProjectInfo::from_buffer(&b)))
    }

    /// Get a project by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage query fails.
    pub fn get_by_id(&self, id: i64) -> Result<Option<ProjectInfo>> {
        let buffer = self
            .storage
            .get_buffer(id)
            .context("failed to get project by id")?;

        Ok(buffer.map(|b| ProjectInfo::from_buffer(&b)))
    }

    /// Remove a project and all its associated data.
    ///
    /// # Errors
    ///
    /// Returns an error if the project doesn't exist or deletion fails.
    pub fn forget(&self, name: &str) -> Result<()> {
        let _timer = ScopedTimer::new("project_forget");

        let buffer = self
            .storage
            .get_buffer_by_name(name)
            .context("failed to find project")?
            .context("project not found")?;

        self.storage
            .delete_buffer(buffer.id)
            .context("failed to delete project")?;

        tracing::info!(project_id = buffer.id, name = name, "project forgotten");

        Ok(())
    }
}

impl ProjectInfo {
    fn from_buffer(buffer: &arags_storage::sqlite::buffers::Buffer) -> Self {
        Self {
            id: buffer.id,
            name: buffer.name.clone(),
            path: PathBuf::from(&buffer.path),
            created_at: DateTime::from_timestamp(buffer.created_at, 0).unwrap_or_default(),
            last_indexed_at: buffer
                .last_indexed_at
                .and_then(|ts| DateTime::from_timestamp(ts, 0)),
            total_chunks: buffer.total_chunks,
            total_files: buffer.total_files,
        }
    }
}
