use std::collections::HashMap;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

use crate::types::{FileSymbols, Symbol, SymbolKind};

/// A language definition: the tree-sitter `Language`, a set of queries that
/// extract top-level symbols, and file-extension matchers.
struct LangDef {
    language: Language,
    /// Queries that each extract one or more `(capture_name, SymbolKind)`
    /// pairs.
    queries: Vec<(String, SymbolKind)>,
}

/// Build a map of supported language definitions.
fn supported_languages() -> HashMap<&'static str, LangDef> {
    let mut map: HashMap<&'static str, LangDef> = HashMap::new();

    // ── Rust ──────────────────────────────────────────────────────────────
    map.insert(
        "rust",
        LangDef {
            language: tree_sitter_rust::LANGUAGE.into(),
            queries: vec![
                (
                    "(function_item name: (identifier) @name)".into(),
                    SymbolKind::Function,
                ),
                (
                    "(struct_item name: (type_identifier) @name)".into(),
                    SymbolKind::Struct,
                ),
                (
                    "(enum_item name: (type_identifier) @name)".into(),
                    SymbolKind::Enum,
                ),
                (
                    "(trait_item name: (type_identifier) @name)".into(),
                    SymbolKind::Trait,
                ),
                (
                    "(type_item name: (type_identifier) @name)".into(),
                    SymbolKind::TypeAlias,
                ),
                (
                    "(const_item name: (identifier) @name)".into(),
                    SymbolKind::Const,
                ),
                (
                    "(static_item name: (identifier) @name)".into(),
                    SymbolKind::Static,
                ),
                (
                    "(macro_definition name: (identifier) @name)".into(),
                    SymbolKind::Macro,
                ),
                (
                    "(impl_item type: (type_identifier) @name)".into(),
                    SymbolKind::Impl,
                ),
            ],
        },
    );

    // ── TypeScript / TSX ──────────────────────────────────────────────────
    map.insert(
        "typescript",
        LangDef {
            language: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            queries: vec![
                (
                    "(function_declaration name: (identifier) @name)".into(),
                    SymbolKind::Function,
                ),
                (
                    "(class_declaration name: (type_identifier) @name)".into(),
                    SymbolKind::Class,
                ),
                (
                    "(interface_declaration name: (type_identifier) @name)".into(),
                    SymbolKind::Interface,
                ),
                (
                    "(type_alias_declaration name: (type_identifier) @name)".into(),
                    SymbolKind::TypeAlias,
                ),
                (
                    "(enum_declaration name: (identifier) @name)".into(),
                    SymbolKind::Enum,
                ),
            ],
        },
    );

    // ── TypeScript React (TSX) ────────────────────────────────────────────
    map.insert(
        "tsx",
        LangDef {
            language: tree_sitter_typescript::LANGUAGE_TSX.into(),
            queries: vec![
                (
                    "(function_declaration name: (identifier) @name)".into(),
                    SymbolKind::Function,
                ),
                (
                    "(class_declaration name: (type_identifier) @name)".into(),
                    SymbolKind::Class,
                ),
                (
                    "(interface_declaration name: (type_identifier) @name)".into(),
                    SymbolKind::Interface,
                ),
                (
                    "(type_alias_declaration name: (type_identifier) @name)".into(),
                    SymbolKind::TypeAlias,
                ),
                (
                    "(enum_declaration name: (identifier) @name)".into(),
                    SymbolKind::Enum,
                ),
            ],
        },
    );

    // ── JavaScript / JSX ──────────────────────────────────────────────────
    map.insert(
        "javascript",
        LangDef {
            language: tree_sitter_javascript::LANGUAGE.into(),
            queries: vec![
                (
                    "(function_declaration name: (identifier) @name)".into(),
                    SymbolKind::Function,
                ),
                (
                    "(class_declaration name: (type_identifier) @name)".into(),
                    SymbolKind::Class,
                ),
            ],
        },
    );

    // ── Python ────────────────────────────────────────────────────────────
    map.insert(
        "python",
        LangDef {
            language: tree_sitter_python::LANGUAGE.into(),
            queries: vec![
                (
                    "(function_definition name: (identifier) @name)".into(),
                    SymbolKind::Function,
                ),
                (
                    "(class_definition name: (identifier) @name)".into(),
                    SymbolKind::Class,
                ),
            ],
        },
    );

    // ── Go ────────────────────────────────────────────────────────────────
    map.insert(
        "go",
        LangDef {
            language: tree_sitter_go::LANGUAGE.into(),
            queries: vec![
                (
                    "(function_declaration name: (identifier) @name)".into(),
                    SymbolKind::Function,
                ),
                (
                    "(method_declaration name: (field_identifier) @name)".into(),
                    SymbolKind::Method,
                ),
                (
                    "(type_declaration (type_spec name: (type_identifier) @name))".into(),
                    SymbolKind::TypeAlias,
                ),
            ],
        },
    );

    map
}

/// Language name → list of file extensions.
fn language_extensions() -> HashMap<&'static str, Vec<&'static str>> {
    let mut map: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
    map.insert("rust", vec!["rs"]);
    map.insert("typescript", vec!["ts"]);
    map.insert("tsx", vec!["tsx"]);
    map.insert("javascript", vec!["js", "jsx", "mjs", "cjs"]);
    map.insert("python", vec!["py", "pyi"]);
    map.insert("go", vec!["go"]);
    map
}

/// Detect the language name from a file path.
pub fn detect_language(path: &str) -> Option<&'static str> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())?;

    let ext_map = language_extensions();
    for (lang, exts) in &ext_map {
        if exts.contains(&ext.as_str()) {
            return Some(lang);
        }
    }
    None
}

/// Parse a single source file and return its symbols.
pub fn parse_file(path: &str, source: &str) -> Result<FileSymbols> {
    let lang_name = detect_language(path).context("unsupported or unrecognized language")?;
    let langs = supported_languages();
    let def = langs
        .get(lang_name)
        .context("language definition not found")?;

    let mut parser = Parser::new();
    parser
        .set_language(&def.language)
        .context("failed to set tree-sitter language")?;

    let tree = parser
        .parse(source, None)
        .context("failed to parse source file")?;
    let root = tree.root_node();

    let mut symbols: Vec<Symbol> = Vec::new();
    let mut seen: Vec<(String, SymbolKind)> = Vec::new();

    for (query_str, kind) in &def.queries {
        let query = Query::new(&def.language, query_str)
            .with_context(|| format!("failed to compile query: {query_str}"))?;
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());

        // tree-sitter 0.24 QueryMatches is a StreamingIterator.
        while let Some(match_) = matches.next() {
            for capture in match_.captures.iter() {
                let name_idx = capture.index as usize;
                let Some(name) = query.capture_names().get(name_idx) else {
                    continue;
                };
                if *name != "name" {
                    continue;
                }
                let node = capture.node;
                let name_str = node
                    .utf8_text(source.as_bytes())
                    .unwrap_or_default()
                    .to_string();

                // Deduplicate within this file.
                if seen.contains(&(name_str.clone(), *kind)) {
                    continue;
                }
                seen.push((name_str.clone(), *kind));

                symbols.push(Symbol {
                    name: name_str,
                    kind: *kind,
                    line: node.start_position().row + 1, // 1-based
                });
            }
        }
    }

    // Sort by line number.
    symbols.sort_by_key(|s| s.line);

    Ok(FileSymbols {
        path: path.to_string(),
        language: lang_name.to_string(),
        symbols,
    })
}

/// Parse many files returning only those whose language is recognised.
pub fn parse_files(files: &[crate::types::FileEntry]) -> Vec<anyhow::Result<FileSymbols>> {
    files
        .iter()
        .filter(|f| detect_language(&f.path).is_some())
        .map(|f| {
            let content = std::fs::read_to_string(&f.path)
                .with_context(|| format!("failed to read {}", f.path))?;
            parse_file(&f.path, &content)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language_rust() {
        assert_eq!(detect_language("src/lib.rs"), Some("rust"));
        assert_eq!(detect_language("main.rs"), Some("rust"));
    }

    #[test]
    fn test_detect_language_typescript() {
        assert_eq!(detect_language("index.ts"), Some("typescript"));
        assert_eq!(detect_language("component.tsx"), Some("tsx"));
    }

    #[test]
    fn test_detect_language_javascript() {
        assert_eq!(detect_language("app.js"), Some("javascript"));
        assert_eq!(detect_language("app.jsx"), Some("javascript"));
        assert_eq!(detect_language("module.mjs"), Some("javascript"));
    }

    #[test]
    fn test_detect_language_python() {
        assert_eq!(detect_language("main.py"), Some("python"));
        assert_eq!(detect_language("types.pyi"), Some("python"));
    }

    #[test]
    fn test_detect_language_go() {
        assert_eq!(detect_language("server.go"), Some("go"));
    }

    #[test]
    fn test_detect_language_unknown() {
        assert_eq!(detect_language("readme.md"), None);
        assert_eq!(detect_language("Makefile"), None);
        assert_eq!(detect_language(""), None);
    }

    #[test]
    fn test_parse_rust_functions_and_structs() {
        let source = r#"
use std::collections::HashMap;

/// A widget that does things.
struct Widget {
    name: String,
}

impl Widget {
    fn new(name: String) -> Self {
        Widget { name }
    }

    pub fn greet(&self) -> String {
        format!("Hello, {}", self.name)
    }
}

enum Color {
    Red,
    Blue,
}

trait Drawable {
    fn draw(&self);
}

type Handler = Box<dyn Fn()>;

const MAX_COUNT: usize = 100;

static APP_NAME: &str = "test";

macro_rules! vec_of {
    ($($x:expr),*) => { vec![$($x),*] };
}

fn main() {
    let w = Widget::new("World".into());
    println!("{}", w.greet());
}
"#;

        let result = parse_file("test.rs", source).unwrap();
        assert_eq!(result.language, "rust");
        assert_eq!(result.path, "test.rs");

        // Verify extracted symbols
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        let kinds: Vec<SymbolKind> = result.symbols.iter().map(|s| s.kind).collect();

        assert!(names.contains(&"Widget"), "should extract struct Widget");
        assert!(names.contains(&"Color"), "should extract enum Color");
        assert!(names.contains(&"Drawable"), "should extract trait Drawable");
        assert!(
            names.contains(&"Handler"),
            "should extract type alias Handler"
        );
        assert!(
            names.contains(&"MAX_COUNT"),
            "should extract const MAX_COUNT"
        );
        assert!(
            names.contains(&"APP_NAME"),
            "should extract static APP_NAME"
        );
        assert!(names.contains(&"main"), "should extract fn main");
        assert!(names.contains(&"vec_of"), "should extract macro vec_of");

        // Check that impl block is captured
        assert!(
            kinds.contains(&SymbolKind::Impl),
            "should extract impl block"
        );
    }

    #[test]
    fn test_parse_typescript() {
        let source = r#"
import { something } from "./module";

interface User {
    name: string;
    age: number;
}

type Callback = (x: number) => void;

enum Direction {
    Up,
    Down,
}

class Greeter {
    greeting: string;

    constructor(message: string) {
        this.greeting = message;
    }

    greet(): string {
        return "Hello, " + this.greeting;
    }
}

function helper(): void {
    console.log("ok");
}
"#;
        let result = parse_file("test.ts", source).unwrap();
        assert_eq!(result.language, "typescript");

        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"User"), "should extract interface User");
        assert!(
            names.contains(&"Callback"),
            "should extract type alias Callback"
        );
        assert!(
            names.contains(&"Direction"),
            "should extract enum Direction"
        );
        assert!(names.contains(&"Greeter"), "should extract class Greeter");
        assert!(names.contains(&"helper"), "should extract function helper");
    }

    #[test]
    fn test_parse_python() {
        let source = r#"
import os
from typing import Optional

class Animal:
    def __init__(self, name: str):
        self.name = name

    def speak(self) -> str:
        return "..."

def create_animal(name: str) -> Animal:
    return Animal(name)
"#;
        let result = parse_file("test.py", source).unwrap();
        assert_eq!(result.language, "python");

        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Animal"), "should extract class Animal");
        assert!(
            names.contains(&"create_animal"),
            "should extract function create_animal"
        );
    }

    #[test]
    fn test_parse_go() {
        let source = r#"
package main

import "fmt"

type Person struct {
    Name string
    Age  int
}

func (p *Person) Greet() string {
    return "Hello, " + p.Name
}

func main() {
    fmt.Println("hello")
}
"#;
        let result = parse_file("test.go", source).unwrap();
        assert_eq!(result.language, "go");

        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Person"), "should extract type Person");
        assert!(names.contains(&"main"), "should extract func main");
        assert!(names.contains(&"Greet"), "should extract method Greet");
    }

    #[test]
    fn test_parse_empty_file() {
        let result = parse_file("empty.rs", "").unwrap();
        assert!(result.symbols.is_empty());
    }

    #[test]
    fn test_parse_unsupported_language() {
        let result = parse_file("readme.md", "# Hello");
        assert!(result.is_err());
    }
}
