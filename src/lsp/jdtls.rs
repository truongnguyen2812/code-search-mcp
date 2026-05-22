//! Eclipse JDT Language Server client for Java.
use super::LspClient;
use anyhow::Result;
use std::path::Path;

/// Spawn a jdtls (Eclipse JDT LS) server.
/// Requires `jdtls` to be on PATH or configured via JDTLS_PATH env var.
pub fn spawn(root: &Path) -> Result<LspClient> {
    let jdtls_bin = std::env::var("JDTLS_PATH").unwrap_or_else(|_| "jdtls".to_string());
    let data_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".code-search-mcp")
        .join("jdtls-data");
    std::fs::create_dir_all(&data_dir)?;
    LspClient::spawn(
        &jdtls_bin,
        &["-data", data_dir.to_str().unwrap_or("/tmp/jdtls-data")],
        &root.to_path_buf(),
    )
}
