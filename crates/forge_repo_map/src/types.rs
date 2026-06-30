use serde::{Deserialize, Serialize};

/// A file entry returned by the repo walker — path + size info.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
}

/// Kinds of top-level symbols that can be extracted from source files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    TypeAlias,
    Interface,
    Class,
    Const,
    Static,
    Macro,
    Impl,
    Module,
    Import,
    Variable,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "fn",
            SymbolKind::Method => "method",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::TypeAlias => "type",
            SymbolKind::Interface => "interface",
            SymbolKind::Class => "class",
            SymbolKind::Const => "const",
            SymbolKind::Static => "static",
            SymbolKind::Macro => "macro",
            SymbolKind::Impl => "impl",
            SymbolKind::Module => "mod",
            SymbolKind::Import => "import",
            SymbolKind::Variable => "var",
        }
    }
}

/// A single symbol extracted from a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub line: usize,
}

/// All symbols found within a single source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSymbols {
    pub path: String,
    pub language: String,
    pub symbols: Vec<Symbol>,
}

/// The complete repository structure map.
///
/// Contains an overview of every parsed file and its top-level symbols,
/// forming a compact index of the codebase that an agent can use as context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoMap {
    /// Root directory that was scanned.
    pub root: String,
    /// Per-file symbol listings.
    pub files: Vec<FileSymbols>,
    /// Total number of files parsed.
    pub total_files: usize,
    /// Total number of symbols extracted.
    pub total_symbols: usize,
}

impl RepoMap {
    /// Render the map as a compact human-readable text block.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("repo-map: {}\n", self.root));
        out.push_str(&format!(
            "{} files, {} symbols\n\n",
            self.total_files, self.total_symbols
        ));

        for file in &self.files {
            out.push_str(&format!("{}  [{}]\n", file.path, file.language));
            for sym in &file.symbols {
                out.push_str(&format!("  {:>8}  {}\n", sym.kind.as_str(), sym.name));
            }
            out.push('\n');
        }

        out
    }

    /// Render the map as a JSON string.
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}
