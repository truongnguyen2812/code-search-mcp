//! YAML language server client.
use anyhow::Result;
use std::path::Path;

use super::LspClient;

/// Spawn a YAML language server process.
/// Requires `yaml-language-server` on PATH or `YAML_LS_PATH` env var.
pub fn spawn(root: &Path) -> Result<LspClient> {
    let bin = std::env::var("YAML_LS_PATH")
        .unwrap_or_else(|_| "yaml-language-server".to_string());
    LspClient::spawn(&bin, &["--stdio"], &root.to_path_buf())
}
