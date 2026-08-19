use anyhow::{Context, Result};
use rusqlite::params;

use super::conn::Storage;

/// Pattern extracted from analysis.
#[derive(Debug, Clone)]
pub struct Pattern {
    pub id: i64,
    pub buffer_id: Option<i64>,
    pub pattern_type: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub examples: Option<String>,
    pub confidence: Option<f64>,
    pub created_at: i64,
}

impl Storage {
    /// Insert a new pattern.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub fn insert_pattern(
        &self,
        buffer_id: Option<i64>,
        pattern_type: Option<&str>,
        name: &str,
        description: Option<&str>,
        examples: Option<&str>,
        confidence: Option<f64>,
    ) -> Result<i64> {
        let conn = self.conn();
        let conn = conn.lock();

        let id = conn
            .execute(
                "INSERT INTO patterns (buffer_id, pattern_type, name, description, examples, confidence) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![buffer_id, pattern_type, name, description, examples, confidence],
            )
            .context("failed to insert pattern")?;

        let pattern_id = i64::try_from(id).context("pattern id overflow")?;
        tracing::info!(pattern_id, name, pattern_type, "inserted pattern");

        Ok(pattern_id)
    }

    /// Get patterns for a buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_patterns(&self, buffer_id: Option<i64>) -> Result<Vec<Pattern>> {
        let conn = self.conn();
        let conn = conn.lock();

        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(bid) =
            buffer_id
        {
            (
                "SELECT id, buffer_id, pattern_type, name, description, examples, confidence, created_at FROM patterns WHERE buffer_id = ?1 ORDER BY name".to_string(),
                vec![Box::new(bid)],
            )
        } else {
            (
                "SELECT id, buffer_id, pattern_type, name, description, examples, confidence, created_at FROM patterns ORDER BY name".to_string(),
                vec![],
            )
        };

        let mut stmt = conn
            .prepare(&sql)
            .context("failed to prepare get_patterns query")?;

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(AsRef::as_ref).collect();

        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(Pattern {
                    id: row.get(0)?,
                    buffer_id: row.get(1)?,
                    pattern_type: row.get(2)?,
                    name: row.get(3)?,
                    description: row.get(4)?,
                    examples: row.get(5)?,
                    confidence: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_storage() -> (Storage, TempDir) {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::open(tmp.path()).unwrap();
        (storage, tmp)
    }

    #[test]
    fn test_insert_pattern() {
        let (storage, _tmp) = setup_storage();

        let id = storage
            .insert_pattern(
                None,
                Some("architectural"),
                "use of builder pattern",
                Some("Complex objects use builder pattern"),
                None,
                Some(0.85),
            )
            .unwrap();
        assert!(id > 0);

        let patterns = storage.get_patterns(None).unwrap();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].name, "use of builder pattern");
    }
}
