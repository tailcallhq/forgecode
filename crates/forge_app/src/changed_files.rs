use std::sync::Arc;

use forge_domain::{Agent, ContextMessage, Conversation, Role, TextMessage};
use forge_template::Element;

use crate::utils::format_display_path;
use crate::{EnvironmentInfra, FsReadService};

/// Service responsible for detecting externally changed files and rendering
/// notifications
pub struct ChangedFiles<S> {
    services: Arc<S>,
    agent: Agent,
}

impl<S> ChangedFiles<S> {
    /// Creates a new ChangedFiles
    pub fn new(services: Arc<S>, agent: Agent) -> Self {
        Self { services, agent }
    }
}

impl<S: FsReadService + EnvironmentInfra<Config = forge_config::ForgeConfig>> ChangedFiles<S> {
    /// Detects externally changed files and renders a notification if changes
    /// are found. Updates file hashes in conversation metrics to prevent
    /// duplicate notifications.
    pub async fn update_file_stats(&self, mut conversation: Conversation) -> Conversation {
        use crate::file_tracking::FileChangeDetector;
        let parallel_file_reads = self
            .services
            .get_config()
            .map(|c| c.max_parallel_file_reads)
            .unwrap_or(4);
        let changes = FileChangeDetector::new(self.services.clone())
            .detect(&conversation.metrics, parallel_file_reads)
            .await;

        if changes.is_empty() {
            return conversation;
        }

        // Update file hashes to prevent duplicate notifications
        let mut updated_metrics = conversation.metrics.clone();
        for change in &changes {
            if let Some(path_str) = change.path.to_str()
                && let Some(metrics) = updated_metrics.file_operations.get_mut(path_str)
            {
                // Update the file hash
                metrics.content_hash = change.content_hash.clone();
            }
        }
        conversation.metrics = updated_metrics;

        let cwd = self.services.get_environment().cwd;
        let file_elements: Vec<Element> = changes
            .iter()
            .map(|change| {
                let display_path = format_display_path(&change.path, &cwd);
                Element::new("file").text(display_path)
            })
            .collect();

        let notification = Element::new("information")
            .append(
                Element::new("critical")
                    .text("The following files have been modified externally. Please re-read them if its relevant for the task."),
            )
            .append(Element::new("files").append(file_elements))
            .to_string();

        let context = conversation.context.take().unwrap_or_default();

        let message = TextMessage::new(Role::User, notification)
            .droppable(true)
            .model(self.agent.model.clone());

        conversation = conversation.context(context.add_message(ContextMessage::from(message)));

        conversation
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use forge_domain::{
        Agent, AgentId, Context, Conversation, ConversationId, Environment, FileOperation, Metrics,
        ModelId, ProviderId, ToolKind,
    };
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::services::Content;
    use crate::{FsReadService, ReadOutput, compute_hash};

    #[derive(Clone, Default)]
    struct TestServices {
        files: HashMap<String, String>,
        cwd: Option<PathBuf>,
    }

    #[async_trait::async_trait]
    impl FsReadService for TestServices {
        async fn read(
            &self,
            path: String,
            _: Option<u64>,
            _: Option<u64>,
        ) -> anyhow::Result<ReadOutput> {
            self.files
                .get(&path)
                .map(|content| {
                    let hash = compute_hash(content);
                    ReadOutput {
                        content: Content::file(content.clone()),
                        info: forge_domain::FileInfo::new(1, 1, 1, hash),
                    }
                })
                .ok_or_else(|| anyhow::anyhow!(std::io::Error::from(std::io::ErrorKind::NotFound)))
        }
    }

    impl EnvironmentInfra for TestServices {
        type Config = forge_config::ForgeConfig;

        fn get_environment(&self) -> Environment {
            use fake::{Fake, Faker};
            let mut env: Environment = Faker.fake();
            if let Some(cwd) = &self.cwd {
                env.cwd = cwd.clone();
            } else {
                // Use a deterministic cwd that won't match any test paths
                env.cwd = PathBuf::from("/deterministic/test/cwd");
            }
            env
        }

        fn get_config(&self) -> anyhow::Result<forge_config::ForgeConfig> {
            Ok(forge_config::ForgeConfig { max_parallel_file_reads: 4, ..Default::default() })
        }

        async fn update_environment(
            &self,
            _ops: Vec<forge_domain::ConfigOperation>,
        ) -> anyhow::Result<()> {
            unimplemented!()
        }

        fn get_env_var(&self, _key: &str) -> Option<String> {
            None
        }

        fn get_env_vars(&self) -> std::collections::BTreeMap<String, String> {
            std::collections::BTreeMap::new()
        }
    }

    fn fixture(
        files: HashMap<String, String>,
        tracked_files: HashMap<String, Option<String>>,
    ) -> (ChangedFiles<TestServices>, Conversation) {
        fixture_with_cwd(files, tracked_files, None)
    }

    fn fixture_with_cwd(
        files: HashMap<String, String>,
        tracked_files: HashMap<String, Option<String>>,
        cwd: Option<PathBuf>,
    ) -> (ChangedFiles<TestServices>, Conversation) {
        let services = Arc::new(TestServices { files, cwd });
        let agent = Agent::new(
            AgentId::new("test"),
            ProviderId::ANTHROPIC,
            ModelId::new("test-model"),
        );
        let changed_files = ChangedFiles::new(services, agent);

        let mut metrics = Metrics::default();
        for (path, hash) in tracked_files {
            metrics
                .file_operations
                .insert(path, FileOperation::new(ToolKind::Write).content_hash(hash));
        }

        let conversation = Conversation::new(ConversationId::generate()).metrics(metrics);

        (changed_files, conversation)
    }

    #[tokio::test]
    async fn test_no_changes_detected() {
        let content = "hello world";
        let hash = crate::compute_hash(content);

        let (service, mut conversation) = fixture(
            [("/test/file.txt".into(), content.into())].into(),
            [("/test/file.txt".into(), Some(hash))].into(),
        );

        conversation.context = Some(Context::default().add_message(ContextMessage::user(
            "Hey, there!",
            Some(ModelId::new("test")),
        )));

        let actual = service.update_file_stats(conversation.clone()).await;

        assert_eq!(actual.context.clone().unwrap_or_default().messages.len(), 1);
        assert_eq!(actual.context, conversation.context);
    }

    #[tokio::test]
    async fn test_changes_detected_adds_notification() {
        let old_hash = crate::compute_hash("old content");
        let new_content = "new content";

        let (service, conversation) = fixture(
            [("/test/file.txt".into(), new_content.into())].into(),
            [("/test/file.txt".into(), Some(old_hash))].into(),
        );

        let actual = service.update_file_stats(conversation).await;

        let messages = &actual.context.unwrap().messages;
        assert_eq!(messages.len(), 1);
        let message = messages[0].content().unwrap().to_string();
        assert!(message.contains("/test/file.txt"));
        assert!(message.contains("modified externally"));
    }

    #[tokio::test]
    async fn test_updates_content_hash() {
        let old_hash = crate::compute_hash("old content");
        let new_content = "new content";
        let new_hash = crate::compute_hash(new_content);

        let (service, conversation) = fixture(
            [("/test/file.txt".into(), new_content.into())].into(),
            [("/test/file.txt".into(), Some(old_hash))].into(),
        );

        let actual = service.update_file_stats(conversation).await;

        let updated_hash = actual
            .metrics
            .file_operations
            .get("/test/file.txt")
            .and_then(|m| m.content_hash.clone());

        assert_eq!(updated_hash, Some(new_hash));
    }

    #[tokio::test]
    async fn test_multiple_files_changed() {
        let (service, conversation) = fixture(
            [
                ("/test/file1.txt".into(), "new 1".into()),
                ("/test/file2.txt".into(), "new 2".into()),
            ]
            .into(),
            [
                ("/test/file1.txt".into(), Some(crate::compute_hash("old 1"))),
                ("/test/file2.txt".into(), Some(crate::compute_hash("old 2"))),
            ]
            .into(),
        );

        let actual = service.update_file_stats(conversation).await;

        let message = actual.context.unwrap().messages[0]
            .content()
            .unwrap()
            .to_string();

        insta::assert_snapshot!(message);
    }

    #[tokio::test]
    async fn test_uses_relative_paths_within_cwd() {
        let old_hash = crate::compute_hash("old content");
        let new_content = "new content";
        let cwd = PathBuf::from("/home/user/project");
        let absolute_path = "/home/user/project/src/main.rs";

        let (service, conversation) = fixture_with_cwd(
            [(absolute_path.into(), new_content.into())].into(),
            [(absolute_path.into(), Some(old_hash))].into(),
            Some(cwd),
        );

        let actual = service.update_file_stats(conversation).await;

        let message = actual.context.unwrap().messages[0]
            .content()
            .unwrap()
            .to_string();

        let expected = "<information>\n<critical>The following files have been modified externally. Please re-read them if its relevant for the task.</critical>\n<files>\n<file>src/main.rs</file>\n</files>\n</information>";

        assert_eq!(message, expected);
    }
}
