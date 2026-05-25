use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::db::DbPool;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchTextInput {
    /// Text or regex pattern to search for
    pub query: String,
    /// Treat query as a regex (default: false, plain substring)
    pub regex: Option<bool>,
    /// Filter by language: java, kotlin, c, cpp (optional)
    pub language: Option<String>,
    /// Filter to files matching this path substring (optional)
    pub path_filter: Option<String>,
    /// Maximum results (default: 100)
    pub limit: Option<i64>,
    /// Lines of context around each match (default: 2)
    pub context_lines: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct TextMatch {
    pub file: String,
    pub line_no: i64,
    pub content: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
}

pub struct SearchTextTool {
    pub pool: DbPool,
}

impl SearchTextTool {
    pub fn search(&self, input: SearchTextInput) -> Result<Vec<TextMatch>> {
        let conn = self.pool.get()?;
        let limit = input.limit.unwrap_or(100).min(1000);
        let ctx = input.context_lines.unwrap_or(2).min(10);
        let use_regex = input.regex.unwrap_or(false);

        // Strip glob wildcards — trigram FTS already does substring matching
        let query = strip_glob_wildcards(&input.query);
        if query.is_empty() {
            return Ok(Vec::new());
        }

        // Get matching (file_id, line_no, content) triples
        let matches: Vec<(i64, i64, String)> = if use_regex {
            search_regex(&conn, &query, &input.language, &input.path_filter, limit)?
        } else {
            search_by_fts_trigram(&conn, &query, &input.language, &input.path_filter, limit)?
        };

        if matches.is_empty() {
            return Ok(Vec::new());
        }

        // Batch fetch: collect all file_ids and the line ranges we need
        let mut file_ids: Vec<i64> = matches.iter().map(|(fid, _, _)| *fid).collect();
        file_ids.sort_unstable();
        file_ids.dedup();

        // Fetch file paths in one query
        let file_paths = batch_fetch_file_paths(&conn, &file_ids)?;

        // Batch fetch context lines for all matches at once
        let results = batch_fetch_context(&conn, &matches, &file_paths, ctx)?;

        Ok(results)
    }
}

/// Primary search path: FTS5 trigram index for fast substring matching.
/// The trigram tokenizer indexes all 3-character sequences, enabling efficient
/// substring search without full table scans.
fn search_by_fts_trigram(
    conn: &rusqlite::Connection,
    query: &str,
    language: &Option<String>,
    path_filter: &Option<String>,
    limit: i64,
) -> Result<Vec<(i64, i64, String)>> {
    // FTS5 trigram requires the query to be wrapped in double quotes for substring matching
    let fts_query = escape_fts5_trigram(query);

    let mut stmt = conn.prepare(
        "SELECT fl.file_id, fl.line_no, fl.content
         FROM file_lines_fts fts
         JOIN file_lines fl ON fts.rowid = fl.rowid
         JOIN files f ON fl.file_id = f.id
         WHERE file_lines_fts MATCH ?1
           AND (?2 IS NULL OR f.language = ?2)
           AND (?3 IS NULL OR f.path LIKE '%' || ?3 || '%')
         ORDER BY f.path, fl.line_no
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![fts_query, language, path_filter, limit],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
    )?;
    Ok(rows.flatten().collect())
}

/// Regex search with FTS5 pre-filtering: extract literal fragments from the pattern,
/// use trigram FTS to get candidates, then apply full regex in Rust.
fn search_regex(
    conn: &rusqlite::Connection,
    pattern: &str,
    language: &Option<String>,
    path_filter: &Option<String>,
    limit: i64,
) -> Result<Vec<(i64, i64, String)>> {
    let re = Regex::new(pattern)?;

    // Try to extract a literal substring from the regex for pre-filtering
    let literal = extract_longest_literal(pattern);

    if let Some(lit) = &literal {
        if lit.len() >= 3 {
            // Pre-filter using FTS5 trigram on the literal fragment, then apply regex
            let fts_query = escape_fts5_trigram(lit);
            let mut stmt = conn.prepare(
                "SELECT fl.file_id, fl.line_no, fl.content
                 FROM file_lines_fts fts
                 JOIN file_lines fl ON fts.rowid = fl.rowid
                 JOIN files f ON fl.file_id = f.id
                 WHERE file_lines_fts MATCH ?1
                   AND (?2 IS NULL OR f.language = ?2)
                   AND (?3 IS NULL OR f.path LIKE '%' || ?3 || '%')
                 ORDER BY f.path, fl.line_no",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![fts_query, language, path_filter],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
            )?;
            let results: Vec<(i64, i64, String)> = rows
                .flatten()
                .filter(|(_, _, content)| re.is_match(content))
                .take(limit as usize)
                .collect();
            return Ok(results);
        }
    }

    // Fallback: no useful literal to pre-filter, scan with language/path filters only
    let mut stmt = conn.prepare(
        "SELECT fl.file_id, fl.line_no, fl.content
         FROM file_lines fl
         JOIN files f ON fl.file_id = f.id
         WHERE (?1 IS NULL OR f.language = ?1)
           AND (?2 IS NULL OR f.path LIKE '%' || ?2 || '%')
         ORDER BY f.path, fl.line_no",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![language, path_filter],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
    )?;
    let results: Vec<(i64, i64, String)> = rows
        .flatten()
        .filter(|(_, _, content)| re.is_match(content))
        .take(limit as usize)
        .collect();
    Ok(results)
}


/// Batch fetch file paths for a set of file IDs.
fn batch_fetch_file_paths(
    conn: &rusqlite::Connection,
    file_ids: &[i64],
) -> Result<HashMap<i64, String>> {
    let mut map = HashMap::with_capacity(file_ids.len());
    // SQLite doesn't support array binds, so we use a prepared statement in a loop.
    // This is still fast since we have at most `limit` unique file_ids.
    let mut stmt = conn.prepare_cached("SELECT id, path FROM files WHERE id = ?1")?;
    for &fid in file_ids {
        if let Ok(path) = stmt.query_row([fid], |row| row.get::<_, String>(1)) {
            map.insert(fid, path);
        }
    }
    Ok(map)
}

/// Batch fetch context lines for all matches, grouping by file_id for efficiency.
fn batch_fetch_context(
    conn: &rusqlite::Connection,
    matches: &[(i64, i64, String)],
    file_paths: &HashMap<i64, String>,
    ctx: i64,
) -> Result<Vec<TextMatch>> {
    // Group matches by file_id to minimize queries
    let mut by_file: HashMap<i64, Vec<(i64, &str)>> = HashMap::new();
    for (file_id, line_no, content) in matches {
        by_file.entry(*file_id).or_default().push((*line_no, content.as_str()));
    }

    // For each file, fetch all needed context lines in one range query
    let mut stmt = conn.prepare_cached(
        "SELECT line_no, content FROM file_lines
         WHERE file_id = ?1 AND line_no BETWEEN ?2 AND ?3
         ORDER BY line_no",
    )?;

    // Build results in original match order
    let mut results = Vec::with_capacity(matches.len());
    for (file_id, line_no, content) in matches {
        let file_path = file_paths
            .get(file_id)
            .map(|s| s.as_str())
            .unwrap_or("");

        let before_start = (*line_no - ctx).max(1);
        let after_end = *line_no + ctx;

        let context_rows: Vec<(i64, String)> = stmt
            .query_map(rusqlite::params![file_id, before_start, after_end], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .flatten()
            .collect();

        let context_before: Vec<String> = context_rows
            .iter()
            .filter(|(ln, _)| *ln < *line_no)
            .map(|(_, c)| c.clone())
            .collect();
        let context_after: Vec<String> = context_rows
            .iter()
            .filter(|(ln, _)| *ln > *line_no)
            .map(|(_, c)| c.clone())
            .collect();

        results.push(TextMatch {
            file: file_path.to_string(),
            line_no: *line_no,
            content: content.clone(),
            context_before,
            context_after,
        });
    }

    Ok(results)
}

/// Escape a query for FTS5 trigram substring matching.
/// Trigram tokenizer requires the search term in double quotes for exact substring matching.
fn escape_fts5_trigram(query: &str) -> String {
    format!("\"{}\"", query.replace('"', "\"\""))
}

/// Extract the longest contiguous literal substring from a regex pattern.
/// This is used to pre-filter candidates via FTS5 before applying the full regex.
fn extract_longest_literal(pattern: &str) -> Option<String> {
    let mut best = String::new();
    let mut current = String::new();
    let mut in_escape = false;

    for ch in pattern.chars() {
        if in_escape {
            // Some escaped chars are literal (e.g., \n, \t are not useful)
            match ch {
                'n' | 't' | 'r' | 'd' | 'w' | 's' | 'D' | 'W' | 'S' | 'b' | 'B' => {
                    if current.len() > best.len() {
                        best = current.clone();
                    }
                    current.clear();
                }
                _ => current.push(ch),
            }
            in_escape = false;
        } else if ch == '\\' {
            in_escape = true;
        } else if ".*+?^${}()|[]".contains(ch) {
            // Metacharacter: end the current literal run
            if current.len() > best.len() {
                best = current.clone();
            }
            current.clear();
        } else {
            current.push(ch);
        }
    }
    if current.len() > best.len() {
        best = current;
    }

    if best.is_empty() {
        None
    } else {
        Some(best)
    }
}

/// Strip glob wildcard characters (* and ?) from a query.
/// The trigram tokenizer already does substring matching, so wildcards are redundant.
fn strip_glob_wildcards(query: &str) -> String {
    query.chars().filter(|c| *c != '*' && *c != '?').collect::<String>().trim().to_string()
}
