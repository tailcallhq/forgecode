use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A unique identifier for a single user-initiated input from the UI.
///
/// Generated once per incoming `ChatRequest` and stored on the `Conversation`.
/// Every file snapshot taken while processing that request is stamped with this
/// ID, allowing all changes from a single prompt to be grouped and undone
/// together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserInputId(Uuid);

impl UserInputId {
    /// Generate a new random `UserInputId`.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Parse a `UserInputId` from a UUID string.
    ///
    /// # Errors
    /// Returns an error if the string is not a valid UUID.
    pub fn parse(value: impl AsRef<str>) -> anyhow::Result<Self> {
        Ok(Self(Uuid::parse_str(value.as_ref())?))
    }

    /// Return the underlying UUID.
    pub fn into_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for UserInputId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for UserInputId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_user_input_id_is_unique() {
        let id1 = UserInputId::new();
        let id2 = UserInputId::new();

        assert_ne!(id1, id2);
    }

    #[test]
    fn test_user_input_id_serde_round_trip() {
        let fixture = UserInputId::new();

        let serialized = serde_json::to_string(&fixture).unwrap();
        let actual: UserInputId = serde_json::from_str(&serialized).unwrap();

        assert_eq!(actual, fixture);
    }
}
