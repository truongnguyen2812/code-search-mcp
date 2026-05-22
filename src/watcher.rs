//! Incremental file watcher using `notify`.
//! Watches the local source path for changes and re-indexes modified files.

use anyhow::Result;
use notify_debouncer_mini::{
    new_debouncer,
    notify::RecursiveMode,
    DebouncedEventKind,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::db::DbPool;
use crate::indexer::{index_file, Language};

pub async fn start_watcher(pool: DbPool, root: PathBuf) -> Result<()> {
    info!("File watcher starting for {}", root.display());

    // Run blocking watcher loop in a dedicated thread
    tokio::task::spawn_blocking(move || watcher_loop(pool, root))
        .await??;

    Ok(())
}

fn watcher_loop(pool: DbPool, root: PathBuf) -> Result<()> {
    let (tx, rx) = mpsc::channel();

    let mut debouncer = new_debouncer(Duration::from_millis(500), tx)?;
    debouncer.watcher().watch(&root, RecursiveMode::Recursive)?;

    info!("Watching {} for changes", root.display());

    for events in &rx {
        match events {
            Ok(events) => {
                for event in events {
                    if event.kind == DebouncedEventKind::Any {
                        handle_path(&pool, &event.path);
                    }
                }
            }
            Err(e) => warn!("Watcher error: {e}"),
        }
    }

    Ok(())
}

fn handle_path(pool: &DbPool, path: &Path) {
    // Only process files we understand
    let lang = match Language::from_path(path) {
        Some(l) => l,
        None => return,
    };

    if !path.is_file() {
        // File was deleted — remove from index
        if let Ok(conn) = pool.get() {
            let _ = conn.execute(
                "DELETE FROM files WHERE path = ?1",
                [path.to_string_lossy().as_ref()],
            );
            debug!("Removed deleted file from index: {}", path.display());
        }
        return;
    }

    debug!("Re-indexing changed file: {}", path.display());
    match index_file(pool, path, &lang) {
        Ok(_) => {}
        Err(e) => warn!("Failed to re-index {}: {e}", path.display()),
    }
}
