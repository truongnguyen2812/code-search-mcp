//! Tree-sitter based JSON symbol extractor.
use tree_sitter::{Node, Parser};

use super::Symbol;

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_json::LANGUAGE.into())
        .expect("tree-sitter-json");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![],
    };

    let mut symbols = Vec::new();

    super::traverse_tree(tree.root_node(), "", |node, container| {
        if node.kind() == "pair" {
            if let Some(key_node) = node.child_by_field_name("key") {
                let raw_key = node_text(key_node, source);
                let key = raw_key.trim_matches('"').to_string();
                let value_kind = node
                    .child_by_field_name("value")
                    .map(|v| v.kind().to_string())
                    .unwrap_or_default();
                let (sl, sc, el, ec) = node_range(key_node);

                symbols.push(Symbol {
                    name: key.clone(),
                    kind: "key".to_string(),
                    start_line: sl,
                    start_col: sc,
                    end_line: el,
                    end_col: ec,
                    container: container.to_string(),
                    signature: value_kind,
                });

                let next_container = if container.is_empty() {
                    key
                } else {
                    format!("{container}.{}", key)
                };

                return Some(Some(next_container));
            }
        }
        Some(None)
    });

    symbols
}

fn node_text(node: Node, source: &str) -> String {
    source.get(node.byte_range()).unwrap_or("").to_string()
}

fn node_range(node: Node) -> (u32, u32, u32, u32) {
    let start = node.start_position();
    let end = node.end_position();
    (start.row as u32, start.column as u32, end.row as u32, end.column as u32)
}