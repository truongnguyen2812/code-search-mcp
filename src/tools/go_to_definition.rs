use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::debug;

use crate::db::DbPool;
use crate::db::schema::is_fts_ready;
use crate::lsp::LspManager;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GoToDefinitionInput {
    /// Symbol name to find the definition of
    pub symbol: String,
    /// Optional: narrow by language
    pub language: Option<String>,
    /// Optional: container/class name to disambiguate
    pub container: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DefinitionResult {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub language: String,
    pub start_line: i64,
    pub start_col: i64,
    pub end_line: i64,
    pub end_col: i64,
    pub container: String,
    pub signature: String,
}

pub struct GoToDefinitionTool {
    pub pool: DbPool,
    pub lsp: Option<Arc<LspManager>>,
}

impl GoToDefinitionTool {
    pub fn find(&self, input: GoToDefinitionInput) -> Result<Vec<DefinitionResult>> {
        let conn = self.pool.get()?;

        let mut stmt = conn.prepare(
            "SELECT s.name, s.kind, f.path, f.language, s.start_line, s.start_col,
                    s.end_line, s.end_col, s.container, s.signature
             FROM symbols s
             JOIN files f ON s.file_id = f.id
             WHERE s.name = ?1
               AND (?2 IS NULL OR f.language = ?2)
               AND (?3 IS NULL OR s.container LIKE '%' || ?3 || '%')
               AND s.kind IN ('class','interface','enum','function','method','constructor','struct','object','annotation')
             ORDER BY
               CASE s.kind
                 WHEN 'class' THEN 1
                 WHEN 'interface' THEN 2
                 WHEN 'struct' THEN 3
                 ELSE 4
               END,
               length(s.container)
             LIMIT 20",
        )?;

        let rows = stmt.query_map(
            rusqlite::params![input.symbol, input.language, input.container],
            |row| {
                Ok(DefinitionResult {
                    name: row.get(0)?,
                    kind: row.get(1)?,
                    file: row.get(2)?,
                    language: row.get(3)?,
                    start_line: row.get(4)?,
                    start_col: row.get(5)?,
                    end_line: row.get(6)?,
                    end_col: row.get(7)?,
                    container: row.get(8)?,
                    signature: row.get(9)?,
                })
            },
        )?;

        let mut results: Vec<DefinitionResult> = rows.flatten().collect();

        // Try LSP for the first result to get a more precise location
        if let Some(lsp) = &self.lsp {
            if let Some(first) = results.first() {
                let file_path_str = first.file.clone();
                let first_start_line = first.start_line;
                let first_start_col = first.start_col;
                let first_kind = first.kind.clone();
                let first_language = first.language.clone();
                let first_container = first.container.clone();
                let first_signature = first.signature.clone();

                let file_path = std::path::Path::new(&file_path_str);
                match lsp.goto_definition(file_path, first_start_line as u32, first_start_col as u32) {
                    Ok(lsp_locs) if !lsp_locs.is_empty() => {
                        debug!("LSP returned {} definition(s) for '{}'", lsp_locs.len(), input.symbol);
                        let existing_files: std::collections::HashSet<String> =
                            results.iter().map(|r| r.file.clone()).collect();
                        let mut inserts = Vec::new();
                        for (lsp_file, lsp_line, lsp_col) in lsp_locs {
                            if !existing_files.contains(&lsp_file) {
                                inserts.push(DefinitionResult {
                                    name: input.symbol.clone(),
                                    kind: first_kind.clone(),
                                    file: lsp_file,
                                    language: first_language.clone(),
                                    start_line: lsp_line as i64,
                                    start_col: lsp_col as i64,
                                    end_line: lsp_line as i64,
                                    end_col: lsp_col as i64,
                                    container: first_container.clone(),
                                    signature: first_signature.clone(),
                                });
                            }
                        }
                        // Prepend LSP results
                        let mut new_results = inserts;
                        new_results.extend(results);
                        results = new_results;
                    }
                    Err(e) => debug!("LSP goto_definition failed: {e}"),
                    Ok(_) => {}
                }
            }

            if results.is_empty() {
                let mut anchor_stmt = if is_fts_ready(&conn) {
                    let fts_query = format!("\"{}\"", input.symbol.replace('"', "\"\""));
                    let stmt = conn.prepare(
                        "SELECT f.path, fl.line_no, fl.content, f.language
                         FROM file_lines_fts fts
                         JOIN file_lines fl ON fts.rowid = fl.rowid
                         JOIN files f ON fl.file_id = f.id
                         WHERE file_lines_fts MATCH ?1
                           AND (?2 IS NULL OR f.language = ?2)
                         ORDER BY f.path, fl.line_no
                         LIMIT 30",
                    )?;
                    (stmt, fts_query)
                } else {
                    let pattern = format!("%{}%", input.symbol);
                    let stmt = conn.prepare(
                        "SELECT f.path, fl.line_no, fl.content, f.language
                         FROM file_lines fl
                         JOIN files f ON fl.file_id = f.id
                         WHERE fl.content LIKE ?1
                           AND (?2 IS NULL OR f.language = ?2)
                         ORDER BY f.path, fl.line_no
                         LIMIT 30",
                    )?;
                    (stmt, pattern)
                };

                let anchors = anchor_stmt.0.query_map(
                    rusqlite::params![anchor_stmt.1, input.language],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )?;

                for (path, line_no, content, lang) in anchors.flatten() {
                    let Some(col) = content.find(&input.symbol) else {
                        continue;
                    };
                    match lsp.goto_definition(
                        std::path::Path::new(&path),
                        (line_no.saturating_sub(1)) as u32,
                        col as u32,
                    ) {
                        Ok(lsp_locs) if !lsp_locs.is_empty() => {
                            results = lsp_locs
                                .into_iter()
                                .map(|(file, line, col)| DefinitionResult {
                                    name: input.symbol.clone(),
                                    kind: "definition".to_string(),
                                    file,
                                    language: lang.clone(),
                                    start_line: line as i64,
                                    start_col: col as i64,
                                    end_line: line as i64,
                                    end_col: col as i64,
                                    container: String::new(),
                                    signature: String::new(),
                                })
                                .collect();
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(results)
    }
}
