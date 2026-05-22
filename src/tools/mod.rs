// Tool structs and their helper functions are called via the rmcp macro dispatch;
// the dead_code lint doesn't trace through proc-macro generated code.
#![allow(dead_code)]

pub mod ast_query;
pub mod find_references;
pub mod go_to_definition;
pub mod index_status;
pub mod list_files;
pub mod read_file;
pub mod search_symbols;
pub mod search_text;

pub use ast_query::AstQueryTool;
pub use find_references::FindReferencesTool;
pub use go_to_definition::GoToDefinitionTool;
pub use index_status::IndexStatusTool;
pub use list_files::ListFilesTool;
pub use read_file::ReadFileTool;
pub use search_symbols::SearchSymbolsTool;
pub use search_text::SearchTextTool;
