//! JSON language server client.
use anyhow::Result;
use std::path::Path;

use super::LspClient;

/// Spawn a JSON language server process.
/// Requires `vscode-json-language-server` on PATH or `JSON_LS_PATH` env var.
pub fn spawn(root: &Path) -> Result<LspClient> {
    let bin = std::env::var("JSON_LS_PATH")
        .unwrap_or_else(|_| "vscode-json-language-server".to_string());
    LspClient::spawn(&bin, &["--stdio"], &root.to_path_buf())
}
