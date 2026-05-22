# universal-search-mcp Copilot Instructions

## Build & Run

```bash
cargo build                        # debug build
cargo build --release              # optimised binary
cargo run -- --local /path/to/aosp # index + serve (stdio transport)
cargo run -- --local /path/to/aosp --transport http --port 3000  # HTTP/SSE
cargo test                         # run all tests
cargo test <test_name>             # run a single test
```

Key CLI flags (all also accept env vars):
- `--local` / `SEARCH_LOCAL_PATH` — path to source checkout to index
- `--db` / `SEARCH_DB_PATH` — SQLite file (default `~/.universal-search-mcp/index.db`)
- `--index-threads` / `SEARCH_INDEX_THREADS` — parallelism (default: number of logical CPUs)
- `--transport` / `SEARCH_TRANSPORT` — `stdio` (default) or `http`
- `--port` / `SEARCH_HTTP_PORT` — HTTP port (default 3000)
- `--no-watch` — disable the incremental file watcher

## Architecture

```
main.rs          CLI parsing, DB init, spawns indexer + watcher tasks, calls server::run()
server.rs        rmcp ServerHandler — registers all 7 MCP tools, routes stdio / HTTP/SSE
db/              r2d2 SQLite pool; schema.rs runs CREATE TABLE IF NOT EXISTS migrations
indexer/         tree-sitter symbol extraction (java.rs, kotlin.rs, cpp.rs); mod.rs walks
                 the source tree, hashes files to skip unchanged ones, stores symbols + lines
watcher.rs       notify-debouncer watches --local path; calls indexer::index_file on changes
tools/           One file per MCP tool; each tool struct holds a DbPool clone
lsp/             LspClient (JSON-RPC over stdio) + LspManager; jdtls/kotlin_ls/clangd spawners
                 — defined but NOT yet wired into any tool call
remote/          Stub for repo-sync of remote source — download_aosp() is unimplemented!()
```

### Database schema (SQLite, WAL mode)

| Table | Purpose |
|---|---|
| `files` | One row per indexed source file; `path`, `language`, `mtime`, `hash` |
| `symbols` | Extracted symbols; FK → `files`; `name`, `kind`, `container`, `signature`, position |
| `symbols_fts` | FTS5 content table over `symbols(name, container, kind)` — kept in sync by triggers |
| `file_lines` | Every source line stored for full-text search |
| `file_lines_fts` | FTS5 content table over `file_lines` — **created in schema but currently unused** |
| `index_progress` | Key/value progress counters (`status`, `total_files`, `indexed_files`) |

### MCP tools

| Tool | Implementation strategy |
|---|---|
| `search_symbols` | FTS5 prefix search; falls back to `LIKE` |
| `go_to_definition` | Direct `symbols` table query, ordered by kind priority |
| `find_references` | `LIKE '%symbol%'` on `file_lines` — heuristic, no LSP yet |
| `search_text` | `LIKE` for plain text; Rust-side regex filter for regex mode |
| `list_files` | `files` table with optional `LIKE` path filter |
| `ast_query` | Reads file from disk, runs tree-sitter S-expression query inline |
| `index_status` | Reads `index_progress` + aggregate COUNTs |

## Key Conventions

- **Tool structs** live in `src/tools/<name>.rs`, hold a `DbPool` clone, expose one public method (`search`, `find`, `list`, `query`, or `get_status`). Add the tool to `server.rs` `#[rmcp::tool(tool_box)]` impl block.
- **Tool inputs** are `serde::Deserialize + schemars::JsonSchema` structs; use `#[tool(aggr)]` in the server impl to aggregate them.
- **Tool results** always return `Result<CallToolResult, rmcp::Error>`; on error wrap with `CallToolResult::error(vec![Content::text(...)])` — never propagate errors directly.
- **Indexer extractors** (`java.rs`, `kotlin.rs`, `cpp.rs`) use tree-sitter `visit_node` recursion. `container` is built as a dot-separated path (e.g. `OuterClass.InnerClass`).
- Files > 2 MB and non-UTF-8 files are silently skipped during indexing.
- source `out/` and `.repo/` directories are excluded from the walk in `should_skip()`.
- Logs go to **stderr** (keeps stdout clean for MCP stdio transport). Set `RUST_LOG=debug` for verbose output.
- The SQLite pool is configured with WAL journal, 64 MB cache, `NORMAL` synchronous, and a max of 8 connections.

## LSP Server Dependencies

The following LSP servers enable precise go-to-definition and find-references beyond the SQLite index fallback. If a server is missing, the tool logs a WARN and falls back gracefully.

### Installation

```bash
# C/C++ (clangd)
sudo apt install -y clangd

# JSON & YAML (npm-based)
npm install -g vscode-langservers-extracted yaml-language-server

# Go (gopls) — requires Go toolchain
go install golang.org/x/tools/gopls@latest

# Makefile
pip3 install --user make-language-server

# Java (jdtls) — Eclipse JDT Language Server
mkdir -p ~/.local/share/jdtls && cd ~/.local/share/jdtls && \
curl -L https://download.eclipse.org/jdtls/milestones/1.38.0/jdt-language-server-1.38.0-202408011337.tar.gz | tar xz && \
ln -sf ~/.local/share/jdtls/bin/jdtls ~/.local/bin/jdtls

# Kotlin
cd /tmp && \
curl -L https://github.com/fwcd/kotlin-language-server/releases/latest/download/server.zip -o kls.zip && \
unzip -o kls.zip -d ~/.local/share/kotlin-language-server && \
ln -sf ~/.local/share/kotlin-language-server/server/bin/kotlin-language-server ~/.local/bin/kotlin-language-server

# XML (lemminx)
curl -L https://github.com/eclipse/lemminx/releases/latest/download/lemminx-linux.zip -o /tmp/lemminx.zip && \
unzip -o /tmp/lemminx.zip -d ~/.local/bin/

# Ensure PATH includes local bins
export PATH="$HOME/.local/bin:$HOME/go/bin:$PATH"
```

### Env var overrides

Each LSP binary can be overridden via environment variable:

| Language | Env Var | Default Binary |
|----------|---------|----------------|
| Java | `JDTLS_PATH` | `jdtls` |
| Kotlin | `KOTLIN_LS_PATH` | `kotlin-language-server` |
| C/C++ | *(none)* | `clangd` |
| Go | `GOPLS_PATH` | `gopls` |
| Groovy | `GROOVY_LS_PATH` | `groovy-language-server` |
| Ruby | `RUBY_LS_PATH` | `ruby-lsp` |
| JSON | `JSON_LS_PATH` | `vscode-json-language-server` |
| XML | `XML_LS_PATH` | `lemminx` |
| YAML | `YAML_LS_PATH` | `yaml-language-server` |
| AIDL | `AIDL_LS_PATH` | `aidl-language-server` |
| Makefile | `MAKE_LS_PATH` | `make-language-server` |

## Known Incomplete Areas

- **LSP integration**: `src/lsp/` is fully written but `LspManager` is never instantiated — `find_references` and `go_to_definition` fall back only to the SQLite index.
- **`file_lines_fts`**: the FTS5 table is created but `search_text` uses `LIKE` instead of it.
- **`indexed_files` progress counter**: set to `"0"` at start of indexing, never incremented.
- **Remote download**: `remote::download_aosp()` calls `unimplemented!()`.
