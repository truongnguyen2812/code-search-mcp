//! AIDL symbol extractor.
use once_cell::sync::Lazy;
use regex::Regex;

use super::Symbol;

static PACKAGE_RE: Lazy<Option<Regex>> =
    Lazy::new(|| Regex::new(r"^\s*package\s+([A-Za-z_][A-Za-z0-9_.]*)\s*;").ok());
static TYPE_RE: Lazy<Option<Regex>> = Lazy::new(|| {
    Regex::new(r"^\s*(interface|parcelable|union|enum)\s+([A-Za-z_][A-Za-z0-9_]*)")
        .ok()
});
static METHOD_RE: Lazy<Option<Regex>> = Lazy::new(|| {
    Regex::new(r"^\s*(?:oneway\s+)?[A-Za-z_][A-Za-z0-9_<>\[\].\s]*\s+([A-Za-z_][A-Za-z0-9_]*)\s*\([^;]*\)\s*;")
        .ok()
});

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let Some(package_re) = PACKAGE_RE.as_ref() else {
        return symbols;
    };
    let Some(type_re) = TYPE_RE.as_ref() else {
        return symbols;
    };
    let Some(method_re) = METHOD_RE.as_ref() else {
        return symbols;
    };
    let mut package_name = String::new();
    let mut type_name = String::new();

    for (line_idx, line) in source.lines().enumerate() {
        if package_name.is_empty() {
            if let Some(caps) = package_re.captures(line) {
                package_name = caps
                    .get(1)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();
            }
        }

        if let Some(caps) = type_re.captures(line) {
            let kind = caps.get(1).map(|m| m.as_str()).unwrap_or("interface");
            let name = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
            symbols.push(make_symbol(name, kind, &package_name, line_idx, line));
            type_name = name.to_string();
            continue;
        }

        if let Some(caps) = method_re.captures(line) {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let container = if package_name.is_empty() {
                type_name.clone()
            } else if type_name.is_empty() {
                package_name.clone()
            } else {
                format!("{}.{}", package_name, type_name)
            };
            symbols.push(make_symbol(name, "method", &container, line_idx, line));
        }
    }

    symbols
}

fn make_symbol(name: &str, kind: &str, container: &str, line_idx: usize, line: &str) -> Symbol {
    let col = line.find(name).unwrap_or(0) as u32;
    Symbol {
        name: name.to_string(),
        kind: kind.to_string(),
        start_line: line_idx as u32,
        start_col: col,
        end_line: line_idx as u32,
        end_col: (col as usize + name.len()) as u32,
        container: container.to_string(),
        signature: line.trim().to_string(),
    }
}