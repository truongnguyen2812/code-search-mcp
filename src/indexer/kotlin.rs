//! Tree-sitter based Kotlin symbol extractor.
use super::Symbol;
use tree_sitter::{Node, Parser};

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
        .expect("tree-sitter-kotlin-ng");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![],
    };

    let mut symbols = Vec::new();

    super::traverse_tree(tree.root_node(), "", |node, container| {
        match node.kind() {
            "class_declaration" | "object_declaration" | "interface_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    let kind = match node.kind() {
                        "object_declaration" => "object",
                        "interface_declaration" => "interface",
                        _ => "class",
                    };
                    let (sl, sc, el, ec) = node_range(node);
                    symbols.push(Symbol {
                        name: name.clone(),
                        kind: kind.to_string(),
                        start_line: sl,
                        start_col: sc,
                        end_line: el,
                        end_col: ec,
                        container: container.to_string(),
                        signature: String::new(),
                    });
                    let new_container = if container.is_empty() {
                        name.clone()
                    } else {
                        format!("{container}.{name}")
                    };
                    return Some(Some(new_container));
                }
            }
            "function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    let sig = build_fn_signature(node, source);
                    let (sl, sc, el, ec) = node_range(node);
                    symbols.push(Symbol {
                        name,
                        kind: "function".to_string(),
                        start_line: sl,
                        start_col: sc,
                        end_line: el,
                        end_col: ec,
                        container: container.to_string(),
                        signature: sig,
                    });
                }
            }
            "property_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    let (sl, sc, el, ec) = node_range(node);
                    symbols.push(Symbol {
                        name,
                        kind: "property".to_string(),
                        start_line: sl,
                        start_col: sc,
                        end_line: el,
                        end_col: ec,
                        container: container.to_string(),
                        signature: String::new(),
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

fn build_fn_signature(node: Node, source: &str) -> String {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source))
        .unwrap_or_default();
    let params = node
        .child_by_field_name("function_value_parameters")
        .map(|n| node_text(n, source))
        .unwrap_or_default();
    let ret = node
        .child_by_field_name("return_type")
        .map(|n| node_text(n, source))
        .unwrap_or_default();
    if ret.is_empty() {
        format!("fun {name}{params}")
    } else {
        format!("fun {name}{params}: {ret}")
    }
}
