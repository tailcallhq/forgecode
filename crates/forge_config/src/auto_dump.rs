use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The output format used when auto-dumping a conversation on task completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, fake::Dummy)]
#[serde(rename_all = "snake_case")]
pub enum AutoDumpFormat {
    /// Dump as a JSON file
    Json,
    /// Dump as an HTML file
    Html,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_auto_dump_format_variants() {
        assert_eq!(AutoDumpFormat::Json, AutoDumpFormat::Json);
        assert_eq!(AutoDumpFormat::Html, AutoDumpFormat::Html);
    }
}
