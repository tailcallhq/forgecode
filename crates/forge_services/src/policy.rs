use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use anyhow::Context;
use bytes::Bytes;
use forge_app::domain::{
    ExecuteRule, Fetch, McpFilter, McpRule, Permission, PermissionOperation, Policy, PolicyConfig,
    PolicyEngine, ReadRule, Rule, WriteRule,
};
use forge_app::{
    DirectoryReaderInfra, EnvironmentInfra, FileInfoInfra, FileReaderInfra, FileWriterInfra,
    PolicyDecision, PolicyService, SelectPrompt, UserInfra,
};
use strum_macros::{Display, EnumIter};

/// User response for permission confirmation requests
#[derive(Debug, Clone, PartialEq, Eq, Display, EnumIter, strum_macros::EnumString)]
pub enum PolicyPermission {
    /// Accept the operation
    #[strum(to_string = "Accept")]
    Accept,
    /// Reject the operation
    #[strum(to_string = "Reject")]
    Reject,
    /// Accept the operation and remember this choice for similar operations
    #[strum(to_string = "Accept and Remember")]
    AcceptAndRemember,
}

/// Two-choice prompt for operations where both Accept and Reject are
/// persisted so the user is never asked again. Use this instead of
/// [`PolicyPermission`] when there is no meaningful "one-off allow" path.
#[derive(Debug, Clone, PartialEq, Eq, Display, EnumIter, strum_macros::EnumString)]
enum ConfirmPermission {
    /// Allow the operation and remember this choice
    #[strum(to_string = "Accept")]
    Accept,
    /// Deny the operation and remember this choice
    #[strum(to_string = "Reject")]
    Reject,
}

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

    /// Add a policy for a specific operation type
    async fn add_policy_for_operation(
        &self,
        operation: &PermissionOperation,
    ) -> anyhow::Result<Option<PathBuf>>
    where
        I: UserInfra,
    {
        if let Some(new_policy) = create_policy_for_operation(operation, None) {
            // TODO: Can return a diff later
            self.modify_policy(new_policy).await?;
            Ok(Some(self.permissions_path()))
        } else {
            Ok(None)
        }
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

    /// Add or modify a policy in the policies file
    async fn modify_policy(&self, policy: Policy) -> anyhow::Result<()> {
        let policies_path = self.permissions_path();
        let mut policies = self.read_policies().await?.unwrap_or_default();

        // Add the new policy to the collection
        policies = policies.add_policy(policy);

        // Serialize the updated policies to YAML
        let new_content = serde_yml::to_string(&policies)
            .with_context(|| "Failed to serialize policies to YAML")?;

        // Write the updated content
        self.infra
            .write(&policies_path, Bytes::from(new_content.to_owned()))
            .await?;

        Ok(())
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
    /// Unconditionally persist an allow policy for the given operation.
    async fn allow_operation(&self, operation: &PermissionOperation) -> anyhow::Result<()> {
        self.add_policy_for_operation(operation).await.map(|_| ())
    }

    /// Check whether an operation is explicitly permitted by the current
    /// policy without prompting the user. `Confirm` is treated as not
    /// permitted so callers can handle it themselves (e.g. show a warning).
    async fn is_operation_permitted(
        &self,
        operation: &PermissionOperation,
    ) -> anyhow::Result<bool> {
        let (policies, _) = self.get_or_create_policies().await?;
        let engine = PolicyEngine::new(&policies);
        Ok(matches!(engine.can_perform(operation), Permission::Allow))
    }

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
                // Request user confirmation using UserInfra
                let prompt = match operation {
                    PermissionOperation::Read { message, .. } => {
                        SelectPrompt::new(format!("{message}. How would you like to proceed?"))
                    }
                    PermissionOperation::Write { message, .. } => {
                        SelectPrompt::new(format!("{message}. How would you like to proceed?"))
                    }
                    PermissionOperation::Execute { .. } => {
                        SelectPrompt::new("How would you like to proceed?")
                    }
                    PermissionOperation::Fetch { message, .. } => {
                        SelectPrompt::new(format!("{message}. How would you like to proceed?"))
                    }
                    PermissionOperation::Mcp { message, config, cwd } => {
                        let header = mcp_config_header(config);
                        let prompt = SelectPrompt::new(message.clone()).with_header(header);
                        return match self
                            .infra
                            .select_one_enum::<ConfirmPermission>(prompt)
                            .await?
                        {
                            Some(ConfirmPermission::Accept) => {
                                let update_path = self.add_policy_for_operation(operation).await?;
                                Ok(PolicyDecision { allowed: true, path: update_path.or(path) })
                            }
                            Some(ConfirmPermission::Reject) | None => {
                                let deny_policy = Policy::Simple {
                                    permission: Permission::Deny,
                                    rule: forge_app::domain::Rule::Mcp(
                                        forge_app::domain::McpRule {
                                            mcp: forge_app::domain::McpFilter::from_config(
                                                config, cwd,
                                            ),
                                        },
                                    ),
                                };
                                self.modify_policy(deny_policy).await?;
                                Ok(PolicyDecision {
                                    allowed: false,
                                    path: Some(self.permissions_path()),
                                })
                            }
                        };
                    }
                };

                match self
                    .infra
                    .select_one_enum::<PolicyPermission>(prompt)
                    .await?
                {
                    Some(PolicyPermission::Accept) => Ok(PolicyDecision { allowed: true, path }),
                    Some(PolicyPermission::AcceptAndRemember) => {
                        let update_path = self.add_policy_for_operation(operation).await?;
                        Ok(PolicyDecision { allowed: true, path: update_path.or(path) })
                    }
                    Some(PolicyPermission::Reject) | None => {
                        Ok(PolicyDecision { allowed: false, path })
                    }
                }
            }
        }
    }
}

/// Builds the header lines describing an MCP server's configuration.
fn mcp_config_header(config: &forge_app::domain::McpServerConfig) -> Vec<String> {
    use forge_app::domain::McpServerConfig;
    match config {
        McpServerConfig::Stdio(s) => {
            let mut lines = vec![format!("command: {}", s.command)];
            if !s.args.is_empty() {
                lines.push(format!("args: {}", s.args.join(" ")));
            }
            lines
        }
        McpServerConfig::Http(h) => vec![format!("url: {}", h.url)],
    }
}

/// Create a policy for an operation based on its type
fn create_policy_for_operation(
    operation: &PermissionOperation,
    dir: Option<String>,
) -> Option<Policy> {
    fn create_file_policy(
        path: &std::path::Path,
        rule_constructor: fn(String) -> Rule,
    ) -> Option<Policy> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|extension| Policy::Simple {
                permission: Permission::Allow,
                rule: rule_constructor(format!("*.{extension}")),
            })
    }

    match operation {
        PermissionOperation::Read { path, cwd: _, message: _ } => {
            create_file_policy(path, |pattern| {
                Rule::Read(ReadRule { read: pattern, dir: None })
            })
        }
        PermissionOperation::Write { path, cwd: _, message: _ } => {
            create_file_policy(path, |pattern| {
                Rule::Write(WriteRule { write: pattern, dir: None })
            })
        }

        PermissionOperation::Fetch { url, cwd: _, message: _ } => {
            if let Ok(parsed_url) = url::Url::parse(url) {
                parsed_url.host_str().map(|host| Policy::Simple {
                    permission: Permission::Allow,
                    rule: Rule::Fetch(Fetch { url: format!("{host}*"), dir: None }),
                })
            } else {
                Some(Policy::Simple {
                    permission: Permission::Allow,
                    rule: Rule::Fetch(Fetch { url: url.to_string(), dir: None }),
                })
            }
        }
        PermissionOperation::Execute { command, cwd: _ } => {
            let parts: Vec<&str> = command.split_whitespace().collect();
            match parts.as_slice() {
                [] => None,
                [cmd] => Some(Policy::Simple {
                    permission: Permission::Allow,
                    rule: Rule::Execute(ExecuteRule { command: format!("{cmd}*"), dir }),
                }),
                [cmd, subcmd, ..] => Some(Policy::Simple {
                    permission: Permission::Allow,
                    rule: Rule::Execute(ExecuteRule { command: format!("{cmd} {subcmd}*"), dir }),
                }),
            }
        }
        PermissionOperation::Mcp { config, cwd, .. } => Some(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Mcp(McpRule { mcp: McpFilter::from_config(config, cwd) }),
        }),
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_create_policy_for_read_operation() {
        let path = PathBuf::from("/path/to/file.rs");
        let operation = PermissionOperation::Read {
            path,
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Read file: /path/to/file.rs".to_string(),
        };

        let actual = create_policy_for_operation(&operation, None);

        let expected = Some(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Read(ReadRule { read: "*.rs".to_string(), dir: None }),
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_create_policy_for_write_operation() {
        let path = PathBuf::from("/path/to/file.json");
        let operation = PermissionOperation::Write {
            path,
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Create/overwrite file: /path/to/file.json".to_string(),
        };

        let actual = create_policy_for_operation(&operation, None);

        let expected = Some(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Write(WriteRule { write: "*.json".to_string(), dir: None }),
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_create_policy_for_write_patch_operation() {
        let path = PathBuf::from("/path/to/file.toml");
        let operation = PermissionOperation::Write {
            path,
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Modify file: /path/to/file.toml".to_string(),
        };

        let actual = create_policy_for_operation(&operation, None);

        let expected = Some(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Write(WriteRule { write: "*.toml".to_string(), dir: None }),
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_create_policy_for_net_fetch_operation() {
        let url = "https://example.com/api/data".to_string();
        let operation = PermissionOperation::Fetch {
            url,
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Fetch content from URL: https://example.com/api/data".to_string(),
        };

        let actual = create_policy_for_operation(&operation, None);

        let expected = Some(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Fetch(Fetch { url: "example.com*".to_string(), dir: None }),
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_create_policy_for_execute_operation_with_subcommand() {
        let command = "git push origin main".to_string();
        let operation =
            PermissionOperation::Execute { command, cwd: std::path::PathBuf::from("/test/cwd") };

        let actual = create_policy_for_operation(&operation, None);

        let expected = Some(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Execute(ExecuteRule { command: "git push*".to_string(), dir: None }),
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_create_policy_for_execute_operation_single_command() {
        let command = "ls".to_string();
        let operation =
            PermissionOperation::Execute { command, cwd: std::path::PathBuf::from("/test/cwd") };

        let actual = create_policy_for_operation(&operation, None);

        let expected = Some(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Execute(ExecuteRule { command: "ls*".to_string(), dir: None }),
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_create_policy_for_file_without_extension() {
        let path = PathBuf::from("/path/to/file");
        let operation = PermissionOperation::Read {
            path,
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Read file: /path/to/file".to_string(),
        };

        let actual = create_policy_for_operation(&operation, None);

        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_create_policy_for_invalid_url() {
        let url = "not-a-valid-url".to_string();
        let operation = PermissionOperation::Fetch {
            url,
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Fetch content from URL: not-a-valid-url".to_string(),
        };

        let actual = create_policy_for_operation(&operation, None);

        let expected = Some(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Fetch(Fetch { url: "not-a-valid-url".to_string(), dir: None }),
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_create_policy_for_empty_execute_command() {
        let command = "".to_string();
        let operation =
            PermissionOperation::Execute { command, cwd: std::path::PathBuf::from("/test/cwd") };

        let actual = create_policy_for_operation(&operation, None);

        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_create_policy_for_execute_operation_with_working_directory() {
        let command = "ls".to_string();
        let operation =
            PermissionOperation::Execute { command, cwd: std::path::PathBuf::from("/test/cwd") };
        let working_directory = Some("/home/user/project".to_string());

        let actual = create_policy_for_operation(&operation, working_directory.clone());

        let expected = Some(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Execute(ExecuteRule { command: "ls*".to_string(), dir: working_directory }),
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_create_policy_for_mcp_stdio_operation() {
        let operation = PermissionOperation::Mcp {
            config: forge_app::domain::McpServerConfig::new_stdio(
                "npx",
                vec!["-y".to_string(), "@github/mcp".to_string()],
                None,
            ),
            cwd: PathBuf::from("/home/user/project"),
            message: "Connect to MCP server: github".to_string(),
        };

        let actual = create_policy_for_operation(&operation, None);

        let expected = Some(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Mcp(McpRule {
                mcp: McpFilter {
                    command: Some("npx".to_string()),
                    args: Some(vec!["-y".to_string(), "@github/mcp".to_string()]),
                    url: None,
                    dir: Some("/home/user/project".to_string()),
                },
            }),
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_create_policy_for_mcp_http_operation() {
        let operation = PermissionOperation::Mcp {
            config: forge_app::domain::McpServerConfig::new_http("https://mcp.example.com/sse"),
            cwd: PathBuf::from("/home/user/project"),
            message: "Connect to MCP server: example".to_string(),
        };

        let actual = create_policy_for_operation(&operation, None);

        let expected = Some(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Mcp(McpRule {
                mcp: McpFilter {
                    url: Some("https://mcp.example.com/sse".to_string()),
                    dir: Some("/home/user/project".to_string()),
                    ..McpFilter::default()
                },
            }),
        });

        assert_eq!(actual, expected);
    }
}
