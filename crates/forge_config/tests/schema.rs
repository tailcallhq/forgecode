use std::path::Path;

use forge_config::ForgeConfig;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn generate_workflow_schema() -> anyhow::Result<()> {
    let schema = schemars::schema_for!(ForgeConfig);
    let generated_schema = serde_json::to_string_pretty(&schema)?;

    // Use the crate root directory for the schema file
    let crate_root = env!("CARGO_MANIFEST_DIR");
    let schema_path = Path::new(crate_root).join("../../forge.schema.json");

    if is_ci::uncached() {
        // On CI: validate that the generated schema matches the committed file
        let existing_schema = tokio::fs::read_to_string(&schema_path).await?;
        assert_eq!(
            generated_schema.trim(),
            existing_schema.trim(),
            "Generated workflow schema does not match the committed schema file. \
             Please run the test locally to update the schema file."
        );
    } else {
        // Locally: generate and write the schema file
        tokio::fs::write(&schema_path, generated_schema).await?;
    }

    Ok(())
}
