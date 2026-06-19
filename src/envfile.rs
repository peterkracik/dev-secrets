//! Parsing and serialising `.env` files.
//!
//! The parser is intentionally forgiving: it understands `KEY=VALUE`, blank
//! lines, `#` comments, an optional `export ` prefix, and single/double
//! quoted values. The serialiser quotes values only when needed.
//!
//! Values are arbitrary strings — they may contain `=`, spaces, JSON objects,
//! newlines, etc. Anything that would not survive a plain `KEY=VALUE` line is
//! wrapped in double quotes with `\\`, `\"`, `\n`, `\r` and `\t` escaped, so
//! the round-trip serialise → parse is lossless. A double-quoted JSON value
//! like `{"a": 1}` therefore stores and restores exactly.

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
    let chars: Vec<char> = value.chars().collect();
    if chars.len() >= 2 {
        let first = chars[0];
        let last = chars[chars.len() - 1];
        // Double quotes: process escape sequences.
        if first == '"' && last == '"' {
            let inner = &chars[1..chars.len() - 1];
            let mut out = String::with_capacity(inner.len());
            let mut i = 0;
            while i < inner.len() {
                if inner[i] == '\\' && i + 1 < inner.len() {
                    out.push(match inner[i + 1] {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        other => other, // covers \\ and \"
                    });
                    i += 2;
                } else {
                    out.push(inner[i]);
                    i += 1;
                }
            }
            return out;
        }
        // Single quotes: literal, no escape processing.
        if first == '\'' && last == '\'' {
            return chars[1..chars.len() - 1].iter().collect();
        }
    }
    value.to_string()
}

fn quote(value: &str) -> String {
    // Quote whenever the raw value would not survive a plain KEY=VALUE line.
    let needs_quotes = value.is_empty()
        || value.starts_with(['"', '\''])
        || value.contains(|c: char| c.is_whitespace())
        || value.contains('#')
        || value.contains('"')
        || value.contains('\'');
    if needs_quotes {
        let escaped = value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
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

    #[test]
    fn roundtrips_arbitrary_values() {
        let mut map = IndexMap::new();
        // JSON object value
        map.insert("CONFIG".to_string(), r#"{"host": "db", "port": 5432}"#.to_string());
        // value containing '='
        map.insert("EQUATION".to_string(), "a=b+c".to_string());
        // multi-line value
        map.insert("CERT".to_string(), "line1\nline2\nline3".to_string());
        // backslashes and tabs
        map.insert("WINPATH".to_string(), "C:\\tmp\tx".to_string());
        // a reference, which must be preserved verbatim
        map.insert("URL".to_string(), "http://${api.dev.HOST}:5432".to_string());

        let text = serialize(&map);
        let reparsed = parse(&text);
        assert_eq!(reparsed, map);
        // The reference is stored unquoted (no chars that force quoting).
        assert!(text.contains("URL=http://${api.dev.HOST}:5432\n"));
    }

    #[test]
    fn unquoted_value_with_equals() {
        let map = parse("KEY=a=b=c\n");
        assert_eq!(map.get("KEY").unwrap(), "a=b=c");
    }
}
