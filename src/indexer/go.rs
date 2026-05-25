//! Tree-sitter based Go symbol extractor.
use tree_sitter::{Node, Parser};

use super::Symbol;

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .expect("tree-sitter-go");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![],
    };

    let mut symbols = Vec::new();

    super::traverse_tree(tree.root_node(), "", |node, container| {
        match node.kind() {
            "package_clause" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let package = node_text(name_node, source);
                    return Some(Some(package));
                }
            }
            "type_spec" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    let kind = match node.child_by_field_name("type") {
                        Some(type_node) if type_node.kind() == "struct_type" => "struct",
                        Some(type_node) if type_node.kind() == "interface_type" => "interface",
                        _ => "type",
                    };
                    let (sl, sc, el, ec) = node_range(node);
                    symbols.push(Symbol {
                        name,
                        kind: kind.to_string(),
                        start_line: sl,
                        start_col: sc,
                        end_line: el,
                        end_col: ec,
                        container: container.to_string(),
                        signature: node_text(node, source),
                    });
                }
            }
            "function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    let (sl, sc, el, ec) = node_range(node);
                    symbols.push(Symbol {
                        name,
                        kind: "function".to_string(),
                        start_line: sl,
                        start_col: sc,
                        end_line: el,
                        end_col: ec,
                        container: container.to_string(),
                        signature: node_text(node, source),
                    });
                }
            }
            "method_declaration" => {
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
            "var_spec" | "const_spec" => {
                for i in 0..node.child_count() {
                    let child = node.child(i).unwrap();
                    if child.kind() == "identifier" {
                        let name = node_text(child, source);
                        let (sl, sc, el, ec) = node_range(child);
                        symbols.push(Symbol {
                            name,
                            kind: "variable".to_string(),
                            start_line: sl,
                            start_col: sc,
                            end_line: el,
                            end_col: ec,
                            container: container.to_string(),
                            signature: node_text(node, source),
                        });
                    }
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