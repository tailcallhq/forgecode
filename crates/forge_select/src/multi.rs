use std::io::IsTerminal;

use anyhow::Result;
use console::strip_ansi_codes;

use crate::preview::{SelectMode, SelectRow, SelectUiOptions};

/// Builder for multi-select prompts.
pub struct MultiSelectBuilder<T> {
    pub(crate) message: String,
    pub(crate) options: Vec<T>,
}

impl<T> MultiSelectBuilder<T> {
    /// Execute multi-select prompt.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(Vec<T>))` when the user selects one or more options.
    /// - `Ok(None)` when no options are available or the user cancels.
    ///
    /// # Errors
    ///
    /// Returns an error if terminal setup, event handling, or rendering fails.
    pub fn prompt(self) -> Result<Option<Vec<T>>>
    where
        T: std::fmt::Display + Clone,
    {
        if !std::io::stderr().is_terminal() {
            return Ok(None);
        }

        if self.options.is_empty() {
            return Ok(None);
        }

        let rows = self
            .options
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let display = strip_ansi_codes(&item.to_string()).trim().to_string();
                SelectRow::new(index.to_string(), display.clone()).search(display)
            })
            .collect::<Vec<_>>();

        let selected = SelectUiOptions::new(format!("{} ❯ ", self.message), rows)
            .mode(SelectMode::Multi)
            .prompt_multi()?;

        Ok(selected.and_then(|rows| {
            let selected_items = rows
                .into_iter()
                .filter_map(|row| {
                    row.raw
                        .parse::<usize>()
                        .ok()
                        .and_then(|index| self.options.get(index).cloned())
                })
                .collect::<Vec<_>>();

            if selected_items.is_empty() {
                None
            } else {
                Some(selected_items)
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use crate::ForgeWidget;

    #[test]
    fn test_multi_select_builder_creates() {
        let builder = ForgeWidget::multi_select("Select options:", vec!["a", "b", "c"]);
        assert_eq!(builder.message, "Select options:");
        assert_eq!(builder.options, vec!["a", "b", "c"]);
    }
}
