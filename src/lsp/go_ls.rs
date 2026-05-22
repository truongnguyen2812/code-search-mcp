//! Go language server client (gopls).
use anyhow::Result;
use std::path::Path;

use super::LspClient;

/// Spawn a gopls process.
/// Requires `gopls` on PATH or `GOPLS_PATH` env var.
pub fn spawn(root: &Path) -> Result<LspClient> {
    let bin = std::env::var("GOPLS_PATH").unwrap_or_else(|_| "gopls".to_string());
    LspClient::spawn(&bin, &[], &root.to_path_buf())
}
