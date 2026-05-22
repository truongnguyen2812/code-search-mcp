use anyhow::Result;
use tracing::info;

use crate::db::DbPool;

/// Current schema version. Bump this when making breaking schema changes.
const SCHEMA_VERSION: i64 = 2;

/// Fast, non-blocking migration. Only creates base tables.
/// All heavy work (FTS rebuild, index creation) is deferred to background.
pub fn run_migrations(pool: &DbPool) -> Result<()> {
    let conn = pool.get()?;
    conn.execute_batch(CREATE_TABLES)?;

    // Check current schema version and schedule migration if needed
    let current_version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if current_version < SCHEMA_VERSION {
        info!(
            "Schema v{} detected (target: v{}). Heavy migration will run in background.",
            current_version, SCHEMA_VERSION
        );
        // Just mark that FTS needs rebuilding — don't do any heavy work here
        conn.execute(
            "INSERT OR REPLACE INTO index_progress(key, value) VALUES ('fts_status', 'pending_migration')",
            [],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO schema_version(version) VALUES (?1)",
            [SCHEMA_VERSION],
        )?;
    }

    Ok(())
}

/// Rebuild the FTS5 trigram index in the background with progress logging.
/// Also creates optional indexes that are too expensive for synchronous startup.
/// Call this from a spawned task so the server can start serving immediately.
pub fn rebuild_fts_background(pool: &DbPool) -> Result<()> {
    let conn = pool.get()?;

    // Create optional indexes in background (too slow for synchronous startup on large DBs)
    info!("Creating supplementary indexes in background...");
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_files_language ON files(language);
         CREATE INDEX IF NOT EXISTS idx_symbols_container ON symbols(container);",
    )?;
    info!("Supplementary indexes ready.");

    // Check if FTS rebuild/migration is needed
    let fts_status: String = conn
        .query_row(
            "SELECT COALESCE((SELECT value FROM index_progress WHERE key = 'fts_status'), 'ready')",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "ready".to_string());

    if fts_status == "ready" {
        return Ok(());
    }

    // If pending_migration, we need to drop old FTS and recreate with trigram tokenizer
    if fts_status == "pending_migration" {
        info!("Dropping old FTS table and recreating with trigram tokenizer...");
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS file_lines_ai;
             DROP TRIGGER IF EXISTS file_lines_ad;
             DROP TRIGGER IF EXISTS file_lines_au;
             DROP TABLE IF EXISTS file_lines_fts;",
        )?;
        conn.execute_batch(CREATE_FTS_TRIGRAM)?;
        conn.execute(
            "INSERT OR REPLACE INTO index_progress(key, value) VALUES ('fts_status', 'rebuilding')",
            [],
        )?;
        info!("FTS trigram table created. Starting data rebuild...");
    }

    let total_lines: i64 = conn
        .query_row("SELECT COUNT(*) FROM file_lines", [], |row| row.get(0))
        .unwrap_or(0);

    info!(
        "FTS trigram rebuild starting ({} lines to index)...",
        total_lines
    );

    // For very large databases, do a batched rebuild instead of the single 'rebuild' command
    // which can take hours and provides no progress feedback.
    if total_lines > 1_000_000 {
        rebuild_fts_batched(&conn, total_lines)?;
    } else {
        conn.execute_batch("INSERT INTO file_lines_fts(file_lines_fts) VALUES('rebuild');")?;
    }

    // Mark FTS as ready
    conn.execute(
        "INSERT OR REPLACE INTO index_progress(key, value) VALUES ('fts_status', 'ready')",
        [],
    )?;
    info!("FTS trigram rebuild complete.");
    Ok(())
}

/// Batched FTS rebuild with progress logging for large databases.
fn rebuild_fts_batched(conn: &rusqlite::Connection, total_lines: i64) -> Result<()> {
    const BATCH_SIZE: i64 = 500_000;
    let mut offset: i64 = 0;
    let mut indexed: i64 = 0;

    loop {
        let inserted = conn.execute(
            "INSERT INTO file_lines_fts(rowid, content)
             SELECT rowid, content FROM file_lines
             ORDER BY rowid
             LIMIT ?1 OFFSET ?2",
            rusqlite::params![BATCH_SIZE, offset],
        )?;

        indexed += inserted as i64;
        let pct = if total_lines > 0 {
            (indexed * 100) / total_lines
        } else {
            100
        };
        info!(
            "FTS rebuild progress: {}/{} lines ({}%)",
            indexed, total_lines, pct
        );

        if (inserted as i64) < BATCH_SIZE {
            break;
        }
        offset += BATCH_SIZE;
    }

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
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY
);

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
    content_rowid='id'
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
";

/// FTS5 with trigram tokenizer — enables fast substring matching via inverted index.
/// This replaces LIKE '%term%' full table scans with indexed lookups.
const CREATE_FTS_TRIGRAM: &str = "
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
