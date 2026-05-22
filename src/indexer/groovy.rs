//! Groovy symbol extractor.
use once_cell::sync::Lazy;
use regex::Regex;

use super::Symbol;

static TYPE_RE: Lazy<Option<Regex>> = Lazy::new(|| {
    Regex::new(r"^\s*(?:public\s+|private\s+|protected\s+|abstract\s+|final\s+|static\s+)*(class|interface|enum|trait)\s+([A-Za-z_][A-Za-z0-9_]*)")
        .ok()
});

static METHOD_RE: Lazy<Option<Regex>> = Lazy::new(|| {
    Regex::new(r"^\s*(?:public\s+|private\s+|protected\s+|static\s+|final\s+|synchronized\s+)*(?:def|[A-Za-z_][A-Za-z0-9_<>,\[\]?]*)\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(")
        .ok()
});

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let Some(type_re) = TYPE_RE.as_ref() else {
        return symbols;
    };
    let Some(method_re) = METHOD_RE.as_ref() else {
        return symbols;
    };
    let mut class_stack: Vec<(String, i32)> = Vec::new();
    let mut brace_depth: i32 = 0;

    for (line_idx, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("*") {
            update_scope(line, &mut brace_depth, &mut class_stack);
            continue;
        }

        let container = class_stack
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(".");

        if let Some(caps) = type_re.captures(line) {
            let kind = caps.get(1).map(|m| m.as_str()).unwrap_or("class");
            let name = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
            symbols.push(make_symbol(name, kind, &container, line_idx, line));
            class_stack.push((name.to_string(), brace_depth + 1));
            update_scope(line, &mut brace_depth, &mut class_stack);
            continue;
        }

        if let Some(caps) = method_re.captures(line) {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            if !matches!(name, "if" | "for" | "while" | "switch" | "catch") {
                symbols.push(make_symbol(name, "method", &container, line_idx, line));
            }
        }

        update_scope(line, &mut brace_depth, &mut class_stack);
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

fn update_scope(line: &str, brace_depth: &mut i32, class_stack: &mut Vec<(String, i32)>) {
    let opens = line.matches('{').count() as i32;
    let closes = line.matches('}').count() as i32;
    *brace_depth += opens - closes;
    while let Some((_, depth)) = class_stack.last() {
        if *depth > *brace_depth {
            class_stack.pop();
        } else {
            break;
        }
    }
}