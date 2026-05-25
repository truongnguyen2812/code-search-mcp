use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::db::DbPool;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchSymbolsInput {
    /// Symbol name or glob pattern (e.g. "Activity", "on*", "Binder*")
    pub query: String,
    /// Filter by language: java, kotlin, c, cpp (optional)
    pub language: Option<String>,
    /// Filter by kind: class, method, function, field, property, etc. (optional)
    pub kind: Option<String>,
    /// Maximum results to return (default: 50)
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SymbolResult {
    pub name: String,
    pub kind: String,
    pub container: String,
    pub file: String,
    pub language: String,
    pub start_line: i64,
    pub start_col: i64,
    pub signature: String,
}

pub struct SearchSymbolsTool {
    pub pool: DbPool,
}

impl SearchSymbolsTool {
    pub fn search(&self, input: SearchSymbolsInput) -> Result<Vec<SymbolResult>> {
        let conn = self.pool.get()?;
        let query = strip_glob_wildcards(&input.query);
        if query.is_empty() {
            return Ok(Vec::new());
        }

        let language = normalize_optional_filter(input.language).map(|v| v.to_lowercase());
        let kind = normalize_optional_filter(input.kind).map(|v| v.to_lowercase());
        let limit = sanitize_limit(input.limit);

        let results = query_by_fts(&conn, &query, &language, &kind, limit)?;

        Ok(results)
    }
}

fn sanitize_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(50).clamp(1, 500)
}

fn normalize_optional_filter(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn strip_glob_wildcards(query: &str) -> String {
    query.chars().filter(|c| *c != '*' && *c != '?').collect::<String>().trim().to_string()
}

fn query_by_fts(
    conn: &rusqlite::Connection,
    query: &str,
    language: &Option<String>,
    kind: &Option<String>,
    limit: i64,
) -> Result<Vec<SymbolResult>> {
    let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
    let mut stmt = conn.prepare(
        "SELECT s.name, s.kind, s.container, f.path, f.language, s.start_line, s.start_col, s.signature
         FROM symbols_fts fts
         JOIN symbols s ON fts.rowid = s.id
         JOIN files f ON s.file_id = f.id
         WHERE symbols_fts MATCH ?1
           AND (?2 IS NULL OR f.language = ?2)
           AND (?3 IS NULL OR s.kind = ?3)
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![fts_query, language, kind, limit],
        row_to_symbol,
    )?;
    Ok(rows.flatten().collect())
}

fn row_to_symbol(row: &rusqlite::Row) -> rusqlite::Result<SymbolResult> {
    Ok(SymbolResult {
        name: row.get(0)?,
        kind: row.get(1)?,
        container: row.get(2)?,
        file: row.get(3)?,
        language: row.get(4)?,
        start_line: row.get(5)?,
        start_col: row.get(6)?,
        signature: row.get(7)?,
    })
}
