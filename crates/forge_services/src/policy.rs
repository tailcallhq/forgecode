use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use anyhow::Context;
use bytes::Bytes;
use forge_app::domain::{Permission, PermissionOperation, PolicyConfig, PolicyEngine};
use forge_app::{
    DirectoryReaderInfra, EnvironmentInfra, FileInfoInfra, FileReaderInfra, FileWriterInfra,
    PolicyDecision, PolicyService, UserInfra,
};

#[derive(Clone)]
pub struct ForgePolicyService<I> {
    infra: Arc<I>,
}
/// Default policies loaded once at startup from the embedded YAML file
static DEFAULT_POLICIES: LazyLock<PolicyConfig> = LazyLock::new(|| {
    let yaml_content = include_str!("./permissions.default.yaml");
    serde_yml::from_str(yaml_content).expect(
        "Failed to parse default policies YAML. This should never happen as the YAML is embedded.",
    )
});

impl<I> ForgePolicyService<I>
where
    I: FileReaderInfra + FileWriterInfra + FileInfoInfra + EnvironmentInfra + DirectoryReaderInfra,
{
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }

    fn permissions_path(&self) -> PathBuf {
        self.infra.get_environment().permissions_path()
    }

    /// Create a policies collection with sensible defaults
    /// Returns a clone of the preloaded default policies
    fn load_default_policies() -> PolicyConfig {
        DEFAULT_POLICIES.clone()
    }

    /// Load all policy definitions from the forge/policies directory
    async fn read_policies(&self) -> anyhow::Result<Option<PolicyConfig>> {
        let policies_path = self.permissions_path();
        if !self.infra.exists(&policies_path).await? {
            return Ok(None);
        }

        let content = self.infra.read_utf8(&policies_path).await?;
        let policies = serde_yml::from_str(&content)
            .with_context(|| format!("Failed to parse policy {}", policies_path.display()))?;

        Ok(Some(policies))
    }

    /// Create a default policies file if it does not exist
    async fn init_policies(&self) -> anyhow::Result<()> {
        let policies_path = self.permissions_path();

        // Check if the file already exists
        if self.infra.exists(&policies_path).await? {
            return Ok(());
        }

        // Get the default policies content
        let default_policies = Self::load_default_policies();
        let content = serde_yml::to_string(&default_policies)
            .with_context(|| "Failed to serialize default policies to YAML")?;

        // Write the default policies to the file
        self.infra
            .write(&policies_path, Bytes::from(content))
            .await?;

        Ok(())
    }

    /// Get or create policies, prompting user if needed
    #[async_recursion::async_recursion]
    async fn get_or_create_policies(&self) -> anyhow::Result<(PolicyConfig, Option<PathBuf>)>
    where
        I: UserInfra,
    {
        if let Some(policies) = self.read_policies().await? {
            Ok((policies, None))
        } else {
            self.init_policies().await?;
            let (policies, _) = self.get_or_create_policies().await?;
            Ok((policies, Some(self.permissions_path())))
        }
    }
}

#[async_trait::async_trait]
impl<I> PolicyService for ForgePolicyService<I>
where
    I: FileReaderInfra
        + FileWriterInfra
        + FileInfoInfra
        + EnvironmentInfra
        + DirectoryReaderInfra
        + UserInfra,
{
    /// Check if an operation is allowed based on policies and handle user
    /// confirmation
    async fn check_operation_permission(
        &self,
        operation: &PermissionOperation,
    ) -> anyhow::Result<PolicyDecision> {
        let (policies, path) = self.get_or_create_policies().await?;

        let engine = PolicyEngine::new(&policies);
        let permission = engine.can_perform(operation);

        match permission {
            Permission::Deny => Ok(PolicyDecision { allowed: false, path }),
            Permission::Allow => Ok(PolicyDecision { allowed: true, path }),
            Permission::Confirm => {
                // Request user confirmation using the confirm widget
                let confirmation_msg = match operation {
                    PermissionOperation::Read { message, .. } => {
                        format!("{message}. Allow?")
                    }
                    PermissionOperation::Write { message, .. } => {
                        format!("{message}. Allow?")
                    }
                    PermissionOperation::Execute { .. } => "Allow this operation?".to_string(),
                    PermissionOperation::Fetch { message, .. } => {
                        format!("{message}. Allow?")
                    }
                };

                match self.infra.confirm(&confirmation_msg).await? {
                    Some(true) => Ok(PolicyDecision { allowed: true, path }),
                    Some(false) | None => Ok(PolicyDecision { allowed: false, path }),
                }
            }
        }
    }
}
