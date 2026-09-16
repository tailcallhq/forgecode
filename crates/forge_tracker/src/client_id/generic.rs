use machineid_rs::{Encryption, HWIDComponent, IdBuilder};

const PARAPHRASE: &str = "forge_key";
// CPUCores calls sysinfo 0.29's System::new_all(), which enumerates users and
// can dereference uninitialized group data for a missing primary GID (#3863).
// SystemID does not enumerate users. Keep this list limited to that safe path.
const COMPONENTS: [HWIDComponent; 1] = [HWIDComponent::SystemID];

/// Derives a stable client ID from the system ID on non-Android platforms.
///
/// # Errors
///
/// Returns an error if the system ID cannot be read or hashed.
pub fn get_or_create_client_id() -> anyhow::Result<String> {
    let mut builder = IdBuilder::new(Encryption::SHA256);
    for component in COMPONENTS {
        builder.add_component(component);
    }

    builder
        .build(PARAPHRASE)
        .map_err(|e| anyhow::anyhow!("Failed to generate machine ID: {e}"))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_client_id_components_do_not_enumerate_users() {
        let fixture = COMPONENTS;

        let actual = fixture.as_slice();

        assert!(matches!(actual, [HWIDComponent::SystemID]));
    }

    #[test]
    fn test_client_id_uses_only_system_id() {
        let mut fixture = IdBuilder::new(Encryption::SHA256);
        fixture.add_component(HWIDComponent::SystemID);

        let actual = get_or_create_client_id().map_err(|error| error.to_string());

        // Minimal containers may not have a system ID. Preserve the error as well
        // as the ID without requiring host-specific machine identity fixtures.
        let expected = fixture
            .build(PARAPHRASE)
            .map_err(|error| format!("Failed to generate machine ID: {error}"));
        assert_eq!(actual, expected);
    }
}
