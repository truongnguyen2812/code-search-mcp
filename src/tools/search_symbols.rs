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
        let query = input.query.trim().to_string();
        if query.is_empty() {
            return Ok(Vec::new());
        }

        let language = normalize_optional_filter(input.language).map(|v| v.to_lowercase());
        let kind = normalize_optional_filter(input.kind).map(|v| v.to_lowercase());
        let limit = sanitize_limit(input.limit);

        // If the query looks like a glob pattern, use LIKE; otherwise try FTS first
        let results = if query.contains('*') || query.contains('?') {
            let pattern = glob_to_sql_like_pattern(&query);
            query_by_like(&conn, &pattern, &language, &kind, limit)?
        } else {
            // Try FTS first for fast prefix/exact match
            match query_by_fts(&conn, &query, &language, &kind, limit) {
                Ok(fts_results) if !fts_results.is_empty() => fts_results,
                Ok(_) | Err(_) => {
                    let fallback_pattern = format!("%{}%", escape_like(&query));
                    query_by_like(&conn, &fallback_pattern, &language, &kind, limit)?
                }
            }
        };

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

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn glob_to_sql_like_pattern(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 4);
    for ch in query.chars() {
        match ch {
            '*' => out.push('%'),
            '?' => out.push('_'),
            '%' => out.push_str("\\%"),
            '_' => out.push_str("\\_"),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out
}

fn query_by_fts(
    conn: &rusqlite::Connection,
    query: &str,
    language: &Option<String>,
    kind: &Option<String>,
    limit: i64,
) -> Result<Vec<SymbolResult>> {
    let fts_query = format!("{query}*");
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

fn query_by_like(
    conn: &rusqlite::Connection,
    pattern: &str,
    language: &Option<String>,
    kind: &Option<String>,
    limit: i64,
) -> Result<Vec<SymbolResult>> {
    let mut stmt = conn.prepare(
        "SELECT s.name, s.kind, s.container, f.path, f.language, s.start_line, s.start_col, s.signature
         FROM symbols s
         JOIN files f ON s.file_id = f.id
                 WHERE s.name LIKE ?1 ESCAPE '\\' COLLATE NOCASE
           AND (?2 IS NULL OR f.language = ?2)
           AND (?3 IS NULL OR s.kind = ?3)
         ORDER BY length(s.name), s.name
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![pattern, language, kind, limit],
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
