//! XML symbol extractor.
use once_cell::sync::Lazy;
use regex::Regex;

use super::Symbol;

static OPEN_TAG_RE: Lazy<Option<Regex>> =
    Lazy::new(|| Regex::new(r"<([A-Za-z_][A-Za-z0-9_.:-]*)\b[^>]*?>").ok());
static CLOSE_TAG_RE: Lazy<Option<Regex>> =
    Lazy::new(|| Regex::new(r"</([A-Za-z_][A-Za-z0-9_.:-]*)\s*>").ok());

pub fn extract_symbols(source: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let Some(open_tag_re) = OPEN_TAG_RE.as_ref() else {
        return symbols;
    };
    let Some(close_tag_re) = CLOSE_TAG_RE.as_ref() else {
        return symbols;
    };
    let mut stack: Vec<String> = Vec::new();

    for (line_idx, line) in source.lines().enumerate() {
        for caps in close_tag_re.captures_iter(line) {
            if let Some(name) = caps.get(1).map(|m| m.as_str()) {
                if let Some(pos) = stack.iter().rposition(|v| v == name) {
                    stack.truncate(pos);
                }
            }
        }

        for caps in open_tag_re.captures_iter(line) {
            let full = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
            if full.starts_with("</") || full.starts_with("<?") || full.starts_with("<!") {
                continue;
            }
            let Some(name) = caps.get(1).map(|m| m.as_str()) else {
                continue;
            };

            let container = stack.join("/");
            let col = line.find(name).unwrap_or(0) as u32;
            symbols.push(Symbol {
                name: name.to_string(),
                kind: "element".to_string(),
                start_line: line_idx as u32,
                start_col: col,
                end_line: line_idx as u32,
                end_col: (col as usize + name.len()) as u32,
                container,
                signature: full.to_string(),
            });

            if !full.ends_with("/>") {
                stack.push(name.to_string());
            }
        }
    }

    symbols
}