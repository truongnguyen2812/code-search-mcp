//! Groovy/Gradle language server client.
use anyhow::Result;
use std::path::Path;

use super::LspClient;

/// Spawn a Groovy language server process.
/// Requires `groovy-language-server` on PATH or `GROOVY_LS_PATH` env var.
pub fn spawn(root: &Path) -> Result<LspClient> {
    let bin = std::env::var("GROOVY_LS_PATH")
        .unwrap_or_else(|_| "groovy-language-server".to_string());
    LspClient::spawn(&bin, &[], &root.to_path_buf())
}
