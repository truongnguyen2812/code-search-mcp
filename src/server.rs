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
use std::sync::Arc;
use tracing::info;

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
        let tool = SearchSymbolsTool { pool: self.pool.clone() };
        match tool.search(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        }
    }

    /// Find all references to a symbol name in the indexed codebase.
    #[tool(description = "Find all source lines that reference (use) a given symbol name. Returns file path, line number, and surrounding content. Uses LSP for precise cross-file references when available, falls back to text index search.")]
    async fn find_references(&self, Parameters(input): Parameters<FindReferencesInput>) -> Result<CallToolResult, ErrorData> {
        let tool = FindReferencesTool { pool: self.pool.clone(), lsp: self.lsp.clone() };
        match tool.find(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        }
    }

    /// Go to the definition of a symbol — returns file, line, and signature.
    #[tool(description = "Find the definition location of a symbol (class, function, method, etc.). Returns file path, line number, kind, and full signature. Uses LSP for precise cross-file navigation when available. Optionally filter by language or container class.")]
    async fn go_to_definition(&self, Parameters(input): Parameters<GoToDefinitionInput>) -> Result<CallToolResult, ErrorData> {
        let tool = GoToDefinitionTool { pool: self.pool.clone(), lsp: self.lsp.clone() };
        match tool.find(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        }
    }

    /// Full-text or regex search across all indexed source files.
    #[tool(description = "Search for text patterns across all indexed source files. Supports plain substring matching or regex. Returns matching lines with surrounding context. Optionally filter by language or file path.")]
    async fn search_text(&self, Parameters(input): Parameters<SearchTextInput>) -> Result<CallToolResult, ErrorData> {
        let tool = SearchTextTool { pool: self.pool.clone() };
        match tool.search(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        }
    }

    /// List indexed source files, optionally filtered by path or language.
    #[tool(description = "List source files in the index. Filter by path substring (e.g. 'frameworks/base'), glob (e.g. '*.java'), or language (java/kotlin/c/cpp).")]
    async fn list_files(&self, Parameters(input): Parameters<ListFilesInput>) -> Result<CallToolResult, ErrorData> {
        let tool = ListFilesTool { pool: self.pool.clone() };
        match tool.list(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        }
    }

    /// Read the complete content of a file.
    #[tool(description = "Read the complete content of a file from the remote machine.")]
    async fn read_file(&self, Parameters(input): Parameters<ReadFileInput>) -> Result<CallToolResult, ErrorData> {
        let tool = ReadFileTool;
        match tool.read(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        }
    }

    /// Run a tree-sitter S-expression pattern query against a source file.
    #[tool(description = "Execute a tree-sitter S-expression query pattern against a specific source file. Returns all captured nodes with their text, position, and capture name. Useful for AST-level structural code search.")]
    async fn ast_query(&self, Parameters(input): Parameters<AstQueryInput>) -> Result<CallToolResult, ErrorData> {
        let tool = AstQueryTool;
        match tool.query(input) {
            Ok(results) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&results).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        }
    }

    /// Get the current indexing status and statistics.
    #[tool(description = "Get the current status of the symbol index: indexing progress, total files/symbols indexed, breakdown by language and symbol kind.")]
    async fn index_status(&self, Parameters(_input): Parameters<IndexStatusInput>) -> Result<CallToolResult, ErrorData> {
        let tool = IndexStatusTool { pool: self.pool.clone() };
        match tool.get_status() {
            Ok(status) => Ok(CallToolResult::success(vec![
                Content::text(serde_json::to_string_pretty(&status).unwrap_or_default())
            ])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {e}"))])),
        }
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
    info!("Starting MCP server on stdio transport");
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

async fn run_http(server: CodeSearchMcpServer, port: u16, allow_remote: bool) -> Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
        session::local::LocalSessionManager,
    };

    info!("Starting MCP server on Streamable HTTP at http://0.0.0.0:{port}/mcp");

    let ct = tokio_util::sync::CancellationToken::new();

    let mut config = StreamableHttpServerConfig::default()
        .with_cancellation_token(ct.child_token());
    if allow_remote {
        config = config.disable_allowed_hosts();
    }

    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        LocalSessionManager::default().into(),
        config,
    );

    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.ok();
            ct.cancel();
        })
        .await?;

    Ok(())
}
