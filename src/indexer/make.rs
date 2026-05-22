//! Makefile symbol extractor.
use once_cell::sync::Lazy;
use regex::Regex;

use super::Symbol;

static TARGET_RE: Lazy<Option<Regex>> =
    Lazy::new(|| Regex::new(r"^\s*([A-Za-z0-9_./%+-]+)\s*:(?!=)").ok());
static VAR_RE: Lazy<Option<Regex>> = Lazy::new(|| {
    Regex::new(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*(?::=|\+=|\?=|=)").ok()
});

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let Some(target_re) = TARGET_RE.as_ref() else {
        return symbols;
    };
    let Some(var_re) = VAR_RE.as_ref() else {
        return symbols;
    };

    for (line_idx, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || line.starts_with('\t') {
            continue;
        }

        if let Some(caps) = target_re.captures(line) {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            symbols.push(make_symbol(name, "target", line_idx, line));
            continue;
        }

        if let Some(caps) = var_re.captures(line) {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            symbols.push(make_symbol(name, "variable", line_idx, line));
        }
    }

    symbols
}

fn make_symbol(name: &str, kind: &str, line_idx: usize, line: &str) -> Symbol {
    let col = line.find(name).unwrap_or(0) as u32;
    Symbol {
        name: name.to_string(),
        kind: kind.to_string(),
        start_line: line_idx as u32,
        start_col: col,
        end_line: line_idx as u32,
        end_col: (col as usize + name.len()) as u32,
        container: String::new(),
        signature: line.trim().to_string(),
    }
}