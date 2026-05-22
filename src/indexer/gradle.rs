//! Gradle symbol extractor.
use once_cell::sync::Lazy;
use regex::Regex;

use super::Symbol;

static TASK_RE: Lazy<Option<Regex>> =
    Lazy::new(|| Regex::new(r"^\s*task\s+([A-Za-z_][A-Za-z0-9_-]*)\b").ok());
static TASK_REGISTER_RE: Lazy<Option<Regex>> = Lazy::new(|| {
    Regex::new(r#"tasks\.(?:register|named)\(\s*['"]([A-Za-z_][A-Za-z0-9_-]*)['"]"#).ok()
});

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut symbols = crate::indexer::groovy::extract_symbols(source);
    let Some(task_re) = TASK_RE.as_ref() else {
        return symbols;
    };
    let Some(task_register_re) = TASK_REGISTER_RE.as_ref() else {
        return symbols;
    };

    for (line_idx, line) in source.lines().enumerate() {
        if let Some(caps) = task_re.captures(line) {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            symbols.push(make_symbol(name, "task", line_idx, line));
            continue;
        }
        if let Some(caps) = task_register_re.captures(line) {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            symbols.push(make_symbol(name, "task", line_idx, line));
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