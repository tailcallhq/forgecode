use base64::Engine;
use derive_getters::Getters;
use serde::{Deserialize, Serialize};

/// Represents a document file (e.g., PDF) with base64-encoded content.
///
/// Similar to `Image` but specifically for non-image binary files that
/// providers can process natively (e.g., Anthropic's document content blocks,
/// Google's inline data, OpenAI's file content parts).
#[derive(Default, Clone, Debug, Serialize, Deserialize, Getters, PartialEq, Eq, Hash)]
pub struct Document {
    data: String,
    mime_type: String,
    filename: Option<String>,
}

impl Document {
    /// Creates a new `Document` from raw bytes, encoding them as base64.
    pub fn new_bytes(content: Vec<u8>, mime_type: impl ToString) -> Self {
        let mime_type = mime_type.to_string();
        let base64_encoded = base64::engine::general_purpose::STANDARD.encode(&content);
        Self::new_base64(base64_encoded, mime_type)
    }

    /// Creates a new `Document` from an already base64-encoded string.
    pub fn new_base64(base64_encoded: String, mime_type: impl ToString) -> Self {
        let mime_type = mime_type.to_string();
        Self { data: base64_encoded, mime_type, filename: None }
    }

    /// Returns the raw base64 data without any prefix.
    pub fn base64_data(&self) -> &str {
        &self.data
    }

    /// Sets the optional filename for the document.
    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }
}
