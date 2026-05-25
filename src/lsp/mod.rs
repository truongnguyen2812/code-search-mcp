//! LSP process manager — spawns and manages language server child processes.
//! Used as a fallback for cross-file queries not covered by the tree-sitter index.

#![allow(dead_code)]

pub mod aidl_ls;
pub mod clangd;
pub mod go_ls;
pub mod groovy_ls;
pub mod jdtls;
pub mod json_ls;
pub mod kotlin_ls;
pub mod make_ls;
pub mod ruby_ls;
pub mod xml_ls;
pub mod yaml_ls;

use anyhow::Result;
use lsp_types::{InitializeParams, InitializeResult, WorkspaceFolder};
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::Arc;
use tracing::warn;

/// A connected LSP server process.
pub struct LspClient {
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    request_id: i64,
    child: Child,
}

impl LspClient {
    /// Check if the LSP server process is still running.
    pub fn is_alive(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => true,
            _ => false,
        }
    }

    /// Shutdown the LSP server gracefully or force-kill if needed.
    pub fn shutdown(&mut self) -> Result<()> {
        let _ = self.notify("shutdown", serde_json::json!({}));
        let _ = self.notify("exit", serde_json::json!({}));
        let _ = self.child.kill();
        Ok(())
    }

    /// Spawn a new LSP server process.
    pub fn spawn(program: &str, args: &[&str], root: &PathBuf) -> Result<Self> {
        let mut child = std::process::Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = BufReader::new(stdout);

        let mut client = LspClient {
            stdin,
            reader,
            request_id: 0,
            child,
        };

        // Send initialize
        let root_uri: lsp_types::Uri = url::Url::from_file_path(root)
            .map_err(|_| anyhow::anyhow!("Invalid root path"))?
            .as_str()
            .parse()
            .map_err(|e| anyhow::anyhow!("URI parse error: {e}"))?;

        let init_params = InitializeParams {
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_uri,
                name: "root".to_string(),
            }]),
            ..Default::default()
        };

        let _: InitializeResult = client.request("initialize", init_params)?;

        // Send initialized notification
        client.notify("initialized", serde_json::json!({}))?;

        Ok(client)
    }

    /// Send a JSON-RPC request and wait for the response.
    pub fn request<P: serde::Serialize, R: serde::de::DeserializeOwned>(
        &mut self,
        method: &str,
        params: P,
    ) -> Result<R> {
        self.request_id += 1;
        let id = self.request_id;

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        self.send_message(&msg)?;

        // Read responses until we get ours
        loop {
            let response = self.read_message()?;
            if let Some(resp_id) = response.get("id") {
                if resp_id.as_i64() == Some(id) {
                    if let Some(result) = response.get("result") {
                        return Ok(serde_json::from_value(result.clone())?);
                    } else if let Some(err) = response.get("error") {
                        return Err(anyhow::anyhow!("LSP error: {err}"));
                    }
                }
            }
        }
    }

    /// Send a JSON-RPC notification (no response expected).
    pub fn notify<P: serde::Serialize>(&mut self, method: &str, params: P) -> Result<()> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.send_message(&msg)
    }

    fn send_message(&mut self, msg: &Value) -> Result<()> {
        let body = serde_json::to_string(msg)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes())?;
        self.stdin.write_all(body.as_bytes())?;
        self.stdin.flush()?;
        Ok(())
    }

    fn read_message(&mut self) -> Result<Value> {
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            self.reader.read_line(&mut line)?;
            let line = line.trim();
            if line.is_empty() {
                break;
            }
            if let Some(rest) = line.strip_prefix("Content-Length: ") {
                content_length = rest.parse()?;
            }
        }

        let mut body = vec![0u8; content_length];
        use std::io::Read;
        self.reader.read_exact(&mut body)?;
        Ok(serde_json::from_slice(&body)?)
    }
}

/// Manages a pool of LSP server instances, one per language.
pub struct LspManager {
    root: PathBuf,
    clients: Mutex<HashMap<String, LspClient>>,
}

impl LspManager {
    pub fn new(root: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            root,
            clients: Mutex::new(HashMap::new()),
        })
    }

    fn server_key_for_language(language: &str) -> Option<&'static str> {
        match language {
            "java" => Some("java"),
            "kotlin" => Some("kotlin"),
            "c" | "cpp" => Some("cpp"),
            "go" => Some("go"),
            "groovy" | "gradle" => Some("groovy"),
            "rb" => Some("ruby"),
            "json" => Some("json"),
            "xml" => Some("xml"),
            "yaml" | "yml" => Some("yaml"),
            "aidl" => Some("aidl"),
            "mk" | "make" => Some("make"),
            _ => None,
        }
    }

    /// Get or spawn an LSP client for the given language.
    /// Returns the server key used to store/retrieve the client.
    pub fn get_or_spawn(&self, language: &str) -> Result<String> {
        let server_key = Self::server_key_for_language(language)
            .ok_or_else(|| anyhow::anyhow!("No LSP for language: {language}"))?;

        let mut clients = self.clients.lock();
        if let Some(client) = clients.get_mut(server_key) {
            if !client.is_alive() {
                warn!("LSP client for {server_key} has exited. Removing and spawning a new one.");
                clients.remove(server_key);
            }
        }
        if clients.contains_key(server_key) {
            return Ok(server_key.to_string());
        }
        let client = match server_key {
            "java" => jdtls::spawn(&self.root),
            "kotlin" => kotlin_ls::spawn(&self.root),
            "cpp" => clangd::spawn(&self.root),
            "go" => go_ls::spawn(&self.root),
            "groovy" => groovy_ls::spawn(&self.root),
            "ruby" => ruby_ls::spawn(&self.root),
            "json" => json_ls::spawn(&self.root),
            "xml" => xml_ls::spawn(&self.root),
            "yaml" => yaml_ls::spawn(&self.root),
            "aidl" => aidl_ls::spawn(&self.root),
            "make" => make_ls::spawn(&self.root),
            _ => return Err(anyhow::anyhow!("No LSP for language: {language}")),
        };
        match client {
            Ok(c) => {
                clients.insert(server_key.to_string(), c);
                Ok(server_key.to_string())
            }
            Err(e) => {
                warn!("Could not spawn LSP for {server_key}: {e}");
                Err(e)
            }
        }
    }

    /// Ask the LSP server for go-to-definition at the given file position.
    /// Returns a list of (file_path, line, col) locations.
    pub fn goto_definition(
        &self,
        file_path: &std::path::Path,
        line: u32,
        col: u32,
    ) -> Result<Vec<(String, u32, u32)>> {
        let language = crate::indexer::Language::from_path(file_path)
            .ok_or_else(|| anyhow::anyhow!("Unknown file language"))?;

        // Ensure client is spawned
        let server_key = self.get_or_spawn(language.as_str())?;

        let file_uri = file_path_to_uri(file_path)?;

        let params = serde_json::json!({
            "textDocument": { "uri": file_uri },
            "position": { "line": line, "character": col },
        });

        let mut clients = self.clients.lock();
        let client = clients
            .get_mut(server_key.as_str())
            .ok_or_else(|| anyhow::anyhow!("LSP client unavailable"))?;

        let response: serde_json::Value = client.request("textDocument/definition", params)?;

        Ok(parse_lsp_locations(&response))
    }

    /// Ask the LSP server for all references to the symbol at the given file position.
    /// Returns a list of (file_path, line, col) locations.
    pub fn find_references(
        &self,
        file_path: &std::path::Path,
        line: u32,
        col: u32,
    ) -> Result<Vec<(String, u32, u32)>> {
        let language = crate::indexer::Language::from_path(file_path)
            .ok_or_else(|| anyhow::anyhow!("Unknown file language"))?;

        let server_key = self.get_or_spawn(language.as_str())?;

        let file_uri = file_path_to_uri(file_path)?;

        let params = serde_json::json!({
            "textDocument": { "uri": file_uri },
            "position": { "line": line, "character": col },
            "context": { "includeDeclaration": false },
        });

        let mut clients = self.clients.lock();
        let client = clients
            .get_mut(server_key.as_str())
            .ok_or_else(|| anyhow::anyhow!("LSP client unavailable"))?;

        let response: serde_json::Value = client.request("textDocument/references", params)?;

        Ok(parse_lsp_locations(&response))
    }

    /// Gracefully shutdown all active LSP clients.
    pub fn shutdown_all(&self) {
        let mut clients = self.clients.lock();
        for (key, client) in clients.iter_mut() {
            tracing::info!("Shutting down LSP server for {key}");
            let _ = client.shutdown();
        }
        clients.clear();
    }
}

impl Drop for LspManager {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}

/// Convert a file path to a `file://` URI string usable by LSP servers.
fn file_path_to_uri(path: &std::path::Path) -> Result<String> {
    url::Url::from_file_path(path)
        .map(|u| u.to_string())
        .map_err(|_| anyhow::anyhow!("Cannot convert path to URI: {}", path.display()))
}

/// Parse LSP Location / LocationLink / Location[] responses into (path, line, col) tuples.
fn parse_lsp_locations(value: &serde_json::Value) -> Vec<(String, u32, u32)> {
    let mut out = Vec::new();

    let locations: Vec<&serde_json::Value> = if value.is_array() {
        value.as_array().unwrap().iter().collect()
    } else if value.is_object() {
        vec![value]
    } else {
        return out;
    };

    for loc in locations {
        // Standard Location: { uri, range: { start: { line, character } } }
        let uri = loc
            .get("uri")
            .or_else(|| loc.get("targetUri"))
            .and_then(|u| u.as_str());
        let range = loc
            .get("range")
            .or_else(|| loc.get("targetRange"))
            .and_then(|r| r.as_object());

        if let (Some(uri), Some(range)) = (uri, range) {
            let start = range.get("start").and_then(|s| s.as_object());
            if let Some(start) = start {
                let line = start.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let col = start.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                // Convert file:// URI back to path
                if let Ok(url) = url::Url::parse(uri) {
                    if let Ok(path) = url.to_file_path() {
                        out.push((path.to_string_lossy().into_owned(), line, col));
                    }
                }
            }
        }
    }

    out
}
