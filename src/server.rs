//! MCP server: wires up all tools and exposes them over stdio or Streamable HTTP.

use anyhow::Result;
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
    },
    tool, tool_handler, tool_router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;
use axum::extract::ConnectInfo;

tokio::task_local! {
    pub static CLIENT_IP: String;
}

use crate::Cli;
use crate::TransportMode;
use crate::db::DbPool;
use crate::lsp::LspManager;
use crate::tools::{
    AstQueryTool, FindReferencesTool, GoToDefinitionTool, IndexStatusTool, ListFilesTool,
    ReadFileTool, SearchSymbolsTool, SearchTextTool,
};
use crate::tools::ast_query::AstQueryInput;
use crate::tools::find_references::FindReferencesInput;
use crate::tools::go_to_definition::GoToDefinitionInput;
use crate::tools::index_status::IndexStatusInput;
use crate::tools::list_files::ListFilesInput;
use crate::tools::read_file::ReadFileInput;
use crate::tools::search_symbols::SearchSymbolsInput;
use crate::tools::search_text::SearchTextInput;

/// The MCP server handler — holds shared state accessible to all tool calls.
#[derive(Clone)]
#[allow(dead_code)]
pub struct CodeSearchMcpServer {
    pool: DbPool,
    lsp: Option<Arc<LspManager>>,
    tool_router: ToolRouter<CodeSearchMcpServer>,
}

impl CodeSearchMcpServer {
    pub fn new(pool: DbPool, lsp: Option<Arc<LspManager>>) -> Self {
        Self {
            pool,
            lsp,
            tool_router: CodeSearchMcpServer::tool_router(),
        }
    }
}

#[tool_router]
impl CodeSearchMcpServer {
    /// Search for symbols (classes, methods, functions, fields) by name or glob pattern.
    #[tool(description = "Search for symbols in the source codebase by name or glob pattern. Supports wildcards (* and ?). Optionally filter by language (java/kotlin/c/cpp) or kind (class/method/function/field/property/struct/enum/interface).")]
    async fn search_symbols(&self, Parameters(input): Parameters<SearchSymbolsInput>) -> Result<CallToolResult, ErrorData> {
        let start = std::time::Instant::now();
        let query = input.query.clone();
        let client_ip = CLIENT_IP.try_with(|ip| ip.clone()).unwrap_or_else(|_| "stdio".to_string());
        info!("Tool search_symbols called by {} with query '{}'", client_ip, query);
        let tool = SearchSymbolsTool { pool: self.pool.clone() };
        let res = match tool.search(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        };
        info!("Tool search_symbols for '{}' took {:?}", query, start.elapsed());
        res
    }

    /// Find all references to a symbol name in the indexed codebase.
    #[tool(description = "Find all source lines that reference (use) a given symbol name. Returns file path, line number, and surrounding content. Uses LSP for precise cross-file references when available, falls back to text index search.")]
    async fn find_references(&self, Parameters(input): Parameters<FindReferencesInput>) -> Result<CallToolResult, ErrorData> {
        let start = std::time::Instant::now();
        let symbol = input.symbol.clone();
        let client_ip = CLIENT_IP.try_with(|ip| ip.clone()).unwrap_or_else(|_| "stdio".to_string());
        info!("Tool find_references called by {} for symbol '{}'", client_ip, symbol);
        let tool = FindReferencesTool { pool: self.pool.clone(), lsp: self.lsp.clone() };
        let res = match tool.find(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        };
        info!("Tool find_references for '{}' took {:?}", symbol, start.elapsed());
        res
    }

    /// Go to the definition of a symbol — returns file, line, and signature.
    #[tool(description = "Find the definition location of a symbol (class, function, method, etc.). Returns file path, line number, kind, and full signature. Uses LSP for precise cross-file navigation when available. Optionally filter by language or container class.")]
    async fn go_to_definition(&self, Parameters(input): Parameters<GoToDefinitionInput>) -> Result<CallToolResult, ErrorData> {
        let start = std::time::Instant::now();
        let symbol = input.symbol.clone();
        let client_ip = CLIENT_IP.try_with(|ip| ip.clone()).unwrap_or_else(|_| "stdio".to_string());
        info!("Tool go_to_definition called by {} for symbol '{}'", client_ip, symbol);
        let tool = GoToDefinitionTool { pool: self.pool.clone(), lsp: self.lsp.clone() };
        let res = match tool.find(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        };
        info!("Tool go_to_definition for '{}' took {:?}", symbol, start.elapsed());
        res
    }

    /// Full-text or regex search across all indexed source files.
    #[tool(description = "Search for text patterns across all indexed source files. Supports plain substring matching or regex. Returns matching lines with surrounding context. Optionally filter by language or file path.")]
    async fn search_text(&self, Parameters(input): Parameters<SearchTextInput>) -> Result<CallToolResult, ErrorData> {
        let start = std::time::Instant::now();
        let query = input.query.clone();
        let client_ip = CLIENT_IP.try_with(|ip| ip.clone()).unwrap_or_else(|_| "stdio".to_string());
        info!("Tool search_text called by {} with query '{}'", client_ip, query);
        let tool = SearchTextTool { pool: self.pool.clone() };
        let res = match tool.search(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        };
        info!("Tool search_text for '{}' took {:?}", query, start.elapsed());
        res
    }

    /// List indexed source files, optionally filtered by path or language.
    #[tool(description = "List source files in the index. Filter by path substring (e.g. 'frameworks/base'), glob (e.g. '*.java'), or language (java/kotlin/c/cpp).")]
    async fn list_files(&self, Parameters(input): Parameters<ListFilesInput>) -> Result<CallToolResult, ErrorData> {
        let start = std::time::Instant::now();
        let filter = format!("{:?}", input.path_filter);
        let client_ip = CLIENT_IP.try_with(|ip| ip.clone()).unwrap_or_else(|_| "stdio".to_string());
        info!("Tool list_files called by {} with filter {}", client_ip, filter);
        let tool = ListFilesTool { pool: self.pool.clone() };
        let res = match tool.list(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        };
        info!("Tool list_files with filter {} took {:?}", filter, start.elapsed());
        res
    }

    /// Read the complete content of a file.
    #[tool(description = "Read the complete content of a file from the remote machine.")]
    async fn read_file(&self, Parameters(input): Parameters<ReadFileInput>) -> Result<CallToolResult, ErrorData> {
        let start = std::time::Instant::now();
        let path = input.file_path.clone();
        let client_ip = CLIENT_IP.try_with(|ip| ip.clone()).unwrap_or_else(|_| "stdio".to_string());
        info!("Tool read_file called by {} for path '{}'", client_ip, path);
        let tool = ReadFileTool;
        let res = match tool.read(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        };
        info!("Tool read_file for '{}' took {:?}", path, start.elapsed());
        res
    }

    /// Run a tree-sitter S-expression pattern query against a source file.
    #[tool(description = "Execute a tree-sitter S-expression query pattern against a specific source file. Returns all captured nodes with their text, position, and capture name. Useful for AST-level structural code search.")]
    async fn ast_query(&self, Parameters(input): Parameters<AstQueryInput>) -> Result<CallToolResult, ErrorData> {
        let start = std::time::Instant::now();
        let path = input.file_path.clone();
        let client_ip = CLIENT_IP.try_with(|ip| ip.clone()).unwrap_or_else(|_| "stdio".to_string());
        info!("Tool ast_query called by {} for path '{}'", client_ip, path);
        let tool = AstQueryTool;
        let res = match tool.query(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        };
        info!("Tool ast_query for '{}' took {:?}", path, start.elapsed());
        res
    }

    /// Get the current indexing status and statistics.
    #[tool(description = "Get the current status of the symbol index: indexing progress, total files/symbols indexed, breakdown by language and symbol kind.")]
    async fn index_status(&self, Parameters(_input): Parameters<IndexStatusInput>) -> Result<CallToolResult, ErrorData> {
        let start = std::time::Instant::now();
        let client_ip = CLIENT_IP.try_with(|ip| ip.clone()).unwrap_or_else(|_| "stdio".to_string());
        info!("Tool index_status called by {}", client_ip);
        let tool = IndexStatusTool { pool: self.pool.clone() };
        let res = match tool.get_status() {
            Ok(status) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&status).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        };
        info!("Tool index_status took {:?}", start.elapsed());
        res
    }
}

#[tool_handler]
impl ServerHandler for CodeSearchMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_server_info(Implementation::from_build_env())
        .with_protocol_version(ProtocolVersion::V_2024_11_05)
        .with_instructions(
            "source codebase search server. Index Java, Kotlin, C/C++ source files and \
             search symbols, text, and AST patterns. Use search_symbols for symbol lookup, \
             search_text for full-text grep, go_to_definition to navigate, \
             find_references to find usages, ast_query for structural AST search, \
             list_files to browse the file tree, read_file to read file content, and index_status to check indexing progress."
                .to_string(),
        )
    }
}

pub async fn run(cli: Cli, pool: DbPool) -> Result<()> {
    match cli.transport {
        TransportMode::Stdio => info!("Starting MCP server on stdio transport"),
        TransportMode::Http => info!("Starting MCP server on Streamable HTTP at http://0.0.0.0:{}/mcp", cli.port),
    }

    let lsp = cli.local.as_ref().map(|root| LspManager::new(root.clone()));
    let server = CodeSearchMcpServer::new(pool, lsp.clone());

    let res = match cli.transport {
        TransportMode::Stdio => run_stdio(server).await,
        TransportMode::Http => run_http(server, cli.port, cli.allow_remote).await,
    };

    if let Some(lsp_mgr) = lsp {
        lsp_mgr.shutdown_all();
    }

    res
}

async fn run_stdio(server: CodeSearchMcpServer) -> Result<()> {
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

/// Helper to determine the host's local IP address used for routing to the outside network.
fn get_local_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|addr| addr.ip())
}

async fn run_http(server: CodeSearchMcpServer, port: u16, allow_remote: bool) -> Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
        session::local::LocalSessionManager,
    };
    use tower_http::cors::CorsLayer;

    let ct = tokio_util::sync::CancellationToken::new();

    let mut config = StreamableHttpServerConfig::default()
        .with_cancellation_token(ct.child_token());
    if allow_remote {
        config = config.disable_allowed_hosts();
        info!("Allowed hosts validation disabled (allow_remote = true)");
    } else {
        // Automatically allow loopback addresses and the local IP address
        // to prevent connection hangs or 403 Forbidden errors when connecting
        // from client machines on the local network.
        let mut allowed_hosts = vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
            "[::1]".to_string(),
            "::1".to_string(),
            format!("localhost:{port}"),
            format!("127.0.0.1:{port}"),
        ];
        if let Some(local_ip) = get_local_ip() {
            let ip_str = local_ip.to_string();
            info!("Automatically adding local IP {ip_str} to allowed hosts");
            allowed_hosts.push(ip_str.clone());
            allowed_hosts.push(format!("{ip_str}:{port}"));
        }
        info!("Allowed hosts for HTTP transport: {:?}", allowed_hosts);
        config = config.with_allowed_hosts(allowed_hosts);
    }

    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        LocalSessionManager::default().into(),
        config,
    );

    async fn ip_middleware(
        request: axum::extract::Request,
        next: axum::middleware::Next,
    ) -> axum::response::Response {
        let ip = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        CLIENT_IP.scope(ip, next.run(request)).await
    }

    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(axum::middleware::from_fn(ip_middleware))
        .layer(CorsLayer::permissive());

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.ok();
            ct.cancel();
        })
        .await?;

    Ok(())
}
