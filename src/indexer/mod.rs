pub mod cpp;
pub mod go;
pub mod gradle;
pub mod groovy;
pub mod java;
pub mod json;
pub mod kotlin;
pub mod make;
pub mod ruby;
pub mod xml;
pub mod yaml;
pub mod aidl;

use anyhow::Result;
use ignore::WalkBuilder;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};

use crate::db::DbPool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Language {
    Java,
    Kotlin,
    C,
    Cpp,
    Make,
    Go,
    Groovy,
    Gradle,
    Ruby,
    Json,
    Xml,
    Yaml,
    Aidl,
    Other(String),
}

impl Language {
    pub fn from_path(path: &Path) -> Option<Self> {
        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            match file_name {
                "Makefile" | "makefile" | "GNUmakefile" => {
                    return Some(Language::Make);
                }
                _ => {}
            }
        }

        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "java" => Some(Language::Java),
            "kt" | "kts" => Some(Language::Kotlin),
            "c" | "h" => Some(Language::C),
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "c++" => Some(Language::Cpp),
            "go" => Some(Language::Go),
            "groovy" => Some(Language::Groovy),
            "gradle" => Some(Language::Gradle),
            "rb" => Some(Language::Ruby),
            "json" => Some(Language::Json),
            "xml" => Some(Language::Xml),
            "yaml" | "yml" => Some(Language::Yaml),
            "aidl" => Some(Language::Aidl),
            "mk" => Some(Language::Make),
            // Additional source/config formats: indexed for file/text/reference search.
            "toml" | "py" | "js" | "jsx" | "ts" | "tsx" | "rs" | "swift"
            | "scala" | "sh" | "bash" | "zsh" | "bp" | "proto"
            | "smali" => Some(Language::Other(ext)),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Language::Java => "java",
            Language::Kotlin => "kotlin",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Make => "make",
            Language::Go => "go",
            Language::Groovy => "groovy",
            Language::Gradle => "gradle",
            Language::Ruby => "rb",
            Language::Json => "json",
            Language::Xml => "xml",
            Language::Yaml => "yaml",
            Language::Aidl => "aidl",
            Language::Other(lang) => lang.as_str(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub container: String,
    pub signature: String,
}

struct WriteTask {
    path: PathBuf,
    lang: Language,
    mtime: i64,
    hash: String,
    symbols: Vec<Symbol>,
    lines: Vec<String>,
}

fn write_parsed_file(conn: &mut rusqlite::Connection, task: WriteTask) -> Result<()> {
    let tx = conn.transaction()?;

    tx.execute(
        "INSERT INTO files(path, language, mtime, hash)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path) DO UPDATE SET language=excluded.language, mtime=excluded.mtime, hash=excluded.hash",
        rusqlite::params![
            task.path.to_string_lossy().as_ref(),
            task.lang.as_str(),
            task.mtime,
            task.hash
        ],
    )?;

    let file_id: i64 = tx.query_row(
        "SELECT id FROM files WHERE path = ?1",
        [task.path.to_string_lossy().as_ref()],
        |row| row.get(0),
    )?;

    tx.execute("DELETE FROM symbols WHERE file_id = ?1", [file_id])?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO symbols(file_id, name, kind, start_line, start_col, end_line, end_col, container, signature)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        )?;
        for sym in &task.symbols {
            stmt.execute(rusqlite::params![
                file_id,
                sym.name,
                sym.kind,
                sym.start_line,
                sym.start_col,
                sym.end_line,
                sym.end_col,
                sym.container,
                sym.signature,
            ])?;
        }
    }

    tx.execute("DELETE FROM file_lines WHERE file_id = ?1", [file_id])?;
    {
        let mut line_stmt = tx.prepare_cached(
            "INSERT INTO file_lines(file_id, line_no, content) VALUES (?1,?2,?3)",
        )?;
        for (i, line) in task.lines.iter().enumerate() {
            line_stmt.execute(rusqlite::params![file_id, i as i64 + 1, line])?;
        }
    }

    tx.commit()?;
    Ok(())
}

fn parse_and_send(
    pool: &DbPool,
    path: &Path,
    lang: &Language,
    tx: &tokio::sync::mpsc::Sender<WriteTask>,
) -> Result<bool> {
    let metadata = path.metadata().ok();
    let mtime = metadata
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let file_size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

    // Skip very large files (> 2 MB) to keep indexing fast
    if file_size > 2 * 1024 * 1024 {
        debug!("Skipping large file {}", path.display());
        return Ok(false);
    }

    // Fast binary heuristic check
    if is_binary_file(path) {
        debug!("Skipping binary file {}", path.display());
        return Ok(false);
    }

    let conn = pool.get()?;

    // Fast path: skip if mtime is unchanged (avoids reading the file at all)
    let existing_mtime: Option<i64> = conn
        .query_row(
            "SELECT mtime FROM files WHERE path = ?1",
            [path.to_string_lossy().as_ref()],
            |row| row.get(0),
        )
        .ok();
    if existing_mtime == Some(mtime) {
        return Ok(false);
    }
    drop(conn);

    let source = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            debug!("Cannot read {}: {e}", path.display());
            return Ok(false);
        }
    };

    let hash = hex::encode(Sha256::digest(&source));

    // Re-acquire briefly for the hash check
    let conn = pool.get()?;
    let existing_hash: Option<String> = conn
        .query_row(
            "SELECT hash FROM files WHERE path = ?1",
            [path.to_string_lossy().as_ref()],
            |row| row.get(0),
        )
        .ok();
    if existing_hash.as_deref() == Some(&hash) {
        // Update mtime so we skip faster next time
        conn.execute(
            "UPDATE files SET mtime = ?1 WHERE path = ?2",
            rusqlite::params![mtime, path.to_string_lossy().as_ref()],
        )?;
        return Ok(false);
    }
    drop(conn); // release before CPU-intensive parsing below

    let source_str = match std::str::from_utf8(&source) {
        Ok(s) => s,
        Err(_) => return Ok(false), // skip non-UTF-8 files
    };

    // CPU-intensive tree-sitter parsing
    let symbols: Vec<Symbol> = match lang {
        Language::Java => java::extract_symbols(source_str),
        Language::Kotlin => kotlin::extract_symbols(source_str),
        Language::C | Language::Cpp => cpp::extract_symbols(source_str),
        Language::Make => make::extract_symbols(source_str),
        Language::Go => go::extract_symbols(source_str),
        Language::Groovy => groovy::extract_symbols(source_str),
        Language::Gradle => gradle::extract_symbols(source_str),
        Language::Ruby => ruby::extract_symbols(source_str),
        Language::Json => json::extract_symbols(source_str),
        Language::Xml => xml::extract_symbols(source_str),
        Language::Yaml => yaml::extract_symbols(source_str),
        Language::Aidl => aidl::extract_symbols(source_str),
        Language::Other(_) => Vec::new(),
    };

    // Collect lines
    let lines: Vec<String> = if matches!(lang, Language::Other(_)) {
        Vec::new()
    } else {
        source_str.lines().map(|s| s.to_string()).collect()
    };

    let task = WriteTask {
        path: path.to_path_buf(),
        lang: lang.clone(),
        mtime,
        hash,
        symbols,
        lines,
    };

    let _ = tx.blocking_send(task);
    Ok(true)
}

/// Index all source files under `root` using up to `threads` parallel tasks.
pub async fn index_codebase(pool: &DbPool, root: &Path, threads: usize) -> Result<()> {
    info!("Beginning full index of {}", root.display());

    let sem = Arc::new(Semaphore::new(threads));
    let pool = pool.clone();
    let root = root.to_path_buf();

    // Collect files first (WalkBuilder is not Send across awaits)
    let files: Vec<(PathBuf, Language)> = {
        let mut collected = Vec::new();
        let walker = WalkBuilder::new(&root)
            .follow_links(true)
            .same_file_system(false)
            // We apply our own skip logic in should_skip(); disabling ignore rules
            // avoids silently dropping large vendor trees from nested .gitignore files.
            .hidden(false)
            .ignore(false)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .build();

        for entry in walker.flatten() {
            let path = entry.path().to_path_buf();
            if !path.is_file() {
                continue;
            }
            // Skip heavyweight generated / binary directories
            if should_skip(&path, &root) {
                continue;
            }
            if let Some(lang) = Language::from_path(&path) {
                collected.push((path, lang));
            }
        }
        collected
    };

    let total = files.len();
    info!("Found {total} source files to index");

    update_progress(&pool, "total_files", &total.to_string())?;
    update_progress(&pool, "indexed_files", "0")?;
    update_progress(&pool, "status", "indexing")?;

    let log_interval = std::cmp::max(1, total / 10);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<WriteTask>(100);
    let mut handles = Vec::new();
    let indexed_count = Arc::new(AtomicUsize::new(0));

    // Spawn a single dedicated database writer thread
    let writer_pool = pool.clone();
    let writer_counter = indexed_count.clone();
    let writer_handle = tokio::task::spawn_blocking(move || {
        let mut conn = match writer_pool.get() {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Writer failed to get connection: {e}");
                return;
            }
        };
        while let Some(task) = rx.blocking_recv() {
            let path_str = task.path.display().to_string();
            match write_parsed_file(&mut conn, task) {
                Ok(_) => {}
                Err(e) => tracing::warn!("Failed to write {path_str} to database: {e}"),
            }
            let done = writer_counter.fetch_add(1, Ordering::Relaxed) + 1;
            if done % 500 == 0 {
                let _ = update_progress(&writer_pool, "indexed_files", &done.to_string());
            }
            if done % log_interval == 0 || done == total {
                let bar_str = format_progress_bar(done, total);
                tracing::info!("Indexing progress: {bar_str}");
            }
        }
    });

    for (path, lang) in files {
        let permit = sem.clone().acquire_owned().await?;
        let pool_clone = pool.clone();
        let tx_clone = tx.clone();
        let counter = indexed_count.clone();

        let handle = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut sent = false;
            match parse_and_send(&pool_clone, &path, &lang, &tx_clone) {
                Ok(s) => sent = s,
                Err(e) => warn!("Failed to parse {}: {e}", path.display()),
            }
            if !sent {
                let done = counter.fetch_add(1, Ordering::Relaxed) + 1;
                if done % 500 == 0 {
                    let _ = update_progress(&pool_clone, "indexed_files", &done.to_string());
                }
                if done % log_interval == 0 || done == total {
                    let bar_str = format_progress_bar(done, total);
                    info!("Indexing progress: {bar_str}");
                }
            }
        });
        handles.push(handle);
    }

    // Drop the main sender so the writer terminates when all workers finish and drop their senders
    drop(tx);

    for handle in handles {
        let _ = handle.await;
    }
    let _ = writer_handle.await;

    let final_count = indexed_count.load(Ordering::Relaxed);
    update_progress(&pool, "indexed_files", &final_count.to_string())?;
    update_progress(&pool, "status", "done")?;
    info!("Indexing complete ({final_count} files processed)");
    Ok(())
}

fn should_skip(path: &Path, root: &Path) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let s = rel.to_string_lossy();

    // Skip generated build output, repo metadata, and prebuilt/binary-heavy trees
    // that rarely contain source worth indexing.
    if s.starts_with("out/")
        || s.starts_with(".repo/")
        || s.starts_with(".git/")
        || s.starts_with("prebuilts/")
        || s.contains("/.git/")
        || s.contains("/out/")
    {
        return true;
    }

    // Skip binary/compiled artefacts regardless of location
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("so" | "a" | "o" | "class" | "jar" | "aar" | "dex"
            | "apk" | "img" | "bin" | "zip" | "gz" | "tar" | "7z"
            | "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico"
            | "ttf" | "otf" | "woff" | "woff2"
            | "mp3" | "mp4" | "ogg" | "wav")
    )
}

/// Index a single file: parse symbols and store lines.
pub fn index_file(pool: &DbPool, path: &Path, lang: &Language) -> Result<()> {
    let metadata = path.metadata().ok();
    let mtime = metadata
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let file_size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

    // Skip very large files (> 2 MB) to keep indexing fast
    if file_size > 2 * 1024 * 1024 {
        debug!("Skipping large file {}", path.display());
        return Ok(());
    }

    // Fast binary heuristic check
    if is_binary_file(path) {
        debug!("Skipping binary file {}", path.display());
        return Ok(());
    }

    // --- Phase 1: read-only checks (short-lived connection, released before parsing) ---
    let (source, hash) = {
        let conn = pool.get()?;

        // Fast path: skip if mtime is unchanged (avoids reading the file at all)
        let existing_mtime: Option<i64> = conn
            .query_row(
                "SELECT mtime FROM files WHERE path = ?1",
                [path.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .ok();
        if existing_mtime == Some(mtime) {
            return Ok(());
        }

        // conn is dropped here, freeing the pool slot before we do I/O
        drop(conn);

        let source = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                debug!("Cannot read {}: {e}", path.display());
                return Ok(());
            }
        };

        let hash = hex::encode(Sha256::digest(&source));

        // Re-acquire briefly for the hash check
        let conn = pool.get()?;
        let existing_hash: Option<String> = conn
            .query_row(
                "SELECT hash FROM files WHERE path = ?1",
                [path.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .ok();
        if existing_hash.as_deref() == Some(&hash) {
            // Update mtime so we skip faster next time
            conn.execute(
                "UPDATE files SET mtime = ?1 WHERE path = ?2",
                rusqlite::params![mtime, path.to_string_lossy().as_ref()],
            )?;
            return Ok(());
        }
        drop(conn); // release before CPU-intensive parsing below

        (source, hash)
    };

    let source_str = match std::str::from_utf8(&source) {
        Ok(s) => s,
        Err(_) => return Ok(()), // skip non-UTF-8 files
    };

    // --- Phase 2: CPU-intensive tree-sitter parsing (no DB connection held) ---
    let symbols: Vec<Symbol> = match lang {
        Language::Java => java::extract_symbols(source_str),
        Language::Kotlin => kotlin::extract_symbols(source_str),
        Language::C | Language::Cpp => cpp::extract_symbols(source_str),
        Language::Make => make::extract_symbols(source_str),
        Language::Go => go::extract_symbols(source_str),
        Language::Groovy => groovy::extract_symbols(source_str),
        Language::Gradle => gradle::extract_symbols(source_str),
        Language::Ruby => ruby::extract_symbols(source_str),
        Language::Json => json::extract_symbols(source_str),
        Language::Xml => xml::extract_symbols(source_str),
        Language::Yaml => yaml::extract_symbols(source_str),
        Language::Aidl => aidl::extract_symbols(source_str),
        Language::Other(_) => Vec::new(),
    };

    // Collect lines; skip for Other() languages since there are no symbols to look up
    let lines: Vec<&str> = if matches!(lang, Language::Other(_)) {
        Vec::new()
    } else {
        source_str.lines().collect()
    };

    // --- Phase 3: write transaction (connection acquired only now) ---
    let mut conn = pool.get()?;
    let tx = conn.transaction()?;

    tx.execute(
        "INSERT INTO files(path, language, mtime, hash)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path) DO UPDATE SET language=excluded.language, mtime=excluded.mtime, hash=excluded.hash",
        rusqlite::params![
            path.to_string_lossy().as_ref(),
            lang.as_str(),
            mtime,
            hash
        ],
    )?;

    let file_id: i64 = tx.query_row(
        "SELECT id FROM files WHERE path = ?1",
        [path.to_string_lossy().as_ref()],
        |row| row.get(0),
    )?;

    tx.execute("DELETE FROM symbols WHERE file_id = ?1", [file_id])?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO symbols(file_id, name, kind, start_line, start_col, end_line, end_col, container, signature)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        )?;
        for sym in &symbols {
            stmt.execute(rusqlite::params![
                file_id,
                sym.name,
                sym.kind,
                sym.start_line,
                sym.start_col,
                sym.end_line,
                sym.end_col,
                sym.container,
                sym.signature,
            ])?;
        }
    }

    tx.execute("DELETE FROM file_lines WHERE file_id = ?1", [file_id])?;
    {
        let mut line_stmt = tx.prepare_cached(
            "INSERT INTO file_lines(file_id, line_no, content) VALUES (?1,?2,?3)",
        )?;
        for (i, line) in lines.iter().enumerate() {
            line_stmt.execute(rusqlite::params![file_id, i as i64 + 1, line])?;
        }
    }

    tx.commit()?;
    Ok(())
}

fn update_progress(pool: &DbPool, key: &str, value: &str) -> Result<()> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO index_progress(key, value) VALUES(?1,?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

fn is_binary_file(path: &Path) -> bool {
    use std::io::Read;
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = [0u8; 1024];
    let n = match file.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };
    if n == 0 {
        return false;
    }
    let slice = &buf[..n];
    if slice.contains(&0) {
        return true;
    }
    // High ratio of non-ASCII control characters (> 3%)
    let mut control_chars = 0;
    for &b in slice {
        if b < 32 && b != 9 && b != 10 && b != 13 {
            control_chars += 1;
        }
    }
    control_chars * 100 > n * 3
}

/// Generic stack-based iterative tree-sitter AST traversal to avoid stack overflows.
/// The closure `f` is called for each node.
/// Returning `Some(Some(new_container))` visits children with a new container string.
/// Returning `Some(None)` visits children with the same container.
/// Returning `None` skips visiting the children.
pub fn traverse_tree<F>(root: tree_sitter::Node, root_container: &str, mut f: F)
where
    F: FnMut(tree_sitter::Node, &str) -> Option<Option<String>>,
{
    let mut stack = vec![(root, root_container.to_string())];
    while let Some((node, container)) = stack.pop() {
        match f(node, &container) {
            Some(Some(new_container)) => {
                for i in 0..node.child_count() {
                    stack.push((node.child(i).unwrap(), new_container.clone()));
                }
            }
            Some(None) => {
                for i in 0..node.child_count() {
                    stack.push((node.child(i).unwrap(), container.clone()));
                }
            }
            None => {}
        }
    }
}

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
