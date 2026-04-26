use std::io::IsTerminal;

use anyhow::Result;
use console::strip_ansi_codes;
use nucleo_picker::PickerOptions;
use nucleo_picker::error::PickError;
use nucleo_picker::render::StrRenderer;

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
    /// - `Ok(Some(Vec<T>))` - User selected one or more options
    /// - `Ok(None)` - No options available or user cancelled (ESC / Ctrl+C)
    ///
    /// # Errors
    ///
    /// Returns an error if the picker fails to start or interact
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

        let display_options: Vec<String> = self
            .options
            .iter()
            .map(|item| strip_ansi_codes(&item.to_string()).trim().to_string())
            .collect();

        let mut picker: nucleo_picker::Picker<String, _> =
            PickerOptions::default().reversed(true).picker(StrRenderer);

        picker.extend_exact(display_options);

        println!("{}", self.message);

        match picker.pick_multi() {
            Ok(selection) if selection.is_empty() => Ok(None),
            Ok(selection) => {
                let selected_items: Vec<T> = selection
                    .iter()
                    .filter_map(|selected_str| {
                        self.options
                            .iter()
                            .find(|opt| strip_ansi_codes(&opt.to_string()).trim() == *selected_str)
                            .cloned()
                    })
                    .collect();

                if selected_items.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(selected_items))
                }
            }
            Err(PickError::NotInteractive) => Ok(None),
            Err(PickError::UserInterrupted) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("Picker error: {e}")),
        }
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
