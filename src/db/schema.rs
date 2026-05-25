use anyhow::Result;
use tracing::info;

use crate::db::DbPool;

/// Fast, non-blocking table creation. Creates all base tables, FTS tables, and triggers.
pub fn run_migrations(pool: &DbPool) -> Result<()> {
    let conn = pool.get()?;
    conn.execute_batch(CREATE_TABLES)?;

    // Ensure fts_status is set to 'ready'
    conn.execute(
        "INSERT OR IGNORE INTO index_progress(key, value) VALUES ('fts_status', 'ready')",
        [],
    )?;

    Ok(())
}

/// Rebuild or create optional indexes in the background.
/// Call this from a spawned task so the server can start serving immediately.
pub fn rebuild_fts_background(pool: &DbPool) -> Result<()> {
    let mut conn = pool.get()?;

    // Self-healing migration for trigram symbols_fts (runs in background)
    let has_trigram: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE name='symbols_fts' AND sql LIKE '%tokenize=%'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if !has_trigram {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE name='symbols_fts'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if exists {
            info!("Upgrading symbols_fts table to use trigram tokenizer in background...");
            let _ = conn.execute(
                "INSERT OR REPLACE INTO index_progress(key, value) VALUES ('fts_status', 'upgrading')",
                [],
            );
            
            let _ = conn.execute("DROP TABLE IF EXISTS symbols_fts", []);
            let _ = conn.execute(
                "CREATE VIRTUAL TABLE symbols_fts USING fts5(
                    name,
                    container,
                    kind,
                    content='symbols',
                    content_rowid='id',
                    tokenize='trigram'
                );",
                [],
            );

            info!("Populating symbols_fts index with trigram tokens in background...");
            let total: i64 = conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0)).unwrap_or(0);
            if total > 0 {
                let min_id: i64 = conn.query_row("SELECT COALESCE(MIN(id), 0) FROM symbols", [], |r| r.get(0)).unwrap_or(0);
                let max_id: i64 = conn.query_row("SELECT COALESCE(MAX(id), 0) FROM symbols", [], |r| r.get(0)).unwrap_or(0);
                
                let batch_size = 50000;
                let mut current_id = min_id;
                let mut processed = 0;
                
                while current_id <= max_id {
                    let next_id = current_id + batch_size;
                    
                    let tx = conn.transaction()?;
                    tx.execute(
                        "INSERT INTO symbols_fts(rowid, name, container, kind)
                         SELECT id, name, container, kind
                         FROM symbols
                         WHERE id >= ?1 AND id < ?2",
                        rusqlite::params![current_id, next_id],
                    )?;
                    tx.commit()?;
                    
                    let batch_count: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM symbols WHERE id >= ?1 AND id < ?2",
                        rusqlite::params![current_id, next_id],
                        |r| r.get(0),
                    ).unwrap_or(0);
                    
                    processed += batch_count;
                    current_id = next_id;
                    
                    let done_val = std::cmp::min(processed as usize, total as usize);
                    let bar_str = format_progress_bar(done_val, total as usize);
                    info!("Upgrading symbols_fts: {bar_str}");
                }
            }
            
            let _ = conn.execute(
                "INSERT OR REPLACE INTO index_progress(key, value) VALUES ('fts_status', 'ready')",
                [],
            );
            info!("symbols_fts index upgrade complete.");
        }
    }

    // Create optional indexes in background (too slow for synchronous startup on large DBs)
    info!("Creating supplementary indexes in background...");
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_files_language ON files(language);
         CREATE INDEX IF NOT EXISTS idx_symbols_container ON symbols(container);",
    )?;
    info!("Supplementary indexes ready.");

    Ok(())
}

/// Check if FTS trigram index is ready for use.
pub fn is_fts_ready(conn: &rusqlite::Connection) -> bool {
    conn.query_row(
        "SELECT COALESCE((SELECT value FROM index_progress WHERE key = 'fts_status'), 'ready')",
        [],
        |row| row.get::<_, String>(0),
    )
    .map(|s| s == "ready")
    .unwrap_or(true)
}

const CREATE_TABLES: &str = "
CREATE TABLE IF NOT EXISTS files (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    path    TEXT    UNIQUE NOT NULL,
    language TEXT   NOT NULL,
    mtime   INTEGER NOT NULL DEFAULT 0,
    hash    TEXT    NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS symbols (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    kind        TEXT    NOT NULL,
    start_line  INTEGER NOT NULL DEFAULT 0,
    start_col   INTEGER NOT NULL DEFAULT 0,
    end_line    INTEGER NOT NULL DEFAULT 0,
    end_col     INTEGER NOT NULL DEFAULT 0,
    container   TEXT    NOT NULL DEFAULT '',
    signature   TEXT    NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_symbols_name     ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_file_id  ON symbols(file_id);
CREATE INDEX IF NOT EXISTS idx_symbols_kind     ON symbols(kind);

CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
    name,
    container,
    kind,
    content='symbols',
    content_rowid='id',
    tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
    INSERT INTO symbols_fts(rowid, name, container, kind)
    VALUES (new.id, new.name, new.container, new.kind);
END;

CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
    INSERT INTO symbols_fts(symbols_fts, rowid, name, container, kind)
    VALUES ('delete', old.id, old.name, old.container, old.kind);
END;

CREATE TRIGGER IF NOT EXISTS symbols_au AFTER UPDATE ON symbols BEGIN
    INSERT INTO symbols_fts(symbols_fts, rowid, name, container, kind)
    VALUES ('delete', old.id, old.name, old.container, old.kind);
    INSERT INTO symbols_fts(rowid, name, container, kind)
    VALUES (new.id, new.name, new.container, new.kind);
END;

CREATE TABLE IF NOT EXISTS file_lines (
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    line_no INTEGER NOT NULL,
    content TEXT    NOT NULL,
    PRIMARY KEY (file_id, line_no)
);

CREATE TABLE IF NOT EXISTS index_progress (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS file_lines_fts USING fts5(
    content,
    content='file_lines',
    content_rowid='rowid',
    tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS file_lines_ai AFTER INSERT ON file_lines BEGIN
    INSERT INTO file_lines_fts(rowid, content)
    VALUES (new.rowid, new.content);
END;

CREATE TRIGGER IF NOT EXISTS file_lines_ad AFTER DELETE ON file_lines BEGIN
    INSERT INTO file_lines_fts(file_lines_fts, rowid, content)
    VALUES ('delete', old.rowid, old.content);
END;

CREATE TRIGGER IF NOT EXISTS file_lines_au AFTER UPDATE ON file_lines BEGIN
    INSERT INTO file_lines_fts(file_lines_fts, rowid, content)
    VALUES ('delete', old.rowid, old.content);
    INSERT INTO file_lines_fts(rowid, content)
    VALUES (new.rowid, new.content);
END;
";

/// Generates a visual text-based progress bar for console logging.
fn format_progress_bar(done: usize, total: usize) -> String {
    let width = 20;
    let percent = if total > 0 { done as f64 / total as f64 } else { 0.0 };
    let filled = (percent * width as f64).round() as usize;
    let mut bar = String::new();
    for i in 0..width {
        if i < filled {
            bar.push('█');
        } else {
            bar.push('░');
        }
    }
    format!("[{bar}] {done}/{total} ({:.1}%)", percent * 100.0)
}
