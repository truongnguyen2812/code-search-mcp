//! YAML symbol extractor.
use once_cell::sync::Lazy;
use regex::Regex;

use super::Symbol;

static KEY_RE: Lazy<Option<Regex>> =
    Lazy::new(|| Regex::new(r"^(\s*)([A-Za-z0-9_.-]+)\s*:").ok());
static LIST_KEY_RE: Lazy<Option<Regex>> =
    Lazy::new(|| Regex::new(r"^(\s*)-\s*([A-Za-z0-9_.-]+)\s*:").ok());

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let Some(key_re) = KEY_RE.as_ref() else {
        return symbols;
    };
    let Some(list_key_re) = LIST_KEY_RE.as_ref() else {
        return symbols;
    };
    let mut stack: Vec<(usize, String)> = Vec::new();

    for (line_idx, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let captures = key_re.captures(line).or_else(|| list_key_re.captures(line));
        let Some(caps) = captures else {
            continue;
        };

        let indent = caps.get(1).map(|m| m.as_str().len()).unwrap_or(0);
        let key = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

        while let Some((last_indent, _)) = stack.last() {
            if *last_indent >= indent {
                stack.pop();
            } else {
                break;
            }
        }

        let container = stack
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(".");

        let col = line.find(key).unwrap_or(0) as u32;
        symbols.push(Symbol {
            name: key.to_string(),
            kind: "key".to_string(),
            start_line: line_idx as u32,
            start_col: col,
            end_line: line_idx as u32,
            end_col: (col as usize + key.len()) as u32,
            container,
            signature: line.trim().to_string(),
        });

        stack.push((indent, key.to_string()));
    }

    symbols
}