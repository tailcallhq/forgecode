use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ModelConfig;

/// Range of accepted speed-dial slot numbers.
pub const SPEED_DIAL_MIN_SLOT: u8 = 1;
pub const SPEED_DIAL_MAX_SLOT: u8 = 9;

/// Returns `true` when `slot` is a valid speed-dial slot (1..=9).
pub fn is_valid_speed_dial_slot(slot: u8) -> bool {
    (SPEED_DIAL_MIN_SLOT..=SPEED_DIAL_MAX_SLOT).contains(&slot)
}

/// A single speed-dial binding pairing a provider and model to a slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, fake::Dummy)]
pub struct SpeedDialEntry {
    pub provider_id: String,
    pub model_id: String,
}

impl SpeedDialEntry {
    pub fn new(provider_id: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self { provider_id: provider_id.into(), model_id: model_id.into() }
    }
}

impl From<ModelConfig> for SpeedDialEntry {
    fn from(value: ModelConfig) -> Self {
        Self { provider_id: value.provider_id, model_id: value.model_id }
    }
}

impl From<SpeedDialEntry> for ModelConfig {
    fn from(value: SpeedDialEntry) -> Self {
        Self { provider_id: value.provider_id, model_id: value.model_id }
    }
}

/// Persistent speed-dial bindings keyed by slot (1..=9).
///
/// Slots use a `BTreeMap` so that iteration order is stable when listing slots
/// in `:info` or when serialising to TOML. Entries are keyed by `String` at the
/// TOML level so that the table uses friendly headings like
/// `[speed_dial.1]` rather than binary integer keys.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize, JsonSchema, fake::Dummy)]
#[serde(transparent)]
pub struct SpeedDial {
    slots: BTreeMap<String, SpeedDialEntry>,
}

impl SpeedDial {
    /// Returns an empty speed-dial map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when there are no configured slots.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Returns an iterator over `(slot, entry)` pairs, in ascending slot order.
    ///
    /// Malformed slot keys (non-numeric, out of range) are skipped so callers
    /// can treat the result as strictly valid 1..=9 slots.
    pub fn iter(&self) -> impl Iterator<Item = (u8, &SpeedDialEntry)> + '_ {
        let mut entries: Vec<(u8, &SpeedDialEntry)> = self
            .slots
            .iter()
            .filter_map(|(k, v)| k.parse::<u8>().ok().map(|slot| (slot, v)))
            .filter(|(slot, _)| is_valid_speed_dial_slot(*slot))
            .collect();
        entries.sort_by_key(|(slot, _)| *slot);
        entries.into_iter()
    }

    /// Returns the binding for `slot`, or `None` when the slot is empty or
    /// out of range.
    pub fn get(&self, slot: u8) -> Option<&SpeedDialEntry> {
        if !is_valid_speed_dial_slot(slot) {
            return None;
        }
        self.slots.get(&slot.to_string())
    }

    /// Inserts or replaces the binding for `slot`.
    ///
    /// Returns an error if `slot` is outside 1..=9.
    pub fn set(&mut self, slot: u8, entry: SpeedDialEntry) -> Result<(), SpeedDialError> {
        if !is_valid_speed_dial_slot(slot) {
            return Err(SpeedDialError::InvalidSlot(slot));
        }
        self.slots.insert(slot.to_string(), entry);
        Ok(())
    }

    /// Removes the binding for `slot`. Returns the removed entry when present.
    pub fn clear(&mut self, slot: u8) -> Option<SpeedDialEntry> {
        if !is_valid_speed_dial_slot(slot) {
            return None;
        }
        self.slots.remove(&slot.to_string())
    }
}

/// Errors produced by speed-dial operations.
#[derive(Debug, thiserror::Error)]
pub enum SpeedDialError {
    #[error("Speed-dial slot {0} is out of range (allowed: 1..=9)")]
    InvalidSlot(u8),
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_speed_dial_default_is_empty() {
        let fixture = SpeedDial::default();
        assert!(fixture.is_empty());
    }

    #[test]
    fn test_speed_dial_set_and_get() {
        let mut fixture = SpeedDial::new();
        fixture
            .set(1, SpeedDialEntry::new("anthropic", "claude-opus"))
            .unwrap();
        let actual = fixture.get(1).unwrap().clone();
        let expected = SpeedDialEntry::new("anthropic", "claude-opus");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_speed_dial_rejects_zero_slot() {
        let mut fixture = SpeedDial::new();
        let err = fixture
            .set(0, SpeedDialEntry::new("anthropic", "claude-opus"))
            .unwrap_err();
        assert!(matches!(err, SpeedDialError::InvalidSlot(0)));
    }

    #[test]
    fn test_speed_dial_rejects_ten_slot() {
        let mut fixture = SpeedDial::new();
        let err = fixture
            .set(10, SpeedDialEntry::new("anthropic", "claude-opus"))
            .unwrap_err();
        assert!(matches!(err, SpeedDialError::InvalidSlot(10)));
    }

    #[test]
    fn test_speed_dial_clear_removes_entry() {
        let mut fixture = SpeedDial::new();
        fixture
            .set(3, SpeedDialEntry::new("openai", "gpt-5"))
            .unwrap();
        let removed = fixture.clear(3).unwrap();
        assert_eq!(removed, SpeedDialEntry::new("openai", "gpt-5"));
        assert_eq!(fixture.get(3), None);
    }

    #[test]
    fn test_speed_dial_iter_is_sorted() {
        let mut fixture = SpeedDial::new();
        fixture
            .set(7, SpeedDialEntry::new("p1", "m1"))
            .unwrap();
        fixture
            .set(2, SpeedDialEntry::new("p2", "m2"))
            .unwrap();
        fixture
            .set(5, SpeedDialEntry::new("p3", "m3"))
            .unwrap();

        let actual: Vec<u8> = fixture.iter().map(|(s, _)| s).collect();
        let expected = vec![2u8, 5, 7];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_speed_dial_toml_round_trip() {
        let mut fixture = SpeedDial::new();
        fixture
            .set(1, SpeedDialEntry::new("anthropic", "claude-opus-4"))
            .unwrap();
        fixture
            .set(3, SpeedDialEntry::new("openai", "gpt-5.4"))
            .unwrap();

        let toml = toml_edit::ser::to_string_pretty(&fixture).unwrap();
        let decoded: SpeedDial = toml_edit::de::from_str(&toml).unwrap();
        assert_eq!(decoded, fixture);
    }

    #[test]
    fn test_speed_dial_ignores_invalid_slot_keys_in_iter() {
        let mut fixture = SpeedDial::default();
        // Forcefully insert an invalid key (simulating hand-edited TOML).
        fixture
            .slots
            .insert("bogus".to_string(), SpeedDialEntry::new("p", "m"));
        fixture
            .slots
            .insert("0".to_string(), SpeedDialEntry::new("p0", "m0"));
        fixture
            .slots
            .insert("11".to_string(), SpeedDialEntry::new("p11", "m11"));
        fixture
            .slots
            .insert("4".to_string(), SpeedDialEntry::new("p4", "m4"));

        let actual: Vec<u8> = fixture.iter().map(|(s, _)| s).collect();
        let expected = vec![4u8];
        assert_eq!(actual, expected);
    }
}
