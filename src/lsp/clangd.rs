//! clangd client for C/C++ advanced queries.
use anyhow::Result;
use std::path::Path;
use super::LspClient;

/// Spawn a clangd LSP server.
pub fn spawn(root: &Path) -> Result<LspClient> {
    LspClient::spawn("clangd", &["--background-index", "--clang-tidy=false"], &root.to_path_buf())
}
