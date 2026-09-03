//! Assertion helpers for tool calls and text content.

use serde::{Deserialize, Serialize};

/// A single tool call expectation.
///
/// The expected `name` and `args` are matched against actual tool calls.
/// `arg(key, value)` adds an expected argument; `arg_prefix` matches a
/// string prefix instead of exact equality.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedTool {
    pub name: String,
    pub args: Vec<ExpectedArg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedArg {
    pub key: String,
    pub value: ExpectedArgValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExpectedArgValue {
    Exact(String),
    Prefix(String),
    Contains(String),
    Any,
}

impl ExpectedArg {
    pub fn exact(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: ExpectedArgValue::Exact(value.into()),
        }
    }

    pub fn prefix(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: ExpectedArgValue::Prefix(value.into()),
        }
    }

    pub fn contains(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: ExpectedArgValue::Contains(value.into()),
        }
    }

    pub fn any(key: impl Into<String>) -> Self {
        Self { key: key.into(), value: ExpectedArgValue::Any }
    }

    pub fn matches(&self, actual: &serde_json::Value) -> bool {
        let v = match actual.get(&self.key) {
            Some(v) => v,
            None => return matches!(self.value, ExpectedArgValue::Any),
        };
        let s_owned;
        let s = match v.as_str() {
            Some(s) => s,
            None => {
                s_owned = v.to_string();
                s_owned.as_str()
            }
        };
        match &self.value {
            ExpectedArgValue::Exact(e) => s == e,
            ExpectedArgValue::Prefix(p) => s.starts_with(p.as_str()),
            ExpectedArgValue::Contains(c) => s.contains(c.as_str()),
            ExpectedArgValue::Any => true,
        }
    }
}

impl ExpectedTool {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), args: Vec::new() }
    }

    pub fn arg(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.args.push(ExpectedArg::exact(key, value));
        self
    }

    pub fn arg_prefix(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.args.push(ExpectedArg::prefix(key, value));
        self
    }

    pub fn arg_contains(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.args.push(ExpectedArg::contains(key, value));
        self
    }

    pub fn arg_any(mut self, key: impl Into<String>) -> Self {
        self.args.push(ExpectedArg::any(key));
        self
    }

    /// Check this expectation against an actual tool call.
    pub fn matches(&self, name: &str, args: &serde_json::Value) -> bool {
        if name != self.name {
            return false;
        }
        self.args.iter().all(|a| a.matches(args))
    }
}

/// Text-content expectation. All variants do substring matching unless noted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExpectedText {
    /// Substring match (case-sensitive).
    Contains(String),
    /// Exact match.
    Equals(String),
    /// Empty string.
    Empty,
    /// Any non-empty content.
    NonEmpty,
}

impl ExpectedText {
    pub fn contains(s: impl Into<String>) -> Self {
        ExpectedText::Contains(s.into())
    }

    pub fn equals(s: impl Into<String>) -> Self {
        ExpectedText::Equals(s.into())
    }

    pub fn matches(&self, actual: &str) -> bool {
        match self {
            ExpectedText::Contains(s) => actual.contains(s.as_str()),
            ExpectedText::Equals(s) => actual == s,
            ExpectedText::Empty => actual.is_empty(),
            ExpectedText::NonEmpty => !actual.is_empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_arg_matches() {
        let exp = ExpectedTool::new("read").arg("path", "README.md");
        let args = json!({"path": "README.md"});
        assert!(exp.matches("read", &args));
    }

    #[test]
    fn exact_arg_mismatch() {
        let exp = ExpectedTool::new("read").arg("path", "README.md");
        let args = json!({"path": "main.rs"});
        assert!(!exp.matches("read", &args));
    }

    #[test]
    fn prefix_arg_matches() {
        let exp = ExpectedTool::new("write").arg_prefix("path", "crates/");
        let args = json!({"path": "crates/forge_app/src/lib.rs"});
        assert!(exp.matches("write", &args));
    }

    #[test]
    fn contains_arg_matches() {
        let exp = ExpectedTool::new("shell").arg_contains("cmd", "cargo");
        let args = json!({"cmd": "cargo test --workspace"});
        assert!(exp.matches("shell", &args));
    }

    #[test]
    fn any_arg_matches_anything() {
        let exp = ExpectedTool::new("read").arg_any("path");
        let args = json!({"path": "anything"});
        assert!(exp.matches("read", &args));
    }

    #[test]
    fn name_mismatch_returns_false() {
        let exp = ExpectedTool::new("read");
        assert!(!exp.matches("write", &json!({})));
    }

    #[test]
    fn text_contains_matches() {
        assert!(ExpectedText::contains("hello").matches("say hello world"));
    }

    #[test]
    fn text_empty_matches() {
        assert!(ExpectedText::Empty.matches(""));
        assert!(!ExpectedText::Empty.matches("x"));
    }

    #[test]
    fn text_non_empty_matches() {
        assert!(ExpectedText::NonEmpty.matches("x"));
        assert!(!ExpectedText::NonEmpty.matches(""));
    }
}
