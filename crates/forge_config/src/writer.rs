use std::path::Path;

use crate::ForgeConfig;

/// Writes a [`ForgeConfig`] to the user configuration file on disk.
pub struct ConfigWriter {
    config: ForgeConfig,
}

impl ConfigWriter {
    /// Creates a new `ConfigWriter` for the given configuration.
    pub fn new(config: ForgeConfig) -> Self {
        Self { config }
    }

    /// Serializes and writes the configuration to `path`, creating all parent
    /// directories recursively if they do not already exist.
    ///
    /// The output includes a leading `$schema` key pointing to the Forge
    /// configuration JSON schema, which enables editor validation and
    /// auto-complete.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration cannot be serialized or the file
    /// cannot be written.
    pub fn write(&self, path: &Path) -> crate::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let config_toml = toml_edit::ser::to_string_pretty(&self.config)?;
        let contents =
            format!("\"$schema\" = \"https://forgecode.dev/schema.json\"\n\n{config_toml}");

        std::fs::write(path, contents)?;

        Ok(())
    }
}
