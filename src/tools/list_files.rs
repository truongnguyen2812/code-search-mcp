use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::db::DbPool;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListFilesInput {
    /// Path substring or glob to filter by (e.g. "frameworks/base", "*.java")
    pub path_filter: Option<String>,
    /// Filter by language: java, kotlin, c, cpp
    pub language: Option<String>,
    /// Maximum results (default: 200)
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct FileEntry {
    pub path: String,
    pub language: String,
}

pub struct ListFilesTool {
    pub pool: DbPool,
}

impl ListFilesTool {
    pub fn list(&self, input: ListFilesInput) -> Result<Vec<FileEntry>> {
        let conn = self.pool.get()?;
        let limit = input.limit.unwrap_or(200).min(2000);

        let path_pattern = input.path_filter.as_deref().map(|p| {
            // If it looks like a glob, convert * → %
            if p.contains('*') {
                p.replace('*', "%")
            } else {
                format!("%{p}%")
            }
        });

        let mut stmt = conn.prepare(
            "SELECT path, language FROM files
             WHERE (?1 IS NULL OR path LIKE ?1)
               AND (?2 IS NULL OR language = ?2)
             ORDER BY path
             LIMIT ?3",
        )?;

        let rows = stmt.query_map(
            rusqlite::params![path_pattern, input.language, limit],
            |row| {
                Ok(FileEntry {
                    path: row.get(0)?,
                    language: row.get(1)?,
                })
            },
        )?;

        Ok(rows.flatten().collect())
    }
}
