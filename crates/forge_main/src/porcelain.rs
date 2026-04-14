use std::collections::HashMap;
use std::fmt;

use indexmap::IndexSet;

use crate::display_constants::headers;
use crate::info::{Info, Section};

/// Porcelain is an intermediate representation that converts Info into a flat,
/// tabular structure suitable for machine-readable output.
///
/// Structure: Vec<(String, Vec<Option<String>>)>
/// - First element: Section name
/// - Second element: Vec of Option<String> pairs where:
///   - Index 0, 2, 4... are keys
///   - Index 1, 3, 5... are values
///   - None = missing value
#[derive(Debug, PartialEq, Clone)]
pub struct Porcelain(Vec<Vec<Option<String>>>);

impl Porcelain {
    /// Creates a new empty Porcelain instance
    pub fn new() -> Self {
        Porcelain(Vec::new())
    }

    /// Skips the first n rows
    pub fn skip(self, n: usize) -> Self {
        Porcelain(self.0.into_iter().skip(n).collect())
    }

    #[allow(unused)]
    pub fn drop_col(self, c: usize) -> Self {
        Porcelain(
            self.0
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .enumerate()
                        .filter_map(|(i, col)| if i == c { None } else { Some(col) })
                        .collect()
                })
                .collect(),
        )
    }

    /// Drops multiple columns at once
    ///
    /// # Arguments
    /// * `cols` - A slice of column indices to drop
    ///
    /// # Example
    /// ```rust,ignore
    /// let porcelain = Porcelain::new();
    /// let result = porcelain.drop_cols(&[0, 2, 4]);
    /// ```
    #[allow(unused)]
    pub fn drop_cols(self, cols: &[usize]) -> Self {
        Porcelain(
            self.0
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .enumerate()
                        .filter_map(|(i, col)| if cols.contains(&i) { None } else { Some(col) })
                        .collect()
                })
                .collect(),
        )
    }

    /// Maps a function over all cells in the specified column
    pub fn map_col<F>(self, c: usize, f: F) -> Self
    where
        F: Fn(Option<String>) -> Option<String>,
    {
        Porcelain(
            self.0
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .enumerate()
                        .map(|(i, col)| if i == c { f(col) } else { col })
                        .collect()
                })
                .collect(),
        )
    }

    /// Truncates the specified column to a maximum number of characters,
    /// appending "..." after the kept characters if truncated. The "..." is
    /// not counted toward `max_len`.
    pub fn truncate(self, c: usize, max_len: usize) -> Self {
        self.map_col(c, |col| {
            col.map(|value| {
                if value.chars().count() > max_len {
                    let truncated: String = value.chars().take(max_len).collect();
                    format!("{}...", truncated)
                } else {
                    value
                }
            })
        })
    }

    pub fn swap_cols(self, col1: usize, col2: usize) -> Self {
        Porcelain(
            self.0
                .into_iter()
                .map(|mut row| {
                    if row.len() > col1.max(col2) {
                        row.swap(col1, col2);
                    }
                    row
                })
                .collect(),
        )
    }

    /// Sorts rows based on multiple columns
    ///
    /// Preserves the header row (first row) and sorts all subsequent rows.
    /// Columns are sorted in the order specified in `cols`. None values are
    /// sorted after Some values.
    ///
    /// # Arguments
    /// * `cols` - Column indices to sort by, in order of precedence
    ///
    /// # Example
    /// ```ignore
    /// porcelain.sort_by(&[1, 2]) // Sort by column 1, then by column 2
    /// ```
    pub fn sort_by(self, cols: &[usize]) -> Self {
        if self.0.is_empty() || cols.is_empty() {
            return self;
        }

        let mut rows = self.0;
        let header = if !rows.is_empty() {
            Some(rows.remove(0))
        } else {
            None
        };

        rows.sort_by(|a, b| {
            for &col in cols {
                let a_val = a.get(col);
                let b_val = b.get(col);

                let ordering = match (a_val, b_val) {
                    (Some(Some(a)), Some(Some(b))) => a.cmp(b),
                    (Some(Some(_)), Some(None)) => std::cmp::Ordering::Less,
                    (Some(None), Some(Some(_))) => std::cmp::Ordering::Greater,
                    (Some(None), Some(None)) => std::cmp::Ordering::Equal,
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                };

                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
                }
            }
            std::cmp::Ordering::Equal
        });

        if let Some(header) = header {
            rows.insert(0, header);
        }

        Porcelain(rows)
    }

    /// Applies case transformation to specified columns
    ///
    /// # Arguments
    /// * `cols` - Column indices to transform
    /// * `case` - The case to apply (e.g., Case::Snake, Case::Kebab)
    ///
    /// # Example
    /// ```ignore
    /// use convert_case::Case;
    ///
    /// porcelain.to_case(&[0, 1], Case::Snake)
    /// ```
    #[allow(clippy::wrong_self_convention)]
    pub fn to_case(self, cols: &[usize], case: convert_case::Case) -> Self {
        use convert_case::Casing;

        Porcelain(
            self.0
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .enumerate()
                        .map(|(i, col)| {
                            if cols.contains(&i) {
                                col.map(|v| v.to_case(case))
                            } else {
                                col
                            }
                        })
                        .collect()
                })
                .collect(),
        )
    }

    #[allow(unused)]
    pub fn into_body(self) -> Vec<Vec<Option<String>>> {
        // Skip headers and return
        self.0.into_iter().skip(1).collect()
    }

    #[allow(unused)]
    pub fn into_rows(self) -> Vec<Vec<Option<String>>> {
        self.0
    }

    /// Converts from wide format to long format.
    ///
    /// Transforms entity-centric rows (wide format with many columns) into
    /// field-centric rows (long format with three columns: entity_id,
    /// field_name, field_value). This is also known as unpivoting or
    /// melting in data transformation terminology.
    ///
    /// # Example
    /// Input (wide format):
    /// ```text
    /// Headers: [$ID, version, shell, id, title, model]
    /// Row 1:   [env, 0.1.0,   zsh,   None, None, None]
    /// Row 2:   [conversation, None, None, 000-000-000, make agents great again, None]
    /// ```
    ///
    /// Output (long format):
    /// ```text
    /// Headers: [$ID, field, value]
    /// Row 1:   [env, version, 0.1.0]
    /// Row 2:   [env, shell, zsh]
    /// Row 3:   [conversation, id, 000-000-000]
    /// Row 4:   [conversation, title, make agents great again]
    /// ```
    /// Converts all headers (first row) to uppercase
    pub fn uppercase_headers(self) -> Self {
        if self.0.is_empty() {
            return self;
        }

        let mut rows = self.0;
        if let Some(header_row) = rows.first_mut() {
            *header_row = header_row
                .iter()
                .map(|col| col.as_ref().map(|s| s.to_uppercase()))
                .collect();
        }

        Porcelain(rows)
    }

    /// Sets the headers (first row) of the Porcelain structure
    ///
    /// Replaces the first row with the provided headers. If the Porcelain is
    /// empty, creates a new header row. If an empty iterator is provided,
    /// returns self unchanged.
    ///
    /// # Arguments
    /// * `headers` - An iterator of header names to set
    pub fn set_headers<I, S>(self, headers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let new_headers: Vec<Option<String>> =
            headers.into_iter().map(|h| Some(h.into())).collect();

        if new_headers.is_empty() {
            return self;
        }

        let mut rows = self.0;

        if rows.is_empty() {
            // If Porcelain is empty, just add the headers
            rows.push(new_headers);
        } else {
            // Replace the first row with new headers
            if let Some(first) = rows.first_mut() {
                *first = new_headers;
            }
        }

        Porcelain(rows)
    }

    pub fn into_long(self) -> Self {
        if self.0.is_empty() {
            return self;
        }

        let Some(headers) = self.0.first() else {
            return self;
        };
        let data_rows = self.0.get(1..).unwrap_or(&[]);

        if data_rows.is_empty() || headers.is_empty() {
            return self;
        }

        // Create new headers: [$ID, $FIELD, $VALUE]
        let new_headers = vec![
            headers
                .first()
                .cloned()
                .unwrap_or(Some(headers::ID.to_string())),
            Some(headers::FIELD.to_string()),
            Some(headers::VALUE.to_string()),
        ];

        // Create new rows: one row per non-None field for each entity
        let mut new_rows = Vec::new();

        for data_row in data_rows {
            // Get the entity ID (first column value)
            let entity_id = data_row.first().and_then(|v| v.clone());

            // For each field in this entity (excluding $ID column)
            for (i, value) in data_row.iter().enumerate().skip(1) {
                if let Some(value) = value {
                    let field_name = headers.get(i).and_then(|h| h.clone());
                    new_rows.push(vec![entity_id.clone(), field_name, Some(value.to_owned())]);
                }
            }
        }

        let mut result = vec![new_headers];
        result.extend(new_rows);

        Porcelain(result)
    }
}

impl fmt::Display for Porcelain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return Ok(());
        }

        // Calculate the maximum width for each column
        let num_cols = self.0.iter().map(|row| row.len()).max().unwrap_or(0);
        let mut col_widths = vec![0; num_cols];

        for row in &self.0 {
            for (i, cell) in row.iter().enumerate() {
                let width = cell.as_ref().map(|s| s.len()).unwrap_or(0);
                if let Some(col_width) = col_widths.get_mut(i) {
                    *col_width = (*col_width).max(width);
                }
            }
        }

        // Format each row
        let mut lines = Vec::new();
        for row in &self.0 {
            let mut line = String::new();
            for (i, cell) in row.iter().enumerate() {
                let content = cell.as_ref().map(|s| s.as_str()).unwrap_or("");

                if i == row.len() - 1 {
                    // Last column: no padding
                    line.push_str(content);
                } else {
                    // Pad to column width
                    let width = col_widths.get(i).copied().unwrap_or(0);
                    line.push_str(&format!("{:<width$}", content, width = width));
                    line.push_str("  ");
                }
            }
            lines.push(line);
        }

        write!(f, "{}", lines.join("\n"))
    }
}

impl Default for Porcelain {
    fn default() -> Self {
        Self::new()
    }
}

/// Converts Info to Porcelain representation
/// Handles both cases:
/// - Info with titles: Each title becomes a row with its associated items
/// - Info without titles: Each item becomes its own row
impl From<Info> for Porcelain {
    fn from(info: Info) -> Self {
        Porcelain::from(&info)
    }
}

/// Converts Info reference to Porcelain representation
impl From<&Info> for Porcelain {
    fn from(info: &Info) -> Self {
        let mut rows = Vec::new();
        let mut cells = HashMap::<String, Vec<String>>::new();
        let mut in_row = false;
        // Extract all unique keys
        let mut keys = IndexSet::new();
        // Track count of unnamed values separately
        let mut value_counter = 1;
        let mut last_key: Option<String> = None;

        for section in info.sections() {
            match section {
                Section::Title(title) => {
                    if in_row {
                        rows.push(cells.clone());
                        cells = HashMap::new();
                        value_counter = 1;
                    }

                    in_row = true;
                    cells.insert(headers::ID.to_owned(), vec![title.to_owned()]);
                    keys.insert(headers::ID.to_owned());
                }
                Section::Items(key, value) => {
                    let key = if let Some(key) = key.as_ref().cloned().or(last_key) {
                        key
                    } else {
                        let default_key = format!("{}_{}", headers::VALUE, value_counter);
                        value_counter += 1;
                        default_key
                    };
                    last_key = Some(key.clone());

                    cells
                        .entry(key.to_string())
                        .or_default()
                        .push(value.clone());
                    keys.insert(key.to_string());
                }
            }
        }

        if in_row {
            rows.push(cells.clone());
        }

        // Insert Headers
        let mut data = vec![
            keys.iter()
                .map(|head| Some((*head).to_owned()))
                .collect::<Vec<_>>(),
        ];

        // Insert Rows
        data.extend(rows.iter().map(|rows| {
            keys.iter()
                .map(|key| {
                    rows.get(key).and_then(|value| {
                        if value.is_empty() {
                            None
                        } else {
                            Some(value.join(", "))
                        }
                    })
                })
                .collect::<Vec<Option<String>>>()
        }));
        Porcelain(data)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_from_info() {
        let info = Info::new()
            .add_title("user1")
            .add_key_value("name", "Alice")
            .add_key_value("age", "30")
            .add_title("user2")
            .add_key_value("name", "Bob")
            .add_key_value("age", "25");

        let actual = Porcelain::from(info).into_body();
        let expected = vec![
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("30".into()),
            ],
            vec![Some("user2".into()), Some("Bob".into()), Some("25".into())],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_from_unordered_info() {
        let info = Info::new()
            .add_title("user1")
            .add_key_value("name", "Alice")
            .add_key_value("age", "30")
            .add_title("user2")
            .add_key_value("age", "25")
            .add_key_value("name", "Bob");

        let actual = Porcelain::from(info).into_body();
        let expected = vec![
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("30".into()),
            ],
            vec![Some("user2".into()), Some("Bob".into()), Some("25".into())],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_drop_col() {
        let info = Porcelain(vec![
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("30".into()),
            ],
            vec![Some("user2".into()), Some("Bob".into()), Some("25".into())],
        ]);

        let actual = info.drop_col(1).into_rows();

        let expected = vec![
            vec![Some("user1".into()), Some("30".into())],
            vec![Some("user2".into()), Some("25".into())],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_drop_cols() {
        let fixture = Porcelain(vec![
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("30".into()),
                Some("Engineer".into()),
            ],
            vec![
                Some("user2".into()),
                Some("Bob".into()),
                Some("25".into()),
                Some("Designer".into()),
            ],
        ]);

        let actual = fixture.drop_cols(&[1, 3]).into_rows();

        let expected = vec![
            vec![Some("user1".into()), Some("30".into())],
            vec![Some("user2".into()), Some("25".into())],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_drop_cols_equivalent_to_chained() {
        // Test that drop_cols(&[0, 4, 5]) is equivalent to
        // drop_col(0).drop_col(3).drop_col(3)
        let fixture = Porcelain(vec![
            vec![
                Some("col0".into()),
                Some("col1".into()),
                Some("col2".into()),
                Some("col3".into()),
                Some("col4".into()),
                Some("col5".into()),
                Some("col6".into()),
            ],
            vec![
                Some("row2_0".into()),
                Some("row2_1".into()),
                Some("row2_2".into()),
                Some("row2_3".into()),
                Some("row2_4".into()),
                Some("row2_5".into()),
                Some("row2_6".into()),
            ],
        ]);

        let actual = fixture.clone().drop_cols(&[0, 4, 5]).into_rows();
        let expected = fixture.drop_col(0).drop_col(3).drop_col(3).into_rows();

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_map_col() {
        let info = Porcelain(vec![
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("30".into()),
            ],
            vec![Some("user2".into()), Some("Bob".into()), Some("25".into())],
        ]);

        let actual = info
            .map_col(1, |col| col.map(|v| v.to_uppercase()))
            .into_rows();

        let expected = vec![
            vec![
                Some("user1".into()),
                Some("ALICE".into()),
                Some("30".into()),
            ],
            vec![Some("user2".into()), Some("BOB".into()), Some("25".into())],
        ];

        assert_eq!(actual, expected)
    }
    #[test]
    fn test_truncate() {
        let info = Porcelain(vec![
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("very_long_name".into()),
            ],
            vec![
                Some("user2".into()),
                Some("Bob".into()),
                Some("short".into()),
            ],
        ]);

        // truncate(2, 5): "very_long_name" has 14 chars > 5, keep 5 then append "..."
        let actual = info.truncate(2, 5).into_rows();

        let expected = vec![
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("very_...".into()), // 5 chars kept + "..."
            ],
            vec![
                Some("user2".into()),
                Some("Bob".into()),
                Some("short".into()),
            ],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_truncate_unicode_multibyte_chars() {
        // Each emoji is 4 bytes but 1 char — byte-based truncation would misbehave here
        let fixture = Porcelain(vec![vec![
            Some("🦀🦀🦀🦀🦀🦀".into()), // 6 chars, 24 bytes
            Some("hi".into()),           // 2 chars, under limit
        ]]);
        let actual = fixture.truncate(0, 5).into_rows();
        let expected = vec![vec![
            Some("🦀🦀🦀🦀🦀...".into()), // 5 chars kept + "..."
            Some("hi".into()),
        ]];
        assert_eq!(actual, expected)
    }

    #[test]
    fn test_truncate_unicode_exactly_at_max_len() {
        // String is exactly max_len chars — should NOT be truncated
        let fixture = Porcelain(vec![vec![Some("héllo".into())]]); // 5 chars, 6 bytes
        let actual = fixture.truncate(0, 5).into_rows();
        let expected = vec![vec![Some("héllo".into())]];
        assert_eq!(actual, expected)
    }

    #[test]
    fn test_truncate_unicode_exceeds_max_len() {
        // 'é' is 2 bytes but 1 char — byte-based slicing would panic or cut wrong
        let fixture = Porcelain(vec![vec![Some("héllo world".into())]]); // 11 chars
        let actual = fixture.truncate(0, 8).into_rows();
        let expected = vec![vec![Some("héllo wo...".into())]]; // 8 chars kept + "..."
        assert_eq!(actual, expected)
    }

    #[test]
    fn test_swap_cols() {
        let info = Porcelain(vec![
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("30".into()),
            ],
            vec![Some("user2".into()), Some("Bob".into()), Some("25".into())],
        ]);

        let actual = info.swap_cols(0, 1).into_rows();

        let expected = vec![
            vec![
                Some("Alice".into()),
                Some("user1".into()),
                Some("30".into()),
            ],
            vec![Some("Bob".into()), Some("user2".into()), Some("25".into())],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_into_long() {
        let info = Info::new()
            .add_title("env")
            .add_key_value("version", "0.1.0")
            .add_key_value("shell", "zsh")
            .add_title("conversation")
            .add_key_value("id", "000-000-000")
            .add_key_value("title", "make agents great again")
            .add_title("agent")
            .add_key_value("id", "forge")
            .add_key_value("model", "sonnet-4");

        let actual = Porcelain::from(info).into_long();
        let expected = vec![
            vec![
                Some("env".into()),
                Some("version".into()),
                Some("0.1.0".into()),
            ],
            vec![Some("env".into()), Some("shell".into()), Some("zsh".into())],
            vec![
                Some("conversation".into()),
                Some("id".into()),
                Some("000-000-000".into()),
            ],
            vec![
                Some("conversation".into()),
                Some("title".into()),
                Some("make agents great again".into()),
            ],
            vec![
                Some("agent".into()),
                Some("id".into()),
                Some("forge".into()),
            ],
            vec![
                Some("agent".into()),
                Some("model".into()),
                Some("sonnet-4".into()),
            ],
        ];

        assert_eq!(actual.into_body(), expected)
    }

    #[test]
    fn test_from_info_single_col() {
        let info = Info::new()
            .add_title("T1")
            .add_value("a1")
            .add_value("b1")
            .add_title("T2")
            .add_value("a2")
            .add_value("b2")
            .add_title("T3")
            .add_value("a3")
            .add_value("b3");

        let actual = Porcelain::from(info).into_rows();

        let expected = vec![
            //
            vec![
                Some(headers::ID.into()),
                Some(format!("{}_1", headers::VALUE)),
            ],
            vec![Some("T1".into()), Some("a1, b1".into())],
            vec![Some("T2".into()), Some("a2, b2".into())],
            vec![Some("T3".into()), Some("a3, b3".into())],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_into_long_single_col() {
        let info = Info::new()
            .add_title("T1")
            .add_value("a1")
            .add_value("b1")
            .add_title("T2")
            .add_value("a2")
            .add_value("b2")
            .add_title("T3")
            .add_value("a3")
            .add_value("b3");

        let actual = Porcelain::from(info).into_long().into_rows();

        let expected = vec![
            vec![
                Some(headers::ID.into()),
                Some(headers::FIELD.into()),
                Some(headers::VALUE.into()),
            ],
            vec![
                Some("T1".into()),
                Some(format!("{}_1", headers::VALUE)),
                Some("a1, b1".into()),
            ],
            vec![
                Some("T2".into()),
                Some(format!("{}_1", headers::VALUE)),
                Some("a2, b2".into()),
            ],
            vec![
                Some("T3".into()),
                Some(format!("{}_1", headers::VALUE)),
                Some("a3, b3".into()),
            ],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_display_simple() {
        let info = Porcelain(vec![
            vec![
                Some(headers::ID.into()),
                Some("name".into()),
                Some("age".into()),
            ],
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("30".into()),
            ],
            vec![Some("user2".into()), Some("Bob".into()), Some("25".into())],
        ]);

        let actual = info.to_string();
        let expected = [
            //
            "ID     name   age",
            "user1  Alice  30",
            "user2  Bob    25",
        ]
        .join("\n");

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_display_with_none() {
        let info = Porcelain(vec![
            vec![
                Some(headers::ID.into()),
                Some("name".into()),
                Some("age".into()),
            ],
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("30".into()),
            ],
            vec![Some("user2".into()), None, Some("25".into())],
        ]);

        let actual = info.to_string();
        let expected = [
            //
            "ID     name   age",
            "user1  Alice  30",
            "user2         25",
        ]
        .join("\n");

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_to_case() {
        use convert_case::Case;

        let info = Porcelain(vec![
            vec![
                Some(headers::ID.into()),
                Some("user_name".into()),
                Some("user_age".into()),
            ],
            vec![
                Some("user1".into()),
                Some("Alice Smith".into()),
                Some("30".into()),
            ],
            vec![
                Some("user2".into()),
                Some("Bob Jones".into()),
                Some("25".into()),
            ],
        ]);

        let actual = info.to_case(&[1], Case::Snake).into_rows();

        let expected = vec![
            vec![
                Some(headers::ID.into()),
                Some("user_name".into()),
                Some("user_age".into()),
            ],
            vec![
                Some("user1".into()),
                Some("alice_smith".into()),
                Some("30".into()),
            ],
            vec![
                Some("user2".into()),
                Some("bob_jones".into()),
                Some("25".into()),
            ],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_to_case_multiple_cols() {
        use convert_case::Case;

        let info = Porcelain(vec![
            vec![
                Some("FirstName".into()),
                Some("LastName".into()),
                Some("Age".into()),
            ],
            vec![
                Some("Alice".into()),
                Some("Smith".into()),
                Some("30".into()),
            ],
        ]);

        let actual = info.to_case(&[0, 1, 2], Case::Kebab).into_rows();

        let expected = vec![
            vec![
                Some("first-name".into()),
                Some("last-name".into()),
                Some("age".into()),
            ],
            vec![
                Some("alice".into()),
                Some("smith".into()),
                Some("30".into()),
            ],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_display_empty() {
        let info = Porcelain::new();

        let actual = info.to_string();
        let expected = "";

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_sort_by_single_col() {
        let fixture = Porcelain(vec![
            vec![
                Some(headers::ID.into()),
                Some("name".into()),
                Some("age".into()),
            ],
            vec![
                Some("user3".into()),
                Some("Charlie".into()),
                Some("35".into()),
            ],
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("30".into()),
            ],
            vec![Some("user2".into()), Some("Bob".into()), Some("25".into())],
        ]);

        let actual = fixture.sort_by(&[1]).into_rows();

        let expected = vec![
            vec![
                Some(headers::ID.into()),
                Some("name".into()),
                Some("age".into()),
            ],
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("30".into()),
            ],
            vec![Some("user2".into()), Some("Bob".into()), Some("25".into())],
            vec![
                Some("user3".into()),
                Some("Charlie".into()),
                Some("35".into()),
            ],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_sort_by_multiple_cols() {
        let fixture = Porcelain(vec![
            vec![
                Some(headers::ID.into()),
                Some("city".into()),
                Some("name".into()),
            ],
            vec![
                Some("user3".into()),
                Some("NYC".into()),
                Some("Charlie".into()),
            ],
            vec![
                Some("user1".into()),
                Some("NYC".into()),
                Some("Alice".into()),
            ],
            vec![Some("user2".into()), Some("LA".into()), Some("Bob".into())],
            vec![Some("user4".into()), Some("NYC".into()), Some("Bob".into())],
        ]);

        let actual = fixture.sort_by(&[1, 2]).into_rows();

        let expected = vec![
            vec![
                Some(headers::ID.into()),
                Some("city".into()),
                Some("name".into()),
            ],
            vec![Some("user2".into()), Some("LA".into()), Some("Bob".into())],
            vec![
                Some("user1".into()),
                Some("NYC".into()),
                Some("Alice".into()),
            ],
            vec![Some("user4".into()), Some("NYC".into()), Some("Bob".into())],
            vec![
                Some("user3".into()),
                Some("NYC".into()),
                Some("Charlie".into()),
            ],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_sort_by_with_none_values() {
        let fixture = Porcelain(vec![
            vec![
                Some(headers::ID.into()),
                Some("name".into()),
                Some("age".into()),
            ],
            vec![Some("user3".into()), None, Some("35".into())],
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("30".into()),
            ],
            vec![Some("user2".into()), Some("Bob".into()), None],
        ]);

        let actual = fixture.sort_by(&[1]).into_rows();

        let expected = vec![
            vec![
                Some(headers::ID.into()),
                Some("name".into()),
                Some("age".into()),
            ],
            vec![
                Some("user1".into()),
                Some("Alice".into()),
                Some("30".into()),
            ],
            vec![Some("user2".into()), Some("Bob".into()), None],
            vec![Some("user3".into()), None, Some("35".into())],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_sort_by_empty_cols() {
        let fixture = Porcelain(vec![
            vec![Some(headers::ID.into()), Some("name".into())],
            vec![Some("user2".into()), Some("Bob".into())],
            vec![Some("user1".into()), Some("Alice".into())],
        ]);

        let actual = fixture.sort_by(&[]).into_rows();

        let expected = vec![
            vec![Some(headers::ID.into()), Some("name".into())],
            vec![Some("user2".into()), Some("Bob".into())],
            vec![Some("user1".into()), Some("Alice".into())],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_sort_by_empty_porcelain() {
        let fixture = Porcelain::new();
        let actual = fixture.sort_by(&[0, 1]).into_rows();
        let expected: Vec<Vec<Option<String>>> = vec![];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_sort_by_preserves_header() {
        let fixture = Porcelain(vec![
            vec![Some("HEADER".into()), Some("COL1".into())],
            vec![Some("z".into()), Some("value".into())],
            vec![Some("a".into()), Some("value".into())],
        ]);

        let actual = fixture.sort_by(&[0]).into_rows();

        let expected = vec![
            vec![Some("HEADER".into()), Some("COL1".into())],
            vec![Some("a".into()), Some("value".into())],
            vec![Some("z".into()), Some("value".into())],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_set_headers_basic() {
        let fixture = Porcelain(vec![
            vec![Some("old1".into()), Some("old2".into())],
            vec![Some("data1".into()), Some("data2".into())],
        ]);

        let actual = fixture.set_headers(vec!["new1", "new2"]).into_rows();

        let expected = vec![
            vec![Some("new1".into()), Some("new2".into())],
            vec![Some("data1".into()), Some("data2".into())],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_set_headers_empty_porcelain() {
        let fixture = Porcelain::new();

        let actual = fixture.set_headers(vec!["header1", "header2"]).into_rows();

        let expected = vec![vec![Some("header1".into()), Some("header2".into())]];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_set_headers_different_count() {
        let fixture = Porcelain(vec![
            vec![
                Some("old1".into()),
                Some("old2".into()),
                Some("old3".into()),
            ],
            vec![
                Some("data1".into()),
                Some("data2".into()),
                Some("data3".into()),
            ],
        ]);

        let actual = fixture.set_headers(vec!["new1", "new2"]).into_rows();

        let expected = vec![
            vec![Some("new1".into()), Some("new2".into())],
            vec![
                Some("data1".into()),
                Some("data2".into()),
                Some("data3".into()),
            ],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_set_headers_empty_headers() {
        let fixture = Porcelain(vec![
            vec![Some("old1".into()), Some("old2".into())],
            vec![Some("data1".into()), Some("data2".into())],
        ]);

        let actual = fixture.set_headers(Vec::<String>::new()).into_rows();

        let expected = vec![
            vec![Some("old1".into()), Some("old2".into())],
            vec![Some("data1".into()), Some("data2".into())],
        ];

        assert_eq!(actual, expected)
    }

    #[test]
    fn test_set_headers_with_string_refs() {
        let fixture = Porcelain(vec![vec![Some("old".into())], vec![Some("data".into())]]);

        let actual = fixture.set_headers(["ID", "Name", "Age"]).into_rows();

        let expected = vec![
            vec![Some("ID".into()), Some("Name".into()), Some("Age".into())],
            vec![Some("data".into())],
        ];

        assert_eq!(actual, expected)
    }
}
