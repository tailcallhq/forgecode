//! Value-type inference and variable substitution.
//!
//! Ghostty's wire format is untyped: every directive is a bare string.
//! This module turns the string into a typed [`ConfigValue`] using a
//! fixed rule ladder, and rewrites `$name`/`${name}` placeholders
//! using a caller-supplied variable table.

use std::collections::HashMap;

use crate::config::ConfigValue;

/// Infer a [`ConfigValue`] from a raw token using the rule ladder
/// described in the module docs of [`crate`].
pub fn infer_value(key: &str, raw: &str) -> ConfigValue {
    let value = unquote(raw);

    // 1. Color literal: `#RRGGBB` or `#RRGGBBAA`
    if let Some(rgba) = parse_color(value) {
        return ConfigValue::Color(rgba);
    }

    // 2. Boolean: true/false/yes/no/on/off (case-insensitive)
    if let Some(b) = parse_bool(value) {
        return ConfigValue::Bool(b);
    }

    // 3. Integer
    if let Some(n) = parse_integer(value) {
        return ConfigValue::Integer(n);
    }

    // 4. List: only `font-family` is treated as a comma-separated
    // list in this crate. Other comma-bearing keys are stored as
    // single strings.
    if key == "font-family" && value.contains(',') {
        let parts: Vec<String> = value
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() > 1 {
            return ConfigValue::List(parts);
        }
    }

    ConfigValue::String(value.to_string())
}

/// Strip a single matched pair of surrounding quotes (`"` or `'`) if
/// present. Used so users can quote values that contain spaces or
/// `=` without losing the quotes' intent.
pub fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &value[1..bytes.len() - 1];
        }
    }
    value
}

fn parse_color(value: &str) -> Option<u32> {
    let rest = value.strip_prefix('#')?;
    if rest.len() != 6 && rest.len() != 8 {
        return None;
    }
    if !rest.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(rest, 16).ok()?;
    Some(if rest.len() == 6 { (n << 8) | 0xFF } else { n })
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Some(true),
        "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_integer(value: &str) -> Option<i64> {
    if value.is_empty() {
        return None;
    }
    if !value
        .bytes()
        .enumerate()
        .all(|(i, b)| b.is_ascii_digit() || (i == 0 && b == b'-'))
    {
        return None;
    }
    value.parse().ok()
}

/// Apply variable substitution to a single value.
pub fn substitute_value(value: &ConfigValue, vars: &HashMap<String, String>) -> ConfigValue {
    match value {
        ConfigValue::String(s) => ConfigValue::String(substitute_string(s, vars)),
        ConfigValue::List(items) => {
            ConfigValue::List(items.iter().map(|s| substitute_string(s, vars)).collect())
        }
        other => other.clone(),
    }
}

/// Substitute `$name` and `${name}` placeholders inside a string.
/// Undefined variables are left as their literal form.
pub fn substitute_string(input: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // `${name}` form
        let consumed = match bytes[i] {
            b'$' if i + 1 < bytes.len() && bytes[i + 1] == b'{' => {
                match input[i + 2..].find('}') {
                    Some(end) => {
                        let name = &input[i + 2..i + 2 + end];
                        let consumed = 2 + end + 1;
                        if let Some(value) = vars.get(name) {
                            out.push_str(value);
                        } else {
                            out.push_str(&input[i..i + consumed]);
                        }
                        Some(consumed)
                    }
                    None => None,
                }
            }
            _ => None,
        };
        if let Some(consumed) = consumed {
            i += consumed;
            continue;
        }
        // `$name` form (only matches when the `$` is followed by a valid
        // identifier character, otherwise we fall through and emit `$`
        // literally).
        if bytes[i] == b'$' {
            let rest = &input[i + 1..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() {
                out.push('$');
                i += 1;
            } else if let Some(value) = vars.get(&name) {
                out.push_str(value);
                i += 1 + name.len();
            } else {
                out.push('$');
                out.push_str(&name);
                i += 1 + name.len();
            }
        } else {
            let ch = input[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}
