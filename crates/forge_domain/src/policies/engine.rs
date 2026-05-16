use super::operation::PermissionOperation;
use super::policy::Policy;
use crate::PolicyConfig;
use crate::policies::Permission;

/// High-level policy engine that provides convenient methods for checking
/// policies
///
/// This wrapper around Workflow provides easy-to-use methods for services to
/// check if operations are allowed without having to construct Operation enums
/// manually.
pub struct PolicyEngine<'a> {
    policies: &'a PolicyConfig,
}

impl<'a> PolicyEngine<'a> {
    /// Create a new PolicyEngine from a workflow
    pub fn new(policies: &'a PolicyConfig) -> Self {
        Self { policies }
    }

    /// Check if an operation is allowed
    /// Returns permission result
    pub fn can_perform(&self, operation: &PermissionOperation) -> Permission {
        self.evaluate_policies(operation)
    }

    /// Internal helper function to evaluate policies for a given operation.
    /// Returns the permission of the first policy whose rule matches the
    /// operation, or [`Permission::Confirm`] if no policy matches.
    fn evaluate_policies(&self, operation: &PermissionOperation) -> Permission {
        self.evaluate_policy_set(self.policies.policies.iter(), operation)
            .unwrap_or(Permission::Confirm)
    }

    /// Finds the most-specific matching policy. Ties broken by restrictiveness
    /// (Deny > Confirm > Allow).
    fn evaluate_policy_set<'p, I: IntoIterator<Item = &'p Policy>>(
        &self,
        policies: I,
        operation: &PermissionOperation,
    ) -> Option<Permission> {
        policies
            .into_iter()
            .filter_map(|policy| {
                policy
                    .eval(operation)
                    .map(|permission| (permission, policy.specificity()))
            })
            .max_by_key(|(permission, specificity)| (*specificity, permission.restrictiveness()))
            .map(|(permission, _)| permission)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{ExecuteRule, Fetch, Permission, Policy, PolicyConfig, ReadRule, Rule, WriteRule};

    fn fixture_workflow_with_read_policy() -> PolicyConfig {
        PolicyConfig::new().add_policy(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Read(ReadRule { read: "src/**/*.rs".to_string(), dir: None }),
        })
    }

    fn fixture_workflow_with_write_policy() -> PolicyConfig {
        PolicyConfig::new().add_policy(Policy::Simple {
            permission: Permission::Deny,
            rule: Rule::Write(WriteRule { write: "**/*.rs".to_string(), dir: None }),
        })
    }

    fn fixture_workflow_with_execute_policy() -> PolicyConfig {
        PolicyConfig::new().add_policy(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Execute(ExecuteRule { command: "cargo *".to_string(), dir: None }),
        })
    }

    fn fixture_workflow_with_write_policy_confirm() -> PolicyConfig {
        PolicyConfig::new().add_policy(Policy::Simple {
            permission: Permission::Confirm,
            rule: Rule::Write(WriteRule { write: "src/**/*.rs".to_string(), dir: None }),
        })
    }

    fn fixture_workflow_with_net_fetch_policy() -> PolicyConfig {
        PolicyConfig::new().add_policy(Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Fetch(Fetch { url: "https://api.example.com/*".to_string(), dir: None }),
        })
    }

    #[test]
    fn test_policy_engine_can_perform_read() {
        let fixture_workflow = fixture_workflow_with_read_policy();
        let fixture = PolicyEngine::new(&fixture_workflow);
        let operation = PermissionOperation::Read {
            path: std::path::PathBuf::from("src/main.rs"),
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Read file: src/main.rs".to_string(),
        };

        let actual = fixture.can_perform(&operation);

        assert_eq!(actual, Permission::Allow);
    }

    #[test]
    fn test_policy_engine_can_perform_write() {
        let fixture_workflow = fixture_workflow_with_write_policy();
        let fixture = PolicyEngine::new(&fixture_workflow);
        let operation = PermissionOperation::Write {
            path: std::path::PathBuf::from("src/main.rs"),
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Create/overwrite file: src/main.rs".to_string(),
        };

        let actual = fixture.can_perform(&operation);

        assert_eq!(actual, Permission::Deny);
    }

    #[test]
    fn test_policy_engine_can_perform_write_with_confirm() {
        let fixture_workflow = fixture_workflow_with_write_policy_confirm();
        let fixture = PolicyEngine::new(&fixture_workflow);
        let operation = PermissionOperation::Write {
            path: std::path::PathBuf::from("src/main.rs"),
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Create/overwrite file: src/main.rs".to_string(),
        };

        let actual = fixture.can_perform(&operation);

        assert_eq!(actual, Permission::Confirm);
    }

    #[test]
    fn test_policy_engine_can_perform_execute() {
        let fixture_workflow = fixture_workflow_with_execute_policy();
        let fixture = PolicyEngine::new(&fixture_workflow);
        let operation = PermissionOperation::Execute {
            command: "cargo build".to_string(),
            cwd: std::path::PathBuf::from("/test/cwd"),
        };

        let actual = fixture.can_perform(&operation);

        assert_eq!(actual, Permission::Allow);
    }

    /// Regression: specific allow should win over broad deny (issue #3085).
    #[test]
    fn test_policy_engine_specific_allow_should_win_over_broad_deny() {
        let fixture_workflow = PolicyConfig::new()
            .add_policy(Policy::Simple {
                permission: Permission::Allow,
                rule: Rule::Read(ReadRule { read: "**/*".to_string(), dir: None }),
            })
            .add_policy(Policy::Simple {
                permission: Permission::Allow,
                rule: Rule::Write(WriteRule { write: "**/*.rs".to_string(), dir: None }),
            })
            .add_policy(Policy::Simple {
                permission: Permission::Deny,
                rule: Rule::Write(WriteRule { write: "**/*".to_string(), dir: None }),
            });
        let fixture = PolicyEngine::new(&fixture_workflow);
        let operation = PermissionOperation::Write {
            path: std::path::PathBuf::from("test.rs"),
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Create/overwrite file: test.rs".to_string(),
        };

        let actual = fixture.can_perform(&operation);
        let expected = Permission::Allow;

        assert_eq!(actual, expected);
    }

    /// Verifies order independence: deny first, allow second should yield same
    /// result.
    #[test]
    fn test_policy_engine_specific_allow_wins_regardless_of_order() {
        let fixture_workflow = PolicyConfig::new()
            .add_policy(Policy::Simple {
                permission: Permission::Deny,
                rule: Rule::Write(WriteRule { write: "**/*".to_string(), dir: None }),
            })
            .add_policy(Policy::Simple {
                permission: Permission::Allow,
                rule: Rule::Write(WriteRule { write: "**/*.rs".to_string(), dir: None }),
            });
        let fixture = PolicyEngine::new(&fixture_workflow);
        let rs_op = PermissionOperation::Write {
            path: std::path::PathBuf::from("test.rs"),
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Create/overwrite file: test.rs".to_string(),
        };
        let py_op = PermissionOperation::Write {
            path: std::path::PathBuf::from("test.py"),
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Create/overwrite file: test.py".to_string(),
        };

        // .rs matches both rules; the more-specific allow wins.
        assert_eq!(fixture.can_perform(&rs_op), Permission::Allow);
        // .py only matches the broad deny, so it stays denied.
        assert_eq!(fixture.can_perform(&py_op), Permission::Deny);
    }

    /// Verifies carve-out exception: specific deny beats broad allow.
    #[test]
    fn test_policy_engine_specific_deny_carves_out_exception_in_broad_allow() {
        let fixture_workflow = PolicyConfig::new()
            .add_policy(Policy::Simple {
                permission: Permission::Allow,
                rule: Rule::Write(WriteRule { write: "**/*.rs".to_string(), dir: None }),
            })
            .add_policy(Policy::Simple {
                permission: Permission::Deny,
                rule: Rule::Write(WriteRule { write: "secret.rs".to_string(), dir: None }),
            });
        let fixture = PolicyEngine::new(&fixture_workflow);
        let secret_op = PermissionOperation::Write {
            path: std::path::PathBuf::from("secret.rs"),
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Create/overwrite file: secret.rs".to_string(),
        };
        let other_op = PermissionOperation::Write {
            path: std::path::PathBuf::from("main.rs"),
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Create/overwrite file: main.rs".to_string(),
        };

        assert_eq!(fixture.can_perform(&secret_op), Permission::Deny);
        assert_eq!(fixture.can_perform(&other_op), Permission::Allow);
    }

    /// Tie-breaker: Deny wins when specificity is equal.
    #[test]
    fn test_policy_engine_equal_specificity_prefers_deny() {
        let fixture_workflow = PolicyConfig::new()
            .add_policy(Policy::Simple {
                permission: Permission::Allow,
                rule: Rule::Write(WriteRule { write: "secret.rs".to_string(), dir: None }),
            })
            .add_policy(Policy::Simple {
                permission: Permission::Deny,
                rule: Rule::Write(WriteRule { write: "secret.rs".to_string(), dir: None }),
            });
        let fixture = PolicyEngine::new(&fixture_workflow);
        let operation = PermissionOperation::Write {
            path: std::path::PathBuf::from("secret.rs"),
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Create/overwrite file: secret.rs".to_string(),
        };

        let actual = fixture.can_perform(&operation);
        let expected = Permission::Deny;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_policy_engine_path_prefix_specificity() {
        let fixture_workflow = PolicyConfig::new()
            .add_policy(Policy::Simple {
                permission: Permission::Allow,
                rule: Rule::Write(WriteRule { write: "src/**/*.rs".to_string(), dir: None }),
            })
            .add_policy(Policy::Simple {
                permission: Permission::Deny,
                rule: Rule::Write(WriteRule { write: "*.rs".to_string(), dir: None }),
            });
        let fixture = PolicyEngine::new(&fixture_workflow);
        let inside_src = PermissionOperation::Write {
            path: std::path::PathBuf::from("src/utils/helper.rs"),
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Create/overwrite file: src/utils/helper.rs".to_string(),
        };
        let outside_src = PermissionOperation::Write {
            path: std::path::PathBuf::from("test.rs"),
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Create/overwrite file: test.rs".to_string(),
        };

        // Path "narrowness" (src/ prefix) wins because it has more literals.
        assert_eq!(fixture.can_perform(&inside_src), Permission::Allow);
        // File outside src/ only matches the broad deny.
        assert_eq!(fixture.can_perform(&outside_src), Permission::Deny);
    }

    #[test]
    fn test_policy_engine_can_perform_net_fetch() {
        let fixture_workflow = fixture_workflow_with_net_fetch_policy();
        let fixture = PolicyEngine::new(&fixture_workflow);
        let operation = PermissionOperation::Fetch {
            url: "https://api.example.com/data".to_string(),
            cwd: std::path::PathBuf::from("/test/cwd"),
            message: "Fetch content from URL: https://api.example.com/data".to_string(),
        };

        let actual = fixture.can_perform(&operation);

        assert_eq!(actual, Permission::Allow);
    }
}
