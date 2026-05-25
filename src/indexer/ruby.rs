//! Tree-sitter based Ruby symbol extractor.
use tree_sitter::{Node, Parser};

use super::Symbol;

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_ruby::LANGUAGE.into())
        .expect("tree-sitter-ruby");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![],
    };

    let mut symbols = Vec::new();

    super::traverse_tree(tree.root_node(), "", |node, container| {
        match node.kind() {
            "class" | "module" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    let kind = if node.kind() == "class" { "class" } else { "module" };
                    let (sl, sc, el, ec) = node_range(node);
                    symbols.push(Symbol {
                        name: name.clone(),
                        kind: kind.to_string(),
                        start_line: sl,
                        start_col: sc,
                        end_line: el,
                        end_col: ec,
                        container: container.to_string(),
                        signature: node_text(node, source),
                    });

                    let new_container = if container.is_empty() {
                        name
                    } else {
                        format!("{container}::{name}")
                    };
                    return Some(Some(new_container));
                }
            }
            "method" | "singleton_method" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    let (sl, sc, el, ec) = node_range(node);
                    symbols.push(Symbol {
                        name,
                        kind: "method".to_string(),
                        start_line: sl,
                        start_col: sc,
                        end_line: el,
                        end_col: ec,
                        container: container.to_string(),
                        signature: node_text(node, source),
                    });
                }
            }
            _ => {}
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