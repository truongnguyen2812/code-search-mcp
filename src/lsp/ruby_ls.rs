//! Ruby language server client.
use anyhow::Result;
use std::path::Path;

use super::LspClient;

/// Spawn a Ruby language server process.
/// Requires `ruby-lsp` on PATH or `RUBY_LS_PATH` env var.
pub fn spawn(root: &Path) -> Result<LspClient> {
    let bin = std::env::var("RUBY_LS_PATH").unwrap_or_else(|_| "ruby-lsp".to_string());
    LspClient::spawn(&bin, &[], &root.to_path_buf())
}
