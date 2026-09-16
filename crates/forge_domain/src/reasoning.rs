use derive_setters::Setters;
use serde::{Deserialize, Serialize};

/// Represents a reasoning detail that may be included in the response
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Default, Setters)]
#[setters(into)]
pub struct ReasoningDetail {
    pub text: Option<String>,
    pub signature: Option<String>,
    pub data: Option<String>,
    pub id: Option<String>,
    pub format: Option<String>,
    pub index: Option<i32>,
    pub type_of: Option<String>,
}

/// Type alias for partial reasoning (used in streaming)
pub type ReasoningPart = ReasoningDetail;

/// Type alias for complete reasoning
pub type ReasoningFull = ReasoningDetail;

#[derive(Clone, Debug, PartialEq)]
pub enum Reasoning {
    Part(Vec<ReasoningPart>),
    Full(Vec<ReasoningFull>),
}

impl Reasoning {
    pub fn as_partial(&self) -> Option<&Vec<ReasoningPart>> {
        match self {
            Reasoning::Part(parts) => Some(parts),
            Reasoning::Full(_) => None,
        }
    }

    pub fn as_full(&self) -> Option<&Vec<ReasoningFull>> {
        match self {
            Reasoning::Part(_) => None,
            Reasoning::Full(full) => Some(full),
        }
    }

    pub fn from_parts(parts: Vec<Vec<ReasoningPart>>) -> Vec<ReasoningFull> {
        // Flatten all parts and group by type
        let mut grouped: std::collections::HashMap<Option<String>, Vec<ReasoningPart>> =
            std::collections::HashMap::new();

        for part_vec in parts {
            for part in part_vec {
                grouped.entry(part.type_of.clone()).or_default().push(part);
            }
        }

        grouped
            .into_iter()
            .filter_map(|(type_key, parts)| {
                // Merge text from all parts
                let text = parts
                    .iter()
                    .filter_map(|p| p.text.as_deref())
                    .collect::<String>();

                // Get first non-empty value for each field
                let signature = parts.iter().find_map(|p| {
                    p.signature
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                });
                let data = parts.iter().find_map(|p| {
                    p.data
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                });
                let id = parts
                    .iter()
                    .find_map(|p| p.id.as_deref().filter(|s| !s.is_empty()).map(String::from));
                let format = parts.iter().find_map(|p| {
                    p.format
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                });
                let index = parts.iter().find_map(|p| p.index);

                // Only include if at least one field has data
                if text.is_empty() && signature.is_none() && data.is_none() {
                    return None;
                }

                Some(ReasoningFull {
                    text: (!text.is_empty()).then_some(text),
                    signature,
                    data,
                    id,
                    format,
                    index,
                    type_of: type_key,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reasoning_detail_from_parts_groups_by_type() {
        // Create a fixture with parts of different types across streaming
        // deltas
        let fixture = vec![
            // First delta: reasoning.text
            vec![ReasoningPart {
                type_of: Some("reasoning.text".to_string()),
                text: Some("Part 1 ".to_string()),
                ..Default::default()
            }],
            // Second delta: reasoning.text continues
            vec![ReasoningPart {
                type_of: Some("reasoning.text".to_string()),
                text: Some("Part 2".to_string()),
                ..Default::default()
            }],
            // Third delta: reasoning.encrypted appears
            vec![ReasoningPart {
                type_of: Some("reasoning.encrypted".to_string()),
                data: Some("encrypted_data".to_string()),
                id: Some("tool_call_id".to_string()),
                ..Default::default()
            }],
        ];

        // Execute the function to get the actual result
        let actual = Reasoning::from_parts(fixture);

        // Both types should be separate entries
        assert_eq!(actual.len(), 2);

        // Find each type
        let text_entry = actual
            .iter()
            .find(|r| r.type_of == Some("reasoning.text".to_string()))
            .expect("Should have reasoning.text entry");
        let encrypted_entry = actual
            .iter()
            .find(|r| r.type_of == Some("reasoning.encrypted".to_string()))
            .expect("Should have reasoning.encrypted entry");

        // Verify text entry has merged text
        assert_eq!(text_entry.text, Some("Part 1 Part 2".to_string()));

        // Verify encrypted entry has data and id
        assert_eq!(encrypted_entry.data, Some("encrypted_data".to_string()));
        assert_eq!(encrypted_entry.id, Some("tool_call_id".to_string()));
    }

    #[test]
    fn test_reasoning_detail_from_parts_with_different_lengths() {
        // Create a fixture with different types to test grouping
        let fixture = vec![
            vec![
                ReasoningPart {
                    type_of: Some("type1".to_string()),
                    text: Some("a-text".to_string()),
                    signature: Some("a-sig".to_string()),
                    ..Default::default()
                },
                ReasoningPart {
                    type_of: Some("type2".to_string()),
                    text: Some("b-text".to_string()),
                    signature: Some("b-sig".to_string()),
                    ..Default::default()
                },
            ],
            vec![ReasoningPart {
                type_of: Some("type1".to_string()),
                text: Some("c-text".to_string()),
                signature: Some("c-sig".to_string()),
                ..Default::default()
            }],
            vec![
                ReasoningPart {
                    type_of: Some("type1".to_string()),
                    text: Some("d-text".to_string()),
                    signature: Some("d-sig".to_string()),
                    ..Default::default()
                },
                ReasoningPart {
                    type_of: Some("type2".to_string()),
                    text: Some("e-text".to_string()),
                    signature: Some("e-sig".to_string()),
                    ..Default::default()
                },
                ReasoningPart {
                    type_of: Some("type3".to_string()),
                    text: Some("f-text".to_string()),
                    signature: Some("f-sig".to_string()),
                    ..Default::default()
                },
            ],
        ];

        // Execute the function to get the actual result
        let mut actual = Reasoning::from_parts(fixture);
        actual.sort_by(|a, b| a.type_of.cmp(&b.type_of)); // Sort by type for consistent ordering

        // Define the expected result - now grouped by type
        let mut expected = vec![
            // type1: a + c + d (text merged, signature is first non-empty)
            ReasoningFull {
                type_of: Some("type1".to_string()),
                text: Some("a-textc-textd-text".to_string()),
                signature: Some("a-sig".to_string()), // First non-empty signature
                ..Default::default()
            },
            // type2: b + e (text merged, signature is first non-empty)
            ReasoningFull {
                type_of: Some("type2".to_string()),
                text: Some("b-texte-text".to_string()),
                signature: Some("b-sig".to_string()), // First non-empty signature
                ..Default::default()
            },
            // type3: f
            ReasoningFull {
                type_of: Some("type3".to_string()),
                text: Some("f-text".to_string()),
                signature: Some("f-sig".to_string()),
                ..Default::default()
            },
        ];
        expected.sort_by(|a, b| a.type_of.cmp(&b.type_of)); // Sort expected for consistent comparison

        // Assert that the actual result matches the expected result
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_reasoning_detail_from_parts_with_none_values() {
        // Create a fixture with some None values
        let fixture = vec![
            vec![ReasoningPart {
                text: Some("a-text".to_string()),
                signature: None,
                ..Default::default()
            }],
            vec![ReasoningPart {
                text: None,
                signature: Some("b-sig".to_string()),
                ..Default::default()
            }],
            vec![ReasoningPart {
                text: Some("b-test".to_string()),
                signature: None,
                ..Default::default()
            }],
        ];

        // Execute the function to get the actual result
        let actual = Reasoning::from_parts(fixture);

        // Define the expected result
        let expected = vec![ReasoningFull {
            text: Some("a-textb-test".to_string()),
            signature: Some("b-sig".to_string()),
            ..Default::default()
        }];

        // Assert that the actual result matches the expected result
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_reasoning_detail_from_empty_parts() {
        // Empty fixture
        let fixture: Vec<Vec<ReasoningPart>> = vec![];

        // Execute the function to get the actual result
        let actual = Reasoning::from_parts(fixture);

        // Define the expected result - should be an empty vector
        let expected: Vec<ReasoningFull> = vec![];

        // Assert that the actual result matches the expected result
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_reasoning_detail_from_parts_keeps_partial_reasoning() {
        let fixture = vec![
            vec![
                ReasoningPart {
                    type_of: Some("reasoning.text".to_string()),
                    text: Some("text-only".to_string()),
                    signature: None,
                    ..Default::default()
                },
                ReasoningPart {
                    type_of: Some("reasoning.encrypted".to_string()),
                    text: Some("complete-text".to_string()),
                    signature: Some("complete-sig".to_string()),
                    ..Default::default()
                },
            ],
            vec![
                ReasoningPart {
                    type_of: Some("reasoning.text".to_string()),
                    text: Some("more-text".to_string()),
                    signature: None,
                    ..Default::default()
                },
                ReasoningPart {
                    type_of: Some("reasoning.encrypted".to_string()),
                    text: Some("more-text2".to_string()),
                    signature: Some("more-sig".to_string()),
                    ..Default::default()
                },
            ],
        ];

        let mut actual = Reasoning::from_parts(fixture);
        actual.sort_by(|a, b| a.type_of.cmp(&b.type_of)); // Sort by type for consistent ordering

        // Now grouped by type: reasoning.text and reasoning.encrypted are
        // separate entries
        let mut expected = vec![
            ReasoningFull {
                type_of: Some("reasoning.text".to_string()),
                text: Some("text-onlymore-text".to_string()),
                signature: None, // No signature in reasoning.text type
                ..Default::default()
            },
            ReasoningFull {
                type_of: Some("reasoning.encrypted".to_string()),
                text: Some("complete-textmore-text2".to_string()),
                signature: Some("complete-sig".to_string()), // First non-empty signature
                ..Default::default()
            },
        ];
        expected.sort_by(|a, b| a.type_of.cmp(&b.type_of)); // Sort expected as well for consistent comparison
        assert_eq!(actual, expected);
    }
}
