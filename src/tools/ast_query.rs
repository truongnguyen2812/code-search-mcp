use anyhow::Result;
use serde::{Deserialize, Serialize};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

use crate::indexer::Language;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AstQueryInput {
    /// Absolute path to the source file to query
    pub file_path: String,
    /// tree-sitter S-expression query pattern
    pub pattern: String,
}

#[derive(Debug, Serialize)]
pub struct AstMatch {
    pub capture_name: String,
    pub text: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

pub struct AstQueryTool;

impl AstQueryTool {
    pub fn query(&self, input: AstQueryInput) -> Result<Vec<AstMatch>> {
        let path = std::path::Path::new(&input.file_path);
        let lang = Language::from_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unsupported file type: {}", input.file_path))?;

        let source = std::fs::read_to_string(path)?;

        let ts_language = match lang {
            Language::Java => tree_sitter_java::LANGUAGE.into(),
            Language::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
            Language::C | Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Language::Make
            | Language::Go
            | Language::Groovy
            | Language::Gradle
            | Language::Ruby
            | Language::Json
            | Language::Xml
            | Language::Yaml
            | Language::Aidl => {
                return Err(anyhow::anyhow!(
                    "AST query is not supported for file {}",
                    input.file_path
                ));
            }
            Language::Other(name) => {
                return Err(anyhow::anyhow!(
                    "AST query is not supported for language '{}' in file {}",
                    name,
                    input.file_path
                ));
            }
        };

        let mut parser = Parser::new();
        parser.set_language(&ts_language)?;

        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse {}", input.file_path))?;

        let query = Query::new(&ts_language, &input.pattern)?;
        let mut cursor = QueryCursor::new();

        let source_bytes = source.as_bytes();
        let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);

        let mut results = Vec::new();
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize].to_string();
                let text = node
                    .utf8_text(source_bytes)
                    .unwrap_or("")
                    .to_string();
                let start = node.start_position();
                let end = node.end_position();
                results.push(AstMatch {
                    capture_name,
                    text,
                    start_line: start.row as u32,
                    start_col: start.column as u32,
                    end_line: end.row as u32,
                    end_col: end.column as u32,
                });
            }
        }

        Ok(results)
    }
}
