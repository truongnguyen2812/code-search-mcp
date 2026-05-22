//! XML language server client.
use anyhow::Result;
use std::path::Path;

use super::LspClient;

/// Spawn an XML language server process.
/// Requires `lemminx` on PATH or `XML_LS_PATH` env var.
pub fn spawn(root: &Path) -> Result<LspClient> {
    let bin = std::env::var("XML_LS_PATH").unwrap_or_else(|_| "lemminx".to_string());
    LspClient::spawn(&bin, &[], &root.to_path_buf())
}
