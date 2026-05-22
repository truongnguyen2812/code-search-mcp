use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::debug;

use crate::db::DbPool;
use crate::db::schema::is_fts_ready;
use crate::lsp::LspManager;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindReferencesInput {
    /// Symbol name to find references to
    pub symbol: String,
    /// Filter by language (optional)
    pub language: Option<String>,
    /// Maximum results (default: 100)
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ReferenceResult {
    pub file: String,
    pub line_no: i64,
    pub content: String,
    pub language: String,
}

pub struct FindReferencesTool {
    pub pool: DbPool,
    pub lsp: Option<Arc<LspManager>>,
}

impl FindReferencesTool {
    /// Find references by first trying LSP (precise), then falling back to SQLite LIKE (heuristic).
    pub fn find(&self, input: FindReferencesInput) -> Result<Vec<ReferenceResult>> {
        let conn = self.pool.get()?;
        let limit = input.limit.unwrap_or(100).min(500);

        // Try LSP: look up the symbol's definition location, then ask LSP for all references
        if let Some(lsp) = &self.lsp {
            match self.find_via_lsp(&conn, lsp, &input.symbol, &input.language, limit) {
                Ok(results) if !results.is_empty() => {
                    debug!("LSP returned {} reference(s) for '{}'", results.len(), input.symbol);
                    return Ok(results);
                }
                Err(e) => debug!("LSP find_references failed, falling back to index: {e}"),
                Ok(_) => {}
            }
        }

        // Fallback: LIKE search on file_lines
        self.find_via_index(&conn, &input, limit)
    }

    fn find_via_lsp(
        &self,
        conn: &rusqlite::Connection,
        lsp: &LspManager,
        symbol: &str,
        language: &Option<String>,
        limit: i64,
    ) -> Result<Vec<ReferenceResult>> {
        // Find the symbol's definition in the index to get its file+position
        let row: Option<(String, i64, i64, String)> = conn.query_row(
            "SELECT f.path, s.start_line, s.start_col, f.language
             FROM symbols s
             JOIN files f ON s.file_id = f.id
             WHERE s.name = ?1
               AND (?2 IS NULL OR f.language = ?2)
               AND s.kind IN ('class','interface','enum','function','method','constructor','struct','object','annotation')
             ORDER BY length(s.container) ASC
             LIMIT 1",
            rusqlite::params![symbol, language],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).ok();

        let (def_file, def_line, def_col, _def_lang) = if let Some(r) = row {
            r
        } else {
            let anchor: Option<(String, i64, i64, String)> = if is_fts_ready(conn) {
                let fts_query = format!("\"{}\"", symbol.replace('"', "\"\""));
                conn.query_row(
                    "SELECT f.path, fl.line_no, instr(fl.content, ?1), f.language
                     FROM file_lines_fts fts
                     JOIN file_lines fl ON fts.rowid = fl.rowid
                     JOIN files f ON fl.file_id = f.id
                     WHERE file_lines_fts MATCH ?2
                       AND (?3 IS NULL OR f.language = ?3)
                     ORDER BY f.path, fl.line_no
                     LIMIT 1",
                    rusqlite::params![symbol, fts_query, language],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                ).ok()
            } else {
                let pattern = format!("%{}%", symbol);
                conn.query_row(
                    "SELECT f.path, fl.line_no, instr(fl.content, ?1), f.language
                     FROM file_lines fl
                     JOIN files f ON fl.file_id = f.id
                     WHERE fl.content LIKE ?2
                       AND (?3 IS NULL OR f.language = ?3)
                     ORDER BY f.path, fl.line_no
                     LIMIT 1",
                    rusqlite::params![symbol, pattern, language],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                ).ok()
            };

            let Some((file, line_no, col_1based, lang)) = anchor else {
                return Ok(vec![]);
            };
            (file, line_no.saturating_sub(1), col_1based.saturating_sub(1), lang)
        };

        let def_path = std::path::Path::new(&def_file);
        let lsp_refs = lsp.find_references(def_path, def_line as u32, def_col as u32)?;

        // For each LSP location, fetch the line content from the index
        let mut results = Vec::new();
        for (ref_file, ref_line, _ref_col) in lsp_refs.into_iter().take(limit as usize) {
            let (content, lang): (String, String) = conn.query_row(
                "SELECT fl.content, f.language
                 FROM file_lines fl
                 JOIN files f ON fl.file_id = f.id
                 WHERE f.path = ?1 AND fl.line_no = ?2",
                rusqlite::params![ref_file, ref_line as i64 + 1], // LSP is 0-based
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap_or_else(|_| (String::new(), String::new()));

            results.push(ReferenceResult {
                file: ref_file,
                line_no: ref_line as i64 + 1, // convert to 1-based
                content,
                language: lang,
            });
        }

        Ok(results)
    }

    fn find_via_index(
        &self,
        conn: &rusqlite::Connection,
        input: &FindReferencesInput,
        limit: i64,
    ) -> Result<Vec<ReferenceResult>> {
        if is_fts_ready(conn) {
            // Use FTS5 trigram index for fast substring matching
            let fts_query = format!("\"{}\"", input.symbol.replace('"', "\"\""));
            let mut stmt = conn.prepare(
                "SELECT f.path, fl.line_no, fl.content, f.language
                 FROM file_lines_fts fts
                 JOIN file_lines fl ON fts.rowid = fl.rowid
                 JOIN files f ON fl.file_id = f.id
                 WHERE file_lines_fts MATCH ?1
                   AND (?2 IS NULL OR f.language = ?2)
                 ORDER BY f.path, fl.line_no
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![fts_query, input.language, limit],
                |row| {
                    Ok(ReferenceResult {
                        file: row.get(0)?,
                        line_no: row.get(1)?,
                        content: row.get(2)?,
                        language: row.get(3)?,
                    })
                },
            )?;
            Ok(rows.flatten().collect())
        } else {
            // Fallback to LIKE while FTS is rebuilding
            let pattern = format!("%{}%", input.symbol);
            let mut stmt = conn.prepare(
                "SELECT f.path, fl.line_no, fl.content, f.language
                 FROM file_lines fl
                 JOIN files f ON fl.file_id = f.id
                 WHERE fl.content LIKE ?1
                   AND (?2 IS NULL OR f.language = ?2)
                 ORDER BY f.path, fl.line_no
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![pattern, input.language, limit],
                |row| {
                    Ok(ReferenceResult {
                        file: row.get(0)?,
                        line_no: row.get(1)?,
                        content: row.get(2)?,
                        language: row.get(3)?,
                    })
                },
            )?;
            Ok(rows.flatten().collect())
        }
    }
}
