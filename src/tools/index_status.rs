use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::db::DbPool;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IndexStatusInput {}

#[derive(Debug, Serialize)]
pub struct IndexStatus {
    pub status: String,
    pub total_files: i64,
    pub stored_files: i64,
    pub indexed_files: i64,
    pub total_symbols: i64,
    pub fts_status: String,
    pub files_by_language: HashMap<String, i64>,
    pub symbols_by_kind: HashMap<String, i64>,
    pub errors: i64,
}

pub struct IndexStatusTool {
    pub pool: DbPool,
}

impl IndexStatusTool {
    pub fn get_status(&self) -> Result<IndexStatus> {
        let conn = self.pool.get()?;

        let status: String = conn
            .query_row(
                "SELECT value FROM index_progress WHERE key = 'status'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "idle".to_string());

        let total_str: String = conn
            .query_row(
                "SELECT value FROM index_progress WHERE key = 'total_files'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "0".to_string());
        let total_files: i64 = total_str.parse().unwrap_or(0);

        let indexed_str: String = conn
            .query_row(
                "SELECT value FROM index_progress WHERE key = 'indexed_files'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "0".to_string());
        let indexed_files: i64 = indexed_str.parse().unwrap_or(0);

        let fts_status: String = conn
            .query_row(
                "SELECT value FROM index_progress WHERE key = 'fts_status'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "ready".to_string());

        let stored_files: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .unwrap_or(0);

        let total_symbols: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))
            .unwrap_or(0);

        // Count by language
        let mut lang_stmt = conn.prepare(
            "SELECT language, COUNT(*) FROM files GROUP BY language",
        )?;
        let files_by_language: HashMap<String, i64> = lang_stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?
            .flatten()
            .collect();

        // Count symbols by kind
        let mut kind_stmt = conn.prepare(
            "SELECT kind, COUNT(*) FROM symbols GROUP BY kind ORDER BY COUNT(*) DESC",
        )?;
        let symbols_by_kind: HashMap<String, i64> = kind_stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?
            .flatten()
            .collect();

        Ok(IndexStatus {
            status,
            total_files,
            stored_files,
            indexed_files,
            total_symbols,
            fts_status,
            files_by_language,
            symbols_by_kind,
            errors: 0,
        })
    }
}
