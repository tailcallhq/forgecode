use std::fmt::{Display, Formatter};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::operation::PermissionOperation;
use super::policy::Policy;
use super::types::Permission;
use crate::Rule;

/// Collection of policies
///
/// Policies are stored as an ordered list to preserve the order in which they
/// appear in `permissions.yaml`. The [`PolicyEngine`] evaluates them in that
/// order using a *first-matching-policy-wins* model, so the user's declared
/// order is the priority order.
///
/// [`PolicyEngine`]: crate::PolicyEngine
#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PolicyConfig {
    /// Ordered list of policies to evaluate
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policies: Vec<Policy>,
}

impl PolicyConfig {
    /// Create a new empty policies collection
    pub fn new() -> Self {
        Self { policies: Vec::new() }
    }

    /// Append a policy to the collection, preserving declaration order
    pub fn add_policy(mut self, policy: Policy) -> Self {
        self.policies.push(policy);
        self
    }

    /// Evaluate all policies against an operation
    /// Returns permission results for debugging policy decisions
    pub fn eval(&self, operation: &PermissionOperation) -> Vec<Option<Permission>> {
        self.policies
            .iter()
            .map(|policy| policy.eval(operation))
            .collect()
    }

    /// Find all matching rules across all policies
    pub fn find_rules(&self, operation: &PermissionOperation) -> Vec<&Rule> {
        self.policies
            .iter()
            .flat_map(|policy| policy.find_rules(operation))
            .collect()
    }
}

impl Display for PolicyConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.policies.is_empty() {
            write!(f, "No policies defined")
        } else {
            let policies: Vec<String> = self.policies.iter().map(|p| format!("• {p}")).collect();
            write!(f, "Policies:\n{}", policies.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{Permission, PermissionOperation, Policy, Rule, WriteRule};

    fn fixture_write_operation() -> PermissionOperation {
        PermissionOperation::Write {
            path: PathBuf::from("src/main.rs"),
            cwd: PathBuf::from("/test/cwd"),
            message: "Create/overwrite file: src/main.rs".to_string(),
        }
    }

    #[test]
    fn test_policies_eval() {
        let fixture = PolicyConfig::new()
            .add_policy(Policy::Simple {
                permission: Permission::Allow,
                rule: Rule::Write(WriteRule { write: "src/**/*.rs".to_string(), dir: None }),
            })
            .add_policy(Policy::Simple {
                permission: Permission::Deny,
                rule: Rule::Write(WriteRule { write: "**/*.py".to_string(), dir: None }),
            });
        let operation = fixture_write_operation();

        let actual = fixture.eval(&operation);

        assert_eq!(actual.len(), 2);
        assert_eq!(actual[0].as_ref().unwrap(), &Permission::Allow);
        assert_eq!(actual[1], None); // Second rule doesn't match
    }

    #[cfg(test)]
    mod yaml_policies_tests {
        use crate::policies::{Permission, Policy, PolicyConfig, Rule};

        #[tokio::test]
        async fn test_yaml_policies_roundtrip() {
            let yaml_content = forge_test_kit::fixture!("/src/fixtures/policies_test.yml").await;

            let policies: PolicyConfig =
                serde_yml::from_str(&yaml_content).expect("Failed to parse policies YAML");

            assert_eq!(policies.policies.len(), 3);

            // Test first policy - get first policy from the set
            let first_policy = policies.policies.iter().next().unwrap();
            if let Policy::Simple { permission, rule } = first_policy {
                assert_eq!(permission, &Permission::Allow);
                if let Rule::Read(rule) = rule {
                    assert_eq!(rule.read, "**/*.rs");
                } else {
                    panic!("Expected Read rule");
                }
            } else {
                panic!("Expected Simple policy");
            }

            // Test round-trip serialization
            let serialized = serde_yml::to_string(&policies).expect("Failed to serialize policies");
            let deserialized: PolicyConfig =
                serde_yml::from_str(&serialized).expect("Failed to deserialize policies");
            assert_eq!(policies, deserialized);
        }
    }
}
