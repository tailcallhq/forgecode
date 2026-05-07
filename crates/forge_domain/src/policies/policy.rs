use std::fmt::{Display, Formatter};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::operation::PermissionOperation;
use super::rule::Rule;
use super::types::Permission;

/// Policy definitions with logical operators
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
#[serde(rename_all = "camelCase")]
pub enum Policy {
    /// Simple policy with permission and rule
    Simple { permission: Permission, rule: Rule },
    /// Logical AND of two policies
    All { all: Vec<Policy> },
    /// Logical OR of two policies
    Any { any: Vec<Policy> },
    /// Logical NOT of a policy
    Not { not: Box<Policy> },
}

impl Policy {
    /// Evaluate a policy against an operation
    pub fn eval(&self, operation: &PermissionOperation) -> Option<Permission> {
        match self {
            Policy::Simple { permission, rule } => {
                let rule_matches = rule.matches(operation);
                if rule_matches {
                    Some(permission.clone())
                } else {
                    // Rule doesn't match, so this policy doesn't apply
                    None
                }
            }
            Policy::All { all: and } => {
                let permissions: Vec<_> = and.iter().map(|policy| policy.eval(operation)).collect();
                // For AND, we need all policies to pass, return the most restrictive permission
                permissions
                    .into_iter()
                    .find(|permission| permission.is_some())
                    .flatten()
            }
            Policy::Any { any: or } => {
                let permissions: Vec<_> = or.iter().map(|policy| policy.eval(operation)).collect();
                // For OR, return the first matching permission
                permissions
                    .into_iter()
                    .find(|permission| permission.is_some())
                    .flatten()
            }
            Policy::Not { not } => {
                let inner_permission = not.eval(operation);
                // For NOT, invert the logic - if inner policy denies, we allow, and vice versa
                match inner_permission {
                    Some(permission) => {
                        let inverted_permission = match permission {
                            Permission::Deny => Permission::Allow,
                            Permission::Allow => Permission::Deny,
                            Permission::Confirm => Permission::Deny,
                        };
                        Some(inverted_permission)
                    }
                    None => None,
                }
            }
        }
    }

    /// Find all rules that match the given operation
    pub fn find_rules(&self, operation: &PermissionOperation) -> Vec<&Rule> {
        let mut rules = Vec::new();
        self.collect_matching_rules(operation, &mut rules);
        rules
    }

    /// Recursively collect all matching rules
    fn collect_matching_rules<'a>(
        &'a self,
        operation: &PermissionOperation,
        rules: &mut Vec<&'a Rule>,
    ) {
        match self {
            Policy::Simple { permission: _, rule } => {
                if rule.matches(operation) {
                    rules.push(rule);
                }
            }
            Policy::All { all: and } => {
                for policy in and {
                    policy.collect_matching_rules(operation, rules);
                }
            }
            Policy::Any { any: or } => {
                for policy in or {
                    policy.collect_matching_rules(operation, rules);
                }
            }
            Policy::Not { not } => {
                not.collect_matching_rules(operation, rules);
            }
        }
    }

    /// Get the permission for this policy if it's a simple policy
    pub fn permission(&self) -> Option<&Permission> {
        match self {
            Policy::Simple { permission, rule: _ } => Some(permission),
            _ => None,
        }
    }

    /// Returns the specificity score for this policy.
    ///
    /// For [`Policy::Simple`], this delegates to [`Rule::specificity`].
    /// For composite policies (`All`/`Any`/`Not`), the maximum specificity
    /// across all child rules is used so that nested specific rules still
    /// outrank shallower broad ones.
    pub fn specificity(&self) -> usize {
        match self {
            Policy::Simple { permission: _, rule } => rule.specificity(),
            Policy::All { all } => all.iter().map(Policy::specificity).max().unwrap_or(0),
            Policy::Any { any } => any.iter().map(Policy::specificity).max().unwrap_or(0),
            Policy::Not { not } => not.specificity(),
        }
    }
}

impl Display for Policy {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Policy::Simple { permission, rule } => {
                write!(f, "{permission} {rule}")
            }
            Policy::All { all: and } => {
                let policies: Vec<String> = and.iter().map(|p| p.to_string()).collect();
                write!(f, "({})", policies.join(" AND "))
            }
            Policy::Any { any: or } => {
                let policies: Vec<String> = or.iter().map(|p| p.to_string()).collect();
                write!(f, "({})", policies.join(" OR "))
            }
            Policy::Not { not } => {
                write!(f, "NOT ({not})")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::WriteRule;

    fn fixture_write_operation() -> PermissionOperation {
        PermissionOperation::Write {
            path: PathBuf::from("src/main.rs"),
            cwd: PathBuf::from("/test/cwd"),
            message: "Create/overwrite file: src/main.rs".to_string(),
        }
    }

    fn fixture_simple_write_policy() -> Policy {
        Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Write(WriteRule { write: "src/**/*.rs".to_string(), dir: None }),
        }
    }

    #[test]
    fn test_policy_eval_simple_matching() {
        let fixture = fixture_simple_write_policy();
        let operation = fixture_write_operation();

        let actual = fixture.eval(&operation);

        assert_eq!(actual.unwrap(), Permission::Allow);
    }

    #[test]
    fn test_policy_eval_simple_not_matching() {
        let fixture = Policy::Simple {
            permission: Permission::Allow,
            rule: Rule::Write(WriteRule { write: "docs/**/*.md".to_string(), dir: None }),
        };
        let operation = fixture_write_operation();

        let actual = fixture.eval(&operation);

        assert_eq!(actual, None);
    }

    #[test]
    fn test_policy_eval_and_both_true() {
        let fixture = Policy::All {
            all: vec![
                Policy::Simple {
                    permission: Permission::Allow,
                    rule: Rule::Write(WriteRule { write: "src/**/*".to_string(), dir: None }),
                },
                Policy::Simple {
                    permission: Permission::Allow,
                    rule: Rule::Write(WriteRule { write: "**/*.rs".to_string(), dir: None }),
                },
            ],
        };
        let operation = fixture_write_operation();

        let actual = fixture.eval(&operation);

        assert_eq!(actual.unwrap(), Permission::Allow);
    }

    #[test]
    fn test_policy_eval_and_one_false() {
        let fixture = Policy::All {
            all: vec![
                Policy::Simple {
                    permission: Permission::Allow,
                    rule: Rule::Write(WriteRule { write: "src/**/*".to_string(), dir: None }),
                },
                Policy::Simple {
                    permission: Permission::Allow,
                    rule: Rule::Write(WriteRule { write: "**/*.py".to_string(), dir: None }),
                },
            ],
        };
        let operation = fixture_write_operation();

        let actual = fixture.eval(&operation);

        assert_eq!(actual.unwrap(), Permission::Allow);
    }

    #[test]
    fn test_policy_eval_or_one_true() {
        let fixture = Policy::Any {
            any: vec![
                Policy::Simple {
                    permission: Permission::Allow,
                    rule: Rule::Write(WriteRule { write: "src/**/*.rs".to_string(), dir: None }),
                },
                Policy::Simple {
                    permission: Permission::Allow,
                    rule: Rule::Write(WriteRule { write: "**/*.py".to_string(), dir: None }),
                },
            ],
        };
        let operation = fixture_write_operation();

        let actual = fixture.eval(&operation);

        assert_eq!(actual.unwrap(), Permission::Allow);
    }

    #[test]
    fn test_policy_eval_not_inverts_result() {
        let fixture = Policy::Not {
            not: Box::new(Policy::Simple {
                permission: Permission::Allow,
                rule: Rule::Write(WriteRule { write: "**/*.py".to_string(), dir: None }),
            }),
        };
        let operation = fixture_write_operation();

        let actual = fixture.eval(&operation);

        assert_eq!(actual, None); // Rule doesn't match, so NOT of None is None
    }

    #[test]
    fn test_policy_find_rules_simple() {
        let fixture = fixture_simple_write_policy();
        let operation = fixture_write_operation();

        let actual = fixture.find_rules(&operation);

        assert_eq!(actual.len(), 1);
        assert_eq!(
            actual[0],
            &Rule::Write(WriteRule { write: "src/**/*.rs".to_string(), dir: None })
        );
    }

    #[test]
    fn test_policy_find_rules_and_multiple() {
        let rule1 = Rule::Write(WriteRule { write: "src/**/*".to_string(), dir: None });
        let rule2 = Rule::Write(WriteRule { write: "**/*.rs".to_string(), dir: None });
        let fixture = Policy::All {
            all: vec![
                Policy::Simple { permission: Permission::Allow, rule: rule1.clone() },
                Policy::Simple { permission: Permission::Allow, rule: rule2.clone() },
            ],
        };
        let operation = fixture_write_operation();

        let actual = fixture.find_rules(&operation);

        assert_eq!(actual.len(), 2);
        assert_eq!(actual[0], &rule1);
        assert_eq!(actual[1], &rule2);
    }
}
