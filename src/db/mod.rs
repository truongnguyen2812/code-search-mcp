pub mod schema;

use anyhow::Result;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::path::Path;

pub type DbPool = Pool<SqliteConnectionManager>;

pub fn create_pool(path: &Path) -> Result<DbPool> {
    let manager = SqliteConnectionManager::file(path).with_init(|conn| {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             PRAGMA temp_store=MEMORY;
             PRAGMA cache_size=-131072;
             PRAGMA mmap_size=2147483648;
             PRAGMA page_size=8192;
             PRAGMA busy_timeout=300000;
             PRAGMA wal_autocheckpoint=2000;",
        )
    });
    let pool = Pool::builder().max_size(16).build(manager)?;
    Ok(pool)
}
