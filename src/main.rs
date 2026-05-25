mod db;
mod indexer;
mod lsp;
mod server;
mod tools;
mod watcher;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "code-search-mcp",
    about = "MCP server for searching codebases via tree-sitter + LSP",
    version
)]
pub struct Cli {
    /// Path to the local source checkout to index and search
    #[arg(long, env = "SEARCH_LOCAL_PATH")]
    pub local: Option<PathBuf>,

    /// Transport mode: stdio (for Claude Desktop/CLI) or http (Streamable HTTP)
    #[arg(long, default_value = "stdio", env = "SEARCH_TRANSPORT")]
    pub transport: TransportMode,

    /// HTTP port (only used when --transport=http)
    #[arg(long, default_value = "3000", env = "SEARCH_HTTP_PORT")]
    pub port: u16,

    /// Path to the SQLite index database
    #[arg(long, env = "SEARCH_DB_PATH")]
    pub db: Option<PathBuf>,

    /// Number of parallel indexing threads (default: number of logical CPUs)
    #[arg(long, env = "SEARCH_INDEX_THREADS")]
    pub index_threads: Option<usize>,

    /// Disable the incremental file watcher
    #[arg(long, default_value = "false")]
    pub no_watch: bool,

    /// Skip the background FTS trigram rebuild (FTS index is required for search tools)
    #[arg(long, default_value = "false")]
    pub no_fts_rebuild: bool,

    /// Allow remote connections / any Host header (disables host validation)
    #[arg(long, default_value = "false", env = "SEARCH_ALLOW_REMOTE")]
    pub allow_remote: bool,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum TransportMode {
    Stdio,
    Http,
}

impl std::fmt::Display for TransportMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportMode::Stdio => write!(f, "stdio"),
            TransportMode::Http => write!(f, "http"),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialise tracing — for stdio mode, write logs to stderr so stdout stays clean
    let subscriber = fmt::Subscriber::builder()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("code_search_mcp=info".parse()?),
        )
        .with_writer(std::io::stderr)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("code-search-mcp starting up");

    // Resolve DB path
    let db_path = cli.db.clone().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".code-search-mcp")
            .join("index.db")
    });

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Initialise database
    let pool = db::create_pool(&db_path)?;
    db::schema::run_migrations(&pool)?;
    info!("Database ready at {}", db_path.display());

    let no_fts_rebuild = cli.no_fts_rebuild;

    // Start indexer if a local path is provided
    if let Some(ref local_path) = cli.local {
        info!("Starting indexer for {}", local_path.display());
        let pool_clone = pool.clone();
        let local_path_clone = local_path.clone();
        let threads = cli.index_threads.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        });
        tokio::spawn(async move {
            if let Err(e) = indexer::index_codebase(&pool_clone, &local_path_clone, threads).await {
                tracing::error!("Indexer error: {e}");
            } else {
                tracing::info!("Indexing complete. Running post-index optimizations...");
                // Run ANALYZE for query planner
                if let Err(e) = db::analyze(&pool_clone) {
                    tracing::warn!("ANALYZE failed: {e}");
                }
            }

            // FTS rebuild runs AFTER indexer finishes to avoid write lock contention
            if !no_fts_rebuild {
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = db::schema::rebuild_fts_background(&pool_clone) {
                        tracing::error!("FTS rebuild error: {e}");
                    }
                })
                .await
                .ok();
            }
        });

        // Start file watcher for incremental updates
        if !cli.no_watch {
            let pool_clone2 = pool.clone();
            let local_path_clone2 = local_path.clone();
            tokio::spawn(async move {
                if let Err(e) = watcher::start_watcher(pool_clone2, local_path_clone2).await {
                    tracing::error!("Watcher error: {e}");
                }
            });
        }
    } else {
        // No indexer — run FTS rebuild immediately in background (no contention)
        if !no_fts_rebuild {
            let pool_fts = pool.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = db::schema::rebuild_fts_background(&pool_fts) {
                    tracing::error!("FTS rebuild error: {e}");
                }
            });
        } else {
            info!("FTS rebuild skipped (--no-fts-rebuild). Search tools will be unavailable or return empty results until FTS is rebuilt.");
        }
        info!("No --local path provided; running in query-only mode");
    }

    // Start MCP server
    server::run(cli, pool).await?;

    Ok(())
}
