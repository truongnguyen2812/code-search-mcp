# code-search-mcp

An [MCP (Model Context Protocol)](https://modelcontextprotocol.io) server that indexes and searches codebases using [tree-sitter](https://tree-sitter.github.io/tree-sitter/) for symbol extraction and SQLite for fast querying. Symbol extraction is strongest for Java/Kotlin/C/C++, and the index now also includes Go, Groovy/Gradle, Ruby, JSON, XML, YAML, AIDL, Makefiles, and other text source formats for language-aware search and LSP-driven navigation.

## Features

- **Symbol search** — find classes, methods, functions, fields by name or glob pattern using FTS5 full-text indexing
- **Go to definition** — locate where a symbol is defined with file, line, signature, and optionally enhanced via LSP
- **Find references** — find all usages of a symbol across the codebase; uses LSP for precision when available, falls back to text index
- **Full-text search** — fast FTS5-powered text search across all indexed source lines with regex support
- **AST queries** — run tree-sitter S-expression pattern queries against any source file
- **File listing** — browse indexed files filtered by path or language
- **Indexing status** — live progress and per-language/kind breakdown
- **Incremental updates** — file watcher re-indexes changed files automatically
- **Dual transport** — stdio (for Claude Desktop / Claude CLI) or Streamable HTTP MCP

## Database documentation

- See docs/database.md for schema details, progress semantics, and troubleshooting for row-limit confusion (for example seeing only 1000 C files in a viewer).

---

## Installation

### Prerequisites

- **Rust 1.85+** (edition 2024): https://rustup.rs
- **A local source checkout** (see [Downloading the Source](https://source.android.com/docs/setup/download/downloading))

### Build from source

```bash
git clone https://github.com/your-org/code-search-mcp
cd code-search-mcp
cargo build --release
```

The binary is placed at `target/release/code-search-mcp`.

---

## Quick Start

```bash
# Index a local source checkout and start the MCP server on stdio
./target/release/code-search-mcp --local /path/to/aosp

# Use a custom database location
./target/release/code-search-mcp --local /path/to/aosp --db /var/data/aosp.db

# Start over Streamable HTTP MCP transport
./target/release/code-search-mcp --local /path/to/aosp --transport http --port 3000

# Query-only mode (no indexing, use an existing database)
./target/release/code-search-mcp --db /var/data/aosp.db
```

Indexing runs in the background. You can start querying immediately; results improve as more files are indexed. Use the `index_status` tool to monitor progress.

---

## CLI Reference

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--local <PATH>` | `SEARCH_LOCAL_PATH` | — | Path to the source checkout to index and watch |
| `--db <PATH>` | `SEARCH_DB_PATH` | `~/.code-search-mcp/index.db` | SQLite database file |
| `--transport <MODE>` | `SEARCH_TRANSPORT` | `stdio` | Transport: `stdio` or `http` |
| `--port <PORT>` | `SEARCH_HTTP_PORT` | `3000` | HTTP port (MCP endpoint: `/mcp`; only used with `--transport http`) |
| `--index-threads <N>` | `SEARCH_INDEX_THREADS` | `4` | Parallel indexing threads |
| `--no-watch` | — | `false` | Disable incremental file watcher |

Enable verbose logging with `RUST_LOG=debug`.

---

## Integrating with Claude Desktop

Add the following to your `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "code-search": {
      "command": "/path/to/code-search-mcp",
      "args": ["--local", "/path/to/aosp"],
      "env": {
        "SEARCH_INDEX_THREADS": "8"
      }
    }
  }
}
```

Config file locations:
- **macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Linux**: `~/.config/Claude/claude_desktop_config.json`
- **Windows**: `%APPDATA%\Claude\claude_desktop_config.json`

---

## Integrating with Claude CLI (Copilot)

```bash
# Start the server in Streamable HTTP mode, then add it
./code-search-mcp --local /path/to/aosp --transport http --port 3000 &
gh copilot mcp add code-search http://localhost:3000/mcp
```

Or configure it as a stdio server in your `~/.copilot/mcp.json`.

---

## Using `code-search` MCP In VS Code

If you use VS Code MCP config (`.vscode/mcp.json`), this project can be connected as an HTTP MCP server named `code-search`.

### 1) Start the server

From the project root:

```bash
cargo run -- --local /path/to/source --transport http --port 3000
```

Keep this process running. The MCP endpoint is:

```text
http://localhost:3000/mcp
```

### 2) Configure `.vscode/mcp.json`

Use:

```json
{
  "servers": {
    "code-search": {
      "url": "http://localhost:3000/mcp",
      "type": "http"
    }
  },
  "inputs": []
}
```

### 3) Validate the connection

Call `index_status` first. A healthy response includes fields such as `status`, `total_files`, and `total_symbols`.

Then try:

- `list_files` to confirm files are indexed
- `search_symbols` to find classes/methods quickly
- `go_to_definition` and `find_references` for navigation
- `search_text` for grep-style lookup
- `ast_query` for structural queries in a specific file

### Common connectivity checks

- Confirm the server process is still running.
- Confirm the URL includes `/mcp` (not just host + port).
- Confirm port `3000` in `.vscode/mcp.json` matches the server `--port` value.
- If the port is in use, pick another one and update both the run command and `.vscode/mcp.json`.

---

## MCP Tools

### `search_symbols`

Search for symbols (classes, methods, functions, fields) by name or glob pattern.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `query` | string | ✅ | Symbol name or glob (`Activity`, `on*`, `Binder*`) |
| `language` | string | — | Filter by indexed language key (for example: `java`, `kotlin`, `c`, `cpp`, `go`, `groovy`, `gradle`, `rb`, `json`, `xml`, `yaml`, `aidl`, `make`) |
| `kind` | string | — | Filter: `class`, `method`, `function`, `field`, `property`, `struct`, `enum`, `interface` |
| `limit` | number | — | Max results (default: 50, max: 500) |

**Example:**
```json
{ "query": "ActivityManager", "language": "java", "kind": "class" }
```

**Response:**
```json
[
  {
    "name": "ActivityManager",
    "kind": "class",
    "container": "android.app",
    "file": "frameworks/base/core/java/android/app/ActivityManager.java",
    "language": "java",
    "start_line": 142,
    "start_col": 0,
    "signature": ""
  }
]
```

---

### `go_to_definition`

Find where a symbol is defined. Uses the LSP server for precision when available.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `symbol` | string | ✅ | Exact symbol name |
| `language` | string | — | Narrow by language |
| `container` | string | — | Narrow by container/class name |

**Example:**
```json
{ "symbol": "startActivity", "language": "java" }
```

---

### `find_references`

Find all usages of a symbol. LSP-backed when a language server is configured; falls back to text index search.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `symbol` | string | ✅ | Symbol name to find usages of |
| `language` | string | — | Restrict to a language |
| `limit` | number | — | Max results (default: 100, max: 500) |

**Example:**
```json
{ "symbol": "Binder", "language": "java", "limit": 50 }
```

**Response:**
```json
[
  {
    "file": "frameworks/base/core/java/android/os/ServiceManager.java",
    "line_no": 87,
    "content": "    IBinder binder = ServiceManager.getService(\"activity\");",
    "language": "java"
  }
]
```

---

### `search_text`

Full-text search across all indexed source lines. Uses FTS5 for fast matching; supports regex.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `query` | string | ✅ | Text or regex pattern |
| `regex` | boolean | — | Treat `query` as a regex (default: `false`) |
| `language` | string | — | Filter by language |
| `path_filter` | string | — | Filter to files matching this path substring |
| `limit` | number | — | Max results (default: 100, max: 1000) |
| `context_lines` | number | — | Lines of context around each match (default: 2, max: 10) |

**Examples:**
```json
{ "query": "PackageManager", "path_filter": "frameworks/base", "limit": 20 }
{ "query": "void on\\w+\\(", "regex": true, "language": "java" }
```

---

### `ast_query`

Execute a tree-sitter S-expression pattern query against a specific source file. Useful for structural code search beyond simple text matching.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `file_path` | string | ✅ | Absolute path to the source file |
| `pattern` | string | ✅ | tree-sitter S-expression query |

**Example — find all method declarations in a Java file:**
```json
{
  "file_path": "/aosp/frameworks/base/core/java/android/app/Activity.java",
  "pattern": "(method_declaration name: (identifier) @method.name)"
}
```

**Response:**
```json
[
  {
    "capture_name": "method.name",
    "text": "onCreate",
    "start_line": 989,
    "start_col": 17,
    "end_line": 989,
    "end_col": 25
  }
]
```

See the [tree-sitter query syntax documentation](https://tree-sitter.github.io/tree-sitter/using-parsers#pattern-matching-with-queries) for pattern reference.

### How Symbol Search Works (With AST Query Example)

Use this mental model:

1. `search_symbols` is index-first symbol lookup.
2. `ast_query` is file-local structural pattern matching.
3. They are complementary: find candidate symbol/file quickly, then run structural checks in that file.

#### `search_symbols` flow

1. Query `symbols_fts` first for fast symbol-name lookup.
2. Join to `symbols` and `files` to return symbol metadata (kind, container, file, position).
3. If FTS gives no result (or fails to parse), fallback to `LIKE` on `symbols.name`.
4. Optional filters (`language`, `kind`) are applied in SQL.

#### `ast_query` flow

1. Read the target file from disk (not from SQLite symbol tables).
2. Parse that file with tree-sitter for the file language.
3. Execute your S-expression query pattern.
4. Return exact capture text + positions for each AST match.

#### Practical example

Step 1: Find candidate class files by symbol name.

```json
{ "query": "ActivityManager", "language": "java", "kind": "class" }
```

Step 2: In one returned file, find all method declarations structurally.

```json
{
  "file_path": "/aosp/frameworks/base/core/java/android/app/ActivityManager.java",
  "pattern": "(method_declaration name: (identifier) @method.name)"
}
```

Why this pattern helps:

- `search_symbols` is fast for discovery across the whole codebase.
- `ast_query` is precise for shape-based matching inside a single file.
- Combining both avoids broad text grep and reduces false positives.

---

### `list_files`

List indexed source files, optionally filtered by path or language.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `path_filter` | string | — | Path substring or glob (e.g. `frameworks/base`, `*.java`) |
| `language` | string | — | Filter by language |
| `limit` | number | — | Max results (default: 200, max: 2000) |

**Example:**
```json
{ "path_filter": "frameworks/base/services", "language": "java" }
```

---

### `index_status`

Get the current indexing status and statistics.

**Example response:**
```json
{
  "status": "indexing",
  "total_files": 142500,
  "indexed_files": 38200,
  "total_symbols": 1850000,
  "files_by_language": { "java": 22100, "cpp": 11400, "c": 3800, "kotlin": 900 },
  "symbols_by_kind": { "method": 980000, "class": 120000, "function": 450000 },
  "errors": 0
}
```

Status values: `idle` → `indexing` → `done`.

---

## How Search And Indexing Work By Tool

This section explains which indexed tables each tool reads, and where LSP or direct file parsing is used.

### Shared indexing pipeline (used by all query tools)

1. File discovery walks the `--local` tree and keeps only recognized source/text extensions.
2. Each file is hashed; unchanged files are skipped.
3. `files` row is upserted (`path`, `language`, `mtime`, `hash`).
4. Language extractor builds symbols and writes `symbols` rows.
5. File content is split into lines and stored in `file_lines`.
6. FTS trigger tables (`symbols_fts`, `file_lines_fts`) are auto-maintained by SQLite triggers.

### `search_symbols`

- Primary source: `symbols_fts` (FTS prefix query) joined with `symbols` + `files`.
- Fallback: `LIKE` search on `symbols.name` when FTS is empty or query parsing fails.
- Best results for languages with stronger extractors (Java/Kotlin/C/C++ and migrated AST languages).

### `go_to_definition`

- First pass: `symbols` table exact symbol-name lookup (plus optional language/container filters).
- Optional precision step: LSP `textDocument/definition` from candidate source position.
- If no symbol-row candidate exists, it bootstraps from a matching `file_lines` anchor and tries LSP.

### `find_references`

- First pass: tries LSP `textDocument/references` from indexed definition position.
- If no definition symbol exists, uses a `file_lines` anchor to bootstrap LSP.
- Fallback: heuristic `LIKE` search over `file_lines.content`.

### `search_text`

- Reads indexed `file_lines` joined with `files`.
- Supports plain substring and optional regex filtering in Rust.
- Returns matching lines with optional context window.

### `ast_query`

- Does not use SQLite index tables.
- Reads the target file from disk and runs a tree-sitter query pattern directly.
- Only supported for languages with configured tree-sitter AST query handling.

### `list_files`

- Reads only the `files` table.
- Optional language/path filters and result limit.

### `read_file`

- Does not use SQLite index tables.
- Reads the entire target file from disk and returns its string contents.

### `index_status`

- Reads progress counters from `index_progress`.
- Reads aggregate counts from `files` and `symbols`.
- Includes language and symbol-kind breakdown from grouped SQL queries.

---

## LSP Integration (Optional)

For precise go-to-definition and find-references, install language servers:

| Language | Server | Installation |
|---|---|---|
| Java | [Eclipse JDT LS](https://github.com/eclipse-jdtls/eclipse.jdt.ls) | `brew install jdtls` or download from releases |
| Kotlin | [kotlin-language-server](https://github.com/fwcd/kotlin-language-server) | Download from releases |
| C/C++ | [clangd](https://clangd.llvm.org/) | `apt install clangd` / `brew install llvm` |

The server will automatically try to spawn the appropriate LSP when `--local` is provided, and gracefully fall back to the SQLite index if a language server is unavailable.

Set the `JDTLS_PATH` environment variable to override the default `jdtls` binary path.

---

## Architecture

```
main.rs          CLI parsing, DB init, spawns indexer + watcher tasks, calls server::run()
server.rs        rmcp ServerHandler — registers all 7 MCP tools; serves stdio or Streamable HTTP (`/mcp`)
db/              r2d2 SQLite pool; schema.rs creates tables + FTS5 virtual tables + triggers
indexer/         tree-sitter symbol extraction (java.rs, kotlin.rs, cpp.rs); mod.rs walks
                 the source tree, hashes files to skip unchanged ones, stores symbols + lines
watcher.rs       notify-debouncer watches --local path; calls indexer::index_file on changes
tools/           One file per MCP tool; each tool struct holds a DbPool and optional LspManager
lsp/             LspClient (JSON-RPC over stdio) + LspManager (lazy per-language spawning)
                 jdtls.rs / kotlin_ls.rs / clangd.rs — spawner helpers
```

### Database Schema

| Table | Purpose |
|---|---|
| `files` | One row per indexed source file: `path`, `language`, `mtime`, `hash` |
| `symbols` | Extracted symbols; FK → `files`; `name`, `kind`, `container`, `signature`, position |
| `symbols_fts` | FTS5 virtual table over `symbols(name, container, kind)` — kept in sync by triggers |
| `file_lines` | Every source line stored for full-text search |
| `file_lines_fts` | FTS5 virtual table over `file_lines(content)` — kept in sync by triggers |
| `index_progress` | Key/value progress counters (`status`, `total_files`, `indexed_files`) |

---

## Development

```bash
cargo build            # debug build
cargo build --release  # optimised binary
cargo test             # run tests
RUST_LOG=debug cargo run -- --local /path/to/aosp
```

Files > 2 MB and non-UTF-8 files are silently skipped. The `out/` and `.repo/` directories are excluded from indexing.

---

## License

MIT
