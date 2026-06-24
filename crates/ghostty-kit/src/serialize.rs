//! Deterministic JSON serializer for [`GhosttyConfig`].
//!
//! Used by the golden-file tests in `tests/golden.rs`. The output is
//! stable across runs and platforms: keys appear in source order,
//! paths are stringified with their display form, and colors are
//! emitted as `#RRGGBBAA`.

use std::fmt::Write as _;

use crate::config::{ConfigEntry, ConfigValue, GhosttyConfig};

/// Serialize a [`GhosttyConfig`] to a stable JSON string.
///
/// Public so integration tests in `tests/golden.rs` can use it.
#[doc(hidden)]
pub fn to_json(config: &GhosttyConfig) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    let _ = writeln!(
        s,
        "  \"source\": {},",
        json_string(&config.source.display().to_string())
    );
    let _ = writeln!(s, "  \"includes\": [");
    for (i, inc) in config.includes.iter().enumerate() {
        let comma = if i + 1 < config.includes.len() { "," } else { "" };
        let _ = writeln!(
            s,
            "    {}{}",
            json_string(&inc.display().to_string()),
            comma
        );
    }
    let _ = writeln!(s, "  ],");
    let _ = writeln!(s, "  \"entries\": [");
    for (i, entry) in config.entries.iter().enumerate() {
        let comma = if i + 1 < config.entries.len() { "," } else { "" };
        let _ = writeln!(s, "    {}{}", entry_to_json(entry), comma);
    }
    let _ = writeln!(s, "  ]");
    s.push('}');
    s
}

fn entry_to_json(entry: &ConfigEntry) -> String {
    match entry {
        ConfigEntry::KeyValue {
            key,
            value,
            section,
            line,
        } => format!(
            "{{ \"type\": \"key_value\", \"key\": {}, \"value\": {}, \"section\": {}, \"line\": {} }}",
            json_string(key),
            value_to_json(value),
            section
                .as_ref()
                .map(|s| json_string(s))
                .unwrap_or_else(|| "null".to_string()),
            line,
        ),
        ConfigEntry::Include(p) => format!(
            "{{ \"type\": \"include\", \"path\": {} }}",
            json_string(&p.display().to_string())
        ),
        ConfigEntry::Section(name, line) => format!(
            "{{ \"type\": \"section\", \"name\": {}, \"line\": {} }}",
            json_string(name),
            line,
        ),
    }
}

fn value_to_json(value: &ConfigValue) -> String {
    match value {
        ConfigValue::String(s) => format!(
            "{{ \"type\": \"string\", \"value\": {} }}",
            json_string(s)
        ),
        ConfigValue::Bool(b) => format!("{{ \"type\": \"bool\", \"value\": {} }}", b),
        ConfigValue::Integer(n) => format!("{{ \"type\": \"integer\", \"value\": {} }}", n),
        ConfigValue::Color(rgba) => {
            let r = (rgba >> 24) & 0xFF;
            let g = (rgba >> 16) & 0xFF;
            let b = (rgba >> 8) & 0xFF;
            let a = rgba & 0xFF;
            format!(
                "{{ \"type\": \"color\", \"value\": \"#{:02X}{:02X}{:02X}{:02X}\" }}",
                r, g, b, a
            )
        }
        ConfigValue::List(items) => {
            let parts: Vec<String> = items.iter().map(|s| json_string(s)).collect();
            format!(
                "{{ \"type\": \"list\", \"value\": [{}] }}",
                parts.join(", ")
            )
        }
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
