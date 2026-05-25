//! Tree-sitter based Java symbol extractor.
use super::Symbol;
use tree_sitter::{Node, Parser};

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .expect("tree-sitter-java");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![],
    };

    let mut symbols = Vec::new();

    super::traverse_tree(tree.root_node(), "", |node, container| {
        match node.kind() {
            "class_declaration" | "interface_declaration" | "enum_declaration" | "annotation_type_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    let kind = match node.kind() {
                        "class_declaration" => "class",
                        "interface_declaration" => "interface",
                        "enum_declaration" => "enum",
                        _ => "annotation",
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
            "method_declaration" | "constructor_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    let kind = if node.kind() == "constructor_declaration" { "constructor" } else { "method" };
                    let sig = build_method_signature(node, source);
                    let (sl, sc, el, ec) = node_range(node);
                    symbols.push(Symbol {
                        name,
                        kind: kind.to_string(),
                        start_line: sl,
                        start_col: sc,
                        end_line: el,
                        end_col: ec,
                        container: container.to_string(),
                        signature: sig,
                    });
                }
            }
            "field_declaration" => {
                // A field can have multiple declarators
                for i in 0..node.child_count() {
                    let child = node.child(i).unwrap();
                    if child.kind() == "variable_declarator" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            let name = node_text(name_node, source);
                            let (sl, sc, el, ec) = node_range(node);
                            symbols.push(Symbol {
                                name,
                                kind: "field".to_string(),
                                start_line: sl,
                                start_col: sc,
                                end_line: el,
                                end_col: ec,
                                container: container.to_string(),
                                signature: String::new(),
                            });
                        }
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
    source
        .get(node.byte_range())
        .unwrap_or("")
        .to_string()
}

fn node_range(node: Node) -> (u32, u32, u32, u32) {
    let start = node.start_position();
    let end = node.end_position();
    (start.row as u32, start.column as u32, end.row as u32, end.column as u32)
}

fn build_method_signature(node: Node, source: &str) -> String {
    // Return type + name + parameter list
    let ret = node
        .child_by_field_name("type")
        .map(|n| node_text(n, source))
        .unwrap_or_default();
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source))
        .unwrap_or_default();
    let params = node
        .child_by_field_name("parameters")
        .map(|n| node_text(n, source))
        .unwrap_or_default();
    format!("{ret} {name}{params}").trim().to_string()
}
