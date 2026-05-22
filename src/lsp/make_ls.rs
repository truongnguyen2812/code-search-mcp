//! Makefile language server client.
use anyhow::Result;
use std::path::Path;

use super::LspClient;

/// Spawn a Make language server process.
/// Requires `make-language-server` on PATH or `MAKE_LS_PATH` env var.
pub fn spawn(root: &Path) -> Result<LspClient> {
    let bin = std::env::var("MAKE_LS_PATH")
        .unwrap_or_else(|_| "make-language-server".to_string());
    LspClient::spawn(&bin, &["--stdio"], &root.to_path_buf())
}
