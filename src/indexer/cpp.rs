//! Tree-sitter based C/C++ symbol extractor.
use super::Symbol;
use tree_sitter::{Node, Parser};

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_cpp::LANGUAGE.into())
        .expect("tree-sitter-cpp");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![],
    };

    let mut symbols = Vec::new();
    visit_node(tree.root_node(), source, "", &mut symbols);
    symbols
}

fn visit_node(root: Node, source: &str, root_container: &str, symbols: &mut Vec<Symbol>) {
    // Iterative traversal to avoid stack overflow on deeply nested ASTs.
    let mut stack: Vec<(Node, String)> = vec![(root, root_container.to_string())];
    while let Some((node, container)) = stack.pop() {
        match node.kind() {
            "class_specifier" | "struct_specifier" | "enum_specifier" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    let kind = match node.kind() {
                        "class_specifier" => "class",
                        "struct_specifier" => "struct",
                        _ => "enum",
                    };
                    let (sl, sc, el, ec) = node_range(node);
                    symbols.push(Symbol {
                        name: name.clone(),
                        kind: kind.to_string(),
                        start_line: sl,
                        start_col: sc,
                        end_line: el,
                        end_col: ec,
                        container: container.clone(),
                        signature: String::new(),
                    });
                    let new_container = if container.is_empty() {
                        name.clone()
                    } else {
                        format!("{container}::{name}")
                    };
                    for i in 0..node.child_count() {
                        stack.push((node.child(i).unwrap(), new_container.clone()));
                    }
                    continue;
                }
            }
            "function_definition" => {
                let name = extract_function_name(node, source);
                if !name.is_empty() {
                    let sig = build_fn_signature(node, source);
                    let (sl, sc, el, ec) = node_range(node);
                    symbols.push(Symbol {
                        name,
                        kind: "function".to_string(),
                        start_line: sl,
                        start_col: sc,
                        end_line: el,
                        end_col: ec,
                        container: container.clone(),
                        signature: sig,
                    });
                }
            }
            "declaration" => {
                // Variable / typedef declarations at namespace/struct scope
                for i in 0..node.child_count() {
                    let child = node.child(i).unwrap();
                    if child.kind() == "init_declarator" || child.kind() == "declarator" {
                        if let Some(n) = find_identifier(child, source) {
                            let (sl, sc, el, ec) = node_range(node);
                            symbols.push(Symbol {
                                name: n,
                                kind: "variable".to_string(),
                                start_line: sl,
                                start_col: sc,
                                end_line: el,
                                end_col: ec,
                                container: container.clone(),
                                signature: String::new(),
                            });
                        }
                    }
                }
            }
            "namespace_definition" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| node_text(n, source))
                    .unwrap_or_else(|| "<anon>".to_string());
                let new_container = if container.is_empty() {
                    name.clone()
                } else {
                    format!("{container}::{name}")
                };
                for i in 0..node.child_count() {
                    stack.push((node.child(i).unwrap(), new_container.clone()));
                }
                continue;
            }
            _ => {}
        }

        for i in 0..node.child_count() {
            stack.push((node.child(i).unwrap(), container.clone()));
        }
    }
}

fn extract_function_name(node: Node, source: &str) -> String {
    // declarator field usually contains the function name
    if let Some(decl) = node.child_by_field_name("declarator") {
        return find_identifier(decl, source).unwrap_or_default();
    }
    String::new()
}

fn find_identifier(root: Node, source: &str) -> Option<String> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "identifier" || node.kind() == "field_identifier" {
            return Some(node_text(node, source));
        }
        // qualified names like Foo::bar
        if node.kind() == "qualified_identifier" {
            if let Some(name) = node.child_by_field_name("name") {
                return Some(node_text(name, source));
            }
        }
        for i in 0..node.child_count() {
            stack.push(node.child(i).unwrap());
        }
    }
    None
}

fn build_fn_signature(node: Node, source: &str) -> String {
    // type + declarator (includes parameter list)
    let type_part = node
        .child_by_field_name("type")
        .map(|n| node_text(n, source))
        .unwrap_or_default();
    let decl_part = node
        .child_by_field_name("declarator")
        .map(|n| node_text(n, source))
        .unwrap_or_default();
    format!("{type_part} {decl_part}").trim().to_string()
}

fn node_text(node: Node, source: &str) -> String {
    source.get(node.byte_range()).unwrap_or("").to_string()
}

fn node_range(node: Node) -> (u32, u32, u32, u32) {
    let start = node.start_position();
    let end = node.end_position();
    (start.row as u32, start.column as u32, end.row as u32, end.column as u32)
}
