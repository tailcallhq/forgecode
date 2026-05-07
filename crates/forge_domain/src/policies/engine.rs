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

    /// Helper function to evaluate a set of policies using a
    /// *most-specific-pattern-wins* model.
    ///
    /// Among all policies whose rule matches the operation, the one with
    /// the highest [`Policy::specificity`] takes effect. Ties are broken by
    /// preferring the more restrictive permission (`Deny > Confirm > Allow`),
    /// which keeps the engine safe-by-default when two equally-specific
    /// rules disagree.
    ///
    /// Declaration order in `permissions.yaml` does not affect the outcome,
    /// so users can express both "allow specific, deny everything else" and
    /// "deny everything, allow specific" without worrying about ordering.
    ///
    /// Returns `None` if no policy matches.
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

    /// Regression test for https://github.com/tailcallhq/forgecode/issues/3085
    ///
    /// The documented `permissions.yaml` example claims to allow writes to
    /// one kind of file (`**/*.rs`) while denying writes to everything else
    /// (`**/*`). The original engine always short-circuited on the first
    /// matching `Deny`, so the broad `deny write` rule silently overrode
    /// the more-specific `allow write`.
    ///
    /// Under the *most-specific-pattern-wins* model, `**/*.rs` (3 literal
    /// chars) is more specific than `**/*` (0 literals), so writes to `.rs`
    /// files are correctly allowed regardless of declaration order.
    #[test]
    fn test_policy_engine_specific_allow_should_win_over_broad_deny() {
        // policies:
        //   - permission: allow
        //     rule:
        //       read: "**/*"
        //   - permission: allow
        //     rule:
        //       write: "**/*.rs"    // specificity=3 (l: "", "/*/", ".rs")
        //   - permission: deny
        //     rule:
        //       write: "**/*"       // specificity=0 (all wildcards)
        //
        // operation: write "test.rs"
        //   → matches allow "**/*.rs"  (spec=3)
        //   → matches deny  "**/*"     (spec=0)
        //   → 3 > 0  →  allow wins
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

    /// Order-independence companion: same allow/deny pair as the issue
    /// scenario but with the broad deny declared *first*. Under
    /// most-specific-wins the result must be identical — declaration order
    /// must not change the outcome.
    #[test]
    fn test_policy_engine_specific_allow_wins_regardless_of_order() {
        // policies:
        //   - permission: deny
        //     rule:
        //       write: "**/*"       // specificity=0
        //   - permission: allow
        //     rule:
        //       write: "**/*.rs"    // specificity=3
        //
        // operation: write "test.rs"
        //   → matches deny  "**/*"  (spec=0)
        //   → matches allow "**/*.rs" (spec=3)
        //   → 3 > 0  →  allow wins  (same result as issue scenario)
        //
        // operation: write "test.py"
        //   → matches deny "**/*" only (spec=0)
        //   → only rule is deny  →  deny wins
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

    /// Carve-out exception: a broad allow with a narrow deny exception.
    /// Under most-specific-wins the deny on the literal filename
    /// (`secret.rs`, all literals) outranks the broader allow on
    /// `**/*.rs`, so writes to `secret.rs` are blocked while other `.rs`
    /// files remain writable.
    #[test]
    fn test_policy_engine_specific_deny_carves_out_exception_in_broad_allow() {
        // policies:
        //   - permission: allow
        //     rule:
        //       write: "**/*.rs"    // specificity=3
        //   - permission: deny
        //     rule:
        //       write: "secret.rs"  // specificity=9 (all literals)
        //
        // operation: write "secret.rs"
        //   → matches allow "**/*.rs" (spec=3)
        //   → matches deny  "secret.rs" (spec=9)
        //   → 9 > 3  →  deny wins  (carve-out respected)
        //
        // operation: write "main.rs"
        //   → matches allow "**/*.rs" (spec=3)
        //   → no deny match
        //   → allow wins
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

    /// Tie-break test: when two equally-specific rules disagree, the more
    /// restrictive permission wins. Both rules use the literal pattern
    /// `secret.rs`, so specificity is identical; `Deny` must override
    /// `Allow` for safety.
    #[test]
    fn test_policy_engine_equal_specificity_prefers_deny() {
        // policies:
        //   - permission: allow
        //     rule:
        //       write: "secret.rs"  // specificity=9
        //   - permission: deny
        //     rule:
        //       write: "secret.rs"  // specificity=9
        //
        // operation: write "secret.rs"
        //   → matches both rules (spec=9 each)
        //   → specificity tie  →  restrictiveness breaks it
        //   → Deny (2) > Allow (0)  →  deny wins  (safe default)
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
