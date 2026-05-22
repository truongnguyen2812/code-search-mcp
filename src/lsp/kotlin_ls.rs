//! Kotlin Language Server client.
use anyhow::Result;
use std::path::Path;
use super::LspClient;

/// Spawn a kotlin-language-server process.
/// Requires `kotlin-language-server` to be on PATH or configured via KOTLIN_LS_PATH env var.
pub fn spawn(root: &Path) -> Result<LspClient> {
    let bin = std::env::var("KOTLIN_LS_PATH")
        .unwrap_or_else(|_| "kotlin-language-server".to_string());
    LspClient::spawn(&bin, &[], &root.to_path_buf())
}
