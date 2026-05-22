//! AIDL language server client.
use anyhow::Result;
use std::path::Path;

use super::LspClient;

/// Spawn an AIDL language server process.
/// Requires `aidl-language-server` on PATH or `AIDL_LS_PATH` env var.
pub fn spawn(root: &Path) -> Result<LspClient> {
    let bin = std::env::var("AIDL_LS_PATH")
        .unwrap_or_else(|_| "aidl-language-server".to_string());
    LspClient::spawn(&bin, &["--stdio"], &root.to_path_buf())
}
