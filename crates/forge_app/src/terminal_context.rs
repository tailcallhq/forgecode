use std::sync::Arc;

use forge_domain::{TerminalCommand, TerminalContext};

use crate::EnvironmentInfra;

/// Environment variable exported by the zsh plugin containing
/// `\x1F`-separated (ASCII Unit Separator) command strings.
pub const ENV_TERM_COMMANDS: &str = "_FORGE_TERM_COMMANDS";

/// Environment variable exported by the zsh plugin containing
/// `\x1F`-separated exit codes corresponding to [`ENV_TERM_COMMANDS`].
pub const ENV_TERM_EXIT_CODES: &str = "_FORGE_TERM_EXIT_CODES";

/// Environment variable exported by the zsh plugin containing
/// `\x1F`-separated Unix timestamps corresponding to [`ENV_TERM_COMMANDS`].
pub const ENV_TERM_TIMESTAMPS: &str = "_FORGE_TERM_TIMESTAMPS";

/// The separator used to join and split environment variable lists.
///
/// ASCII Unit Separator (`\x1F`) is chosen because it cannot appear in
/// shell command strings, paths, URLs, or exit codes — unlike `:` which
/// is common in all of those.
pub const ENV_LIST_SEPARATOR: char = '\x1F';

/// Service that reads terminal context from environment variables exported by
/// the zsh plugin and constructs a structured [`TerminalContext`].
///
/// The zsh plugin exports three `\x1F`-separated environment variables before
/// invoking forge:
/// - [`ENV_TERM_COMMANDS`]   — the command strings
/// - [`ENV_TERM_EXIT_CODES`] — the corresponding exit codes
/// - [`ENV_TERM_TIMESTAMPS`] — the corresponding Unix timestamps
#[derive(Clone)]
pub struct TerminalContextService<S>(Arc<S>);

impl<S> TerminalContextService<S> {
    /// Creates a new `TerminalContextService` backed by the provided
    /// infrastructure.
    pub fn new(infra: Arc<S>) -> Self {
        Self(infra)
    }
}

impl<S: EnvironmentInfra<Config = forge_config::ForgeConfig>> TerminalContextService<S> {
    /// Reads the terminal context from environment variables.
    ///
    /// Commands are sorted by timestamp (oldest first, most recent last) and
    /// limited to the number specified by [`forge_config::ForgeConfig::max_terminal_commands`].
    /// When `max_terminal_commands` is `0`, all captured commands are included.
    ///
    /// Returns `None` if none of the required variables are set or if no
    /// commands were recorded.
    pub fn get_terminal_context(&self) -> Option<TerminalContext> {
        let commands_raw = self.0.get_env_var(ENV_TERM_COMMANDS)?;

        let commands: Vec<String> = split_env_list(&commands_raw);
        if commands.is_empty() {
            return None;
        }

        let exit_codes_raw = self.0.get_env_var(ENV_TERM_EXIT_CODES).unwrap_or_default();
        let timestamps_raw = self.0.get_env_var(ENV_TERM_TIMESTAMPS).unwrap_or_default();

        let exit_codes: Vec<i32> = split_env_list(&exit_codes_raw)
            .iter()
            .map(|s| s.parse::<i32>().unwrap_or(0))
            .collect();

        let timestamps: Vec<u64> = split_env_list(&timestamps_raw)
            .iter()
            .map(|s| s.parse::<u64>().unwrap_or(0))
            .collect();
        // Zip the three lists together; pad missing exit codes/timestamps with 0.
        // The outer zip() truncates to the length of `commands`, so the
        // repeat() padding never produces extra entries.
        let mut entries: Vec<TerminalCommand> = commands
            .into_iter()
            .zip(exit_codes.into_iter().chain(std::iter::repeat(0)))
            .zip(timestamps.into_iter().chain(std::iter::repeat(0)))
            .map(|((command, exit_code), timestamp)| TerminalCommand {
                command,
                exit_code,
                timestamp,
            })
            .collect();

        // Sort by timestamp so the most recent command appears last.
        entries.sort_by_key(|e| e.timestamp);

        // Limit to the configured maximum number of commands. When the limit is
        // 0 (the default), all commands are included.
        let max = self
            .0
            .get_config()
            .map(|c| c.max_terminal_commands)
            .unwrap_or(0);
        if max > 0 && entries.len() > max {
            entries = entries.split_off(entries.len() - max);
        }

        if entries.is_empty() {
            None
        } else {
            Some(TerminalContext { commands: entries })
        }
    }
}

/// Splits an `\x1F`-separated (ASCII Unit Separator) environment variable
/// value into a list of strings, filtering out any empty segments.
pub fn split_env_list(raw: &str) -> Vec<String> {
    raw.split(ENV_LIST_SEPARATOR)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use forge_domain::{Environment, TerminalCommand, TerminalContext};
    use pretty_assertions::assert_eq;

    use super::*;

    struct MockInfra {
        env_vars: BTreeMap<String, String>,
        config: forge_config::ForgeConfig,
    }

    impl MockInfra {
        fn new(vars: &[(&str, &str)]) -> Arc<Self> {
            Arc::new(Self {
                env_vars: vars
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                config: forge_config::ForgeConfig::default(),
            })
        }

        fn new_with_config(vars: &[(&str, &str)], config: forge_config::ForgeConfig) -> Arc<Self> {
            Arc::new(Self {
                env_vars: vars
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                config,
            })
        }
    }

    impl crate::EnvironmentInfra for MockInfra {
        type Config = forge_config::ForgeConfig;

        fn get_environment(&self) -> Environment {
            use fake::{Fake, Faker};
            Faker.fake()
        }

        fn get_config(&self) -> anyhow::Result<forge_config::ForgeConfig> {
            Ok(self.config.clone())
        }

        async fn update_environment(
            &self,
            _ops: Vec<forge_domain::ConfigOperation>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn get_env_var(&self, key: &str) -> Option<String> {
            self.env_vars.get(key).cloned()
        }

        fn get_env_vars(&self) -> BTreeMap<String, String> {
            self.env_vars.clone()
        }
    }

    #[test]
    fn test_no_env_vars_returns_none() {
        let fixture = TerminalContextService::new(MockInfra::new(&[]));
        let actual = fixture.get_terminal_context();
        assert_eq!(actual, None);
    }

    #[test]
    fn test_empty_commands_returns_none() {
        let fixture = TerminalContextService::new(MockInfra::new(&[(ENV_TERM_COMMANDS, "")]));
        let actual = fixture.get_terminal_context();
        assert_eq!(actual, None);
    }

    #[test]
    fn test_single_command_no_extras() {
        let fixture =
            TerminalContextService::new(MockInfra::new(&[(ENV_TERM_COMMANDS, "cargo build")]));
        let actual = fixture.get_terminal_context();
        let expected = Some(TerminalContext {
            commands: vec![TerminalCommand {
                command: "cargo build".to_string(),
                exit_code: 0,
                timestamp: 0,
            }],
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_multiple_commands_with_exit_codes_and_timestamps() {
        let sep = ENV_LIST_SEPARATOR;
        let fixture = TerminalContextService::new(MockInfra::new(&[
            (ENV_TERM_COMMANDS, &format!("ls{sep}cargo test{sep}git status")),
            (ENV_TERM_EXIT_CODES, &format!("0{sep}1{sep}0")),
            (ENV_TERM_TIMESTAMPS, &format!("1700000001{sep}1700000002{sep}1700000003")),
        ]));
        let actual = fixture.get_terminal_context();
        let expected = Some(TerminalContext {
            commands: vec![
                TerminalCommand {
                    command: "ls".to_string(),
                    exit_code: 0,
                    timestamp: 1700000001,
                },
                TerminalCommand {
                    command: "cargo test".to_string(),
                    exit_code: 1,
                    timestamp: 1700000002,
                },
                TerminalCommand {
                    command: "git status".to_string(),
                    exit_code: 0,
                    timestamp: 1700000003,
                },
            ],
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_split_env_list_empty() {
        let actual = split_env_list("");
        let expected: Vec<String> = vec![];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_split_env_list_single() {
        let actual = split_env_list("hello");
        let expected = vec!["hello".to_string()];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_split_env_list_multiple() {
        let sep = ENV_LIST_SEPARATOR;
        let actual = split_env_list(&format!("a{sep}b{sep}c"));
        let expected = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_split_env_list_command_with_colon() {
        // Commands containing `:` (e.g. URLs, port mappings) must not be split.
        let sep = ENV_LIST_SEPARATOR;
        let actual = split_env_list(&format!("curl https://example.com{sep}docker run -p 8080:80 nginx"));
        let expected = vec![
            "curl https://example.com".to_string(),
            "docker run -p 8080:80 nginx".to_string(),
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_commands_sorted_by_timestamp_oldest_first() {
        // Supply commands in reverse-timestamp order to confirm sorting is applied.
        let sep = ENV_LIST_SEPARATOR;
        let fixture = TerminalContextService::new(MockInfra::new(&[
            (ENV_TERM_COMMANDS, &format!("git status{sep}cargo test{sep}ls")),
            (ENV_TERM_EXIT_CODES, &format!("0{sep}1{sep}0")),
            (ENV_TERM_TIMESTAMPS, &format!("1700000003{sep}1700000002{sep}1700000001")),
        ]));
        let actual = fixture.get_terminal_context();
        let expected = Some(TerminalContext {
            commands: vec![
                TerminalCommand {
                    command: "ls".to_string(),
                    exit_code: 0,
                    timestamp: 1700000001,
                },
                TerminalCommand {
                    command: "cargo test".to_string(),
                    exit_code: 1,
                    timestamp: 1700000002,
                },
                TerminalCommand {
                    command: "git status".to_string(),
                    exit_code: 0,
                    timestamp: 1700000003,
                },
            ],
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_max_terminal_commands_limits_to_most_recent() {
        let sep = ENV_LIST_SEPARATOR;
        let config = forge_config::ForgeConfig {
            max_terminal_commands: 2,
            ..Default::default()
        };
        let fixture = TerminalContextService::new(MockInfra::new_with_config(
            &[
                (ENV_TERM_COMMANDS, &format!("ls{sep}cargo test{sep}git status")),
                (ENV_TERM_EXIT_CODES, &format!("0{sep}1{sep}0")),
                (ENV_TERM_TIMESTAMPS, &format!("1700000001{sep}1700000002{sep}1700000003")),
            ],
            config,
        ));
        let actual = fixture.get_terminal_context();
        // Only the 2 most recent commands (by timestamp) should be kept, oldest first.
        let expected = Some(TerminalContext {
            commands: vec![
                TerminalCommand {
                    command: "cargo test".to_string(),
                    exit_code: 1,
                    timestamp: 1700000002,
                },
                TerminalCommand {
                    command: "git status".to_string(),
                    exit_code: 0,
                    timestamp: 1700000003,
                },
            ],
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_max_terminal_commands_zero_includes_all() {
        // max_terminal_commands = 0 (default) means no limit.
        let sep = ENV_LIST_SEPARATOR;
        let fixture = TerminalContextService::new(MockInfra::new(&[
            (ENV_TERM_COMMANDS, &format!("ls{sep}cargo test{sep}git status")),
            (ENV_TERM_EXIT_CODES, &format!("0{sep}1{sep}0")),
            (ENV_TERM_TIMESTAMPS, &format!("1700000001{sep}1700000002{sep}1700000003")),
        ]));
        let actual = fixture.get_terminal_context();
        assert_eq!(actual.unwrap().commands.len(), 3);
    }
}
