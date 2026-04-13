use std::sync::Arc;

use anyhow::Result;
use forge_app::domain::File;
use forge_app::{
    DirectoryReaderInfra, EnvironmentInfra, FileDiscoveryService, Walker, WalkerInfra,
};

pub struct ForgeDiscoveryService<F> {
    service: Arc<F>,
}

impl<F> ForgeDiscoveryService<F> {
    pub fn new(service: Arc<F>) -> Self {
        Self { service }
    }
}

impl<F: EnvironmentInfra + WalkerInfra> ForgeDiscoveryService<F> {
    async fn discover_with_config(&self, config: Walker) -> Result<Vec<File>> {
        let files = self.service.walk(config).await?;
        Ok(files
            .into_iter()
            .map(|file| File { path: file.path.clone(), is_dir: file.is_dir() })
            .collect())
    }
}

#[async_trait::async_trait]
impl<F: EnvironmentInfra + WalkerInfra + DirectoryReaderInfra + Send + Sync> FileDiscoveryService
    for ForgeDiscoveryService<F>
{
    async fn collect_files(&self, config: Walker) -> Result<Vec<File>> {
        self.discover_with_config(config).await
    }

    async fn list_current_directory(&self) -> Result<Vec<File>> {
        let env = self.service.get_environment();
        let entries = self.service.list_directory_entries(&env.cwd).await?;

        let mut files: Vec<File> = entries
            .into_iter()
            .filter_map(|(path, is_dir)| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .map(|path| File { path: path.to_string(), is_dir })
            })
            .collect();

        // Sort: directories first (alphabetically), then files (alphabetically)
        files.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.path.cmp(&b.path),
        });

        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use forge_app::domain::Environment;
    use forge_app::{EnvironmentInfra, WalkedFile};
    use forge_domain::ConfigOperation;
    use pretty_assertions::assert_eq;

    use super::*;

    #[derive(Clone)]
    struct MockInfra {
        entries: Vec<(PathBuf, bool)>,
        cwd: PathBuf,
    }

    impl MockInfra {
        fn new(entries: Vec<(&str, bool)>, cwd: &str) -> Self {
            Self {
                entries: entries
                    .into_iter()
                    .map(|(name, is_dir)| (PathBuf::from(cwd).join(name), is_dir))
                    .collect(),
                cwd: PathBuf::from(cwd),
            }
        }
    }

    impl EnvironmentInfra for MockInfra {
        type Config = forge_config::ForgeConfig;

        fn get_environment(&self) -> Environment {
            use fake::{Fake, Faker};
            let mut env: Environment = Faker.fake();
            env.cwd = self.cwd.clone();
            env
        }

        fn get_config(&self) -> anyhow::Result<forge_config::ForgeConfig> {
            Ok(forge_config::ForgeConfig::default())
        }

        async fn update_environment(&self, _ops: Vec<ConfigOperation>) -> anyhow::Result<()> {
            unimplemented!()
        }

        fn get_env_var(&self, _key: &str) -> Option<String> {
            None
        }

        fn get_env_vars(&self) -> std::collections::BTreeMap<String, String> {
            std::collections::BTreeMap::new()
        }
    }

    #[async_trait::async_trait]
    impl WalkerInfra for MockInfra {
        async fn walk(&self, _config: Walker) -> Result<Vec<WalkedFile>> {
            Ok(vec![])
        }
    }

    #[async_trait::async_trait]
    impl DirectoryReaderInfra for MockInfra {
        async fn list_directory_entries(
            &self,
            _directory: &std::path::Path,
        ) -> Result<Vec<(PathBuf, bool)>> {
            Ok(self.entries.clone())
        }

        async fn read_directory_files(
            &self,
            _directory: &std::path::Path,
            _pattern: Option<&str>,
        ) -> Result<Vec<(PathBuf, String)>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn test_list_current_directory_sorts_dirs_first() {
        // Fixture: Create entries in random order
        let fixture = MockInfra::new(
            vec![
                ("file1.txt", false),
                ("docs", true),
                ("config.toml", false),
                ("src", true),
                ("README.md", false),
            ],
            "/test",
        );

        let service = ForgeDiscoveryService::new(Arc::new(fixture));

        // Actual: List current directory
        let actual = service.list_current_directory().await.unwrap();

        // Expected: Directories first (sorted), then files (sorted)
        let expected = vec![
            File { path: "docs".to_string(), is_dir: true },
            File { path: "src".to_string(), is_dir: true },
            File { path: "README.md".to_string(), is_dir: false },
            File { path: "config.toml".to_string(), is_dir: false },
            File { path: "file1.txt".to_string(), is_dir: false },
        ];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_list_current_directory_only_files() {
        // Fixture: Only files
        let fixture = MockInfra::new(
            vec![
                ("zebra.txt", false),
                ("alpha.txt", false),
                ("middle.txt", false),
            ],
            "/test",
        );

        let service = ForgeDiscoveryService::new(Arc::new(fixture));

        // Actual: List current directory
        let actual = service.list_current_directory().await.unwrap();

        // Expected: Files sorted alphabetically
        let expected = vec![
            File { path: "alpha.txt".to_string(), is_dir: false },
            File { path: "middle.txt".to_string(), is_dir: false },
            File { path: "zebra.txt".to_string(), is_dir: false },
        ];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_list_current_directory_only_dirs() {
        // Fixture: Only directories
        let fixture = MockInfra::new(
            vec![("zoo", true), ("apple", true), ("berry", true)],
            "/test",
        );

        let service = ForgeDiscoveryService::new(Arc::new(fixture));

        // Actual: List current directory
        let actual = service.list_current_directory().await.unwrap();

        // Expected: Directories sorted alphabetically
        let expected = vec![
            File { path: "apple".to_string(), is_dir: true },
            File { path: "berry".to_string(), is_dir: true },
            File { path: "zoo".to_string(), is_dir: true },
        ];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_list_current_directory_empty() {
        // Fixture: Empty directory
        let fixture = MockInfra::new(vec![], "/test");

        let service = ForgeDiscoveryService::new(Arc::new(fixture));

        // Actual: List current directory
        let actual = service.list_current_directory().await.unwrap();

        // Expected: Empty list
        let expected: Vec<File> = vec![];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_list_current_directory_alphabetical_edge_cases() {
        // Fixture: Test case sensitivity and special characters
        let fixture = MockInfra::new(
            vec![
                (".gitignore", false),
                ("Zulu", true),
                ("_underscore", true),
                ("Apple", true),
                ("zebra.txt", false),
                ("apple.txt", false),
            ],
            "/test",
        );

        let service = ForgeDiscoveryService::new(Arc::new(fixture));

        // Actual: List current directory
        let actual = service.list_current_directory().await.unwrap();

        // Expected: Directories first (case-sensitive sort), then files
        let expected = vec![
            File { path: "Apple".to_string(), is_dir: true },
            File { path: "Zulu".to_string(), is_dir: true },
            File { path: "_underscore".to_string(), is_dir: true },
            File { path: ".gitignore".to_string(), is_dir: false },
            File { path: "apple.txt".to_string(), is_dir: false },
            File { path: "zebra.txt".to_string(), is_dir: false },
        ];

        assert_eq!(actual, expected);
    }
}
