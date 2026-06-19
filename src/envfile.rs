//! Parsing and serialising `.env` files.
//!
//! The parser is intentionally forgiving: it understands `KEY=VALUE`, blank
//! lines, `#` comments, an optional `export ` prefix, and single/double
//! quoted values. The serialiser quotes values only when needed.

use indexmap::IndexMap;

/// Parse `.env` text into an ordered map of key/value pairs.
pub fn parse(text: &str) -> IndexMap<String, String> {
    let mut map = IndexMap::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        map.insert(key.to_string(), unquote(value.trim()));
    }
    map
}

/// Serialise key/value pairs into `.env` text.
pub fn serialize(values: &IndexMap<String, String>) -> String {
    let mut out = String::new();
    for (key, value) in values {
        out.push_str(key);
        out.push('=');
        out.push_str(&quote(value));
        out.push('\n');
    }
    out
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if value.len() >= 2 {
        let first = bytes[0];
        let last = bytes[value.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            let inner = &value[1..value.len() - 1];
            if first == b'"' {
                return inner.replace("\\n", "\n").replace("\\\"", "\"");
            }
            return inner.to_string();
        }
    }
    value.to_string()
}

fn quote(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value.contains(|c: char| c.is_whitespace())
        || value.contains('#')
        || value.contains('"')
        || value.contains('\'');
    if needs_quotes {
        let escaped = value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic() {
        let text = "# comment\nFOO=bar\nexport BAZ=\"hello world\"\n\nEMPTY=\n";
        let map = parse(text);
        assert_eq!(map.get("FOO").unwrap(), "bar");
        assert_eq!(map.get("BAZ").unwrap(), "hello world");
        assert_eq!(map.get("EMPTY").unwrap(), "");
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn roundtrip_quotes_when_needed() {
        let mut map = IndexMap::new();
        map.insert("A".to_string(), "simple".to_string());
        map.insert("B".to_string(), "with space".to_string());
        let text = serialize(&map);
        assert!(text.contains("A=simple\n"));
        assert!(text.contains("B=\"with space\"\n"));
        let reparsed = parse(&text);
        assert_eq!(reparsed, map);
    }
}
