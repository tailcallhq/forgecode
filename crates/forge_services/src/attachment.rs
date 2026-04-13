use std::sync::Arc;

use forge_app::domain::{
    Attachment, AttachmentContent, DirectoryEntry, FileTag, Image, LineNumbers,
};
use forge_app::utils::format_display_path;
use forge_app::{
    AttachmentService, DirectoryReaderInfra, EnvironmentInfra, FileInfoInfra, FileReaderInfra,
};

use crate::range::resolve_range;

#[derive(Clone)]
pub struct ForgeChatRequest<F> {
    infra: Arc<F>,
}

impl<
    F: FileReaderInfra
        + EnvironmentInfra<Config = forge_config::ForgeConfig>
        + FileInfoInfra
        + DirectoryReaderInfra,
> ForgeChatRequest<F>
{
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra }
    }

    async fn prepare_attachments(&self, paths: Vec<FileTag>) -> anyhow::Result<Vec<Attachment>> {
        futures::future::join_all(paths.into_iter().map(|v| self.populate_attachments(v)))
            .await
            .into_iter()
            .collect::<anyhow::Result<Vec<_>>>()
    }

    async fn populate_attachments(&self, tag: FileTag) -> anyhow::Result<Attachment> {
        let mut path = tag.as_ref().to_path_buf();
        let extension = path.extension().map(|v| v.to_string_lossy().to_string());

        if !path.is_absolute() {
            path = self.infra.get_environment().cwd.join(path);
        }

        // Check if path is a directory (exists but is not a file)
        if self.infra.exists(&path).await? && !self.infra.is_file(&path).await? {
            // List all entries (files and directories) efficiently without reading file
            // contents
            let dir_entries = self.infra.list_directory_entries(&path).await?;

            // Create DirectoryEntry for each entry
            let mut entries: Vec<DirectoryEntry> = dir_entries
                .into_iter()
                .map(|(entry_path, is_dir)| {
                    let normalized_path = format_display_path(&entry_path, &path);
                    DirectoryEntry { path: normalized_path, is_dir }
                })
                .collect();

            // Sort entries: directories first, then by name
            entries.sort_by(|a, b| {
                // Directories come before files
                match (a.is_dir, b.is_dir) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.path.cmp(&b.path), // Same type, sort by name
                }
            });

            return Ok(Attachment {
                content: AttachmentContent::DirectoryListing { entries },
                path: path.to_string_lossy().to_string(), // Keep root path absolute
            });
        }

        // Determine file type (text or image with format)
        let mime_type = extension.and_then(|ext| match ext.as_str() {
            "jpeg" | "jpg" => Some("image/jpeg".to_string()),
            "png" => Some("image/png".to_string()),
            "webp" => Some("image/webp".to_string()),
            _ => None,
        });

        //NOTE: Apply the same slicing as file reads for text content
        let content = match mime_type {
            Some(mime_type) => {
                AttachmentContent::Image(Image::new_bytes(self.infra.read(&path).await?, mime_type))
            }
            None => {
                let start = tag.loc.as_ref().and_then(|loc| loc.start);
                let end = tag.loc.as_ref().and_then(|loc| loc.end);
                let max_read_lines = self.infra.get_config()?.max_read_lines;
                let (start_line, end_line) = resolve_range(start, end, max_read_lines);

                // range_read_utf8 returns the range content and FileInfo which
                // carries a content_hash of the **full** file. Using the
                // full-file hash ensures consistency with the external-change
                // detector, which always hashes the entire file.
                let (file_content, file_info) = self
                    .infra
                    .range_read_utf8(&path, start_line, end_line)
                    .await?;

                AttachmentContent::FileContent {
                    content: file_content
                        .to_numbered_from(file_info.start_line as usize)
                        .to_string(),
                    info: file_info,
                }
            }
        };

        Ok(Attachment {
            content,
            path: path.to_string_lossy().to_string(), // Keep root path absolute
        })
    }
}

#[async_trait::async_trait]
impl<
    F: FileReaderInfra
        + EnvironmentInfra<Config = forge_config::ForgeConfig>
        + FileInfoInfra
        + DirectoryReaderInfra,
> AttachmentService for ForgeChatRequest<F>
{
    async fn attachments(&self, url: &str) -> anyhow::Result<Vec<Attachment>> {
        self.prepare_attachments(Attachment::parse_all(url)).await
    }
}

#[cfg(test)]
pub mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use base64::Engine;
    use bytes::Bytes;
    use forge_app::domain::{AttachmentContent, Environment};
    use forge_app::utils::compute_hash;
    use forge_app::{
        AttachmentService, DirectoryReaderInfra, EnvironmentInfra, FileDirectoryInfra,
        FileInfoInfra, FileReaderInfra, FileRemoverInfra, FileWriterInfra,
    };
    use forge_domain::{ConfigOperation, FileInfo};
    use futures::stream;

    use crate::attachment::ForgeChatRequest;

    #[derive(Debug)]
    pub struct MockEnvironmentInfra {}

    impl EnvironmentInfra for MockEnvironmentInfra {
        type Config = forge_config::ForgeConfig;

        fn get_env_var(&self, _key: &str) -> Option<String> {
            None
        }

        fn get_env_vars(&self) -> BTreeMap<String, String> {
            BTreeMap::new()
        }

        fn get_environment(&self) -> Environment {
            use fake::{Fake, Faker};
            let fixture: Environment = Faker.fake();
            fixture.cwd(PathBuf::from("/test")) // Set fixed CWD for predictable tests
        }

        fn get_config(&self) -> anyhow::Result<forge_config::ForgeConfig> {
            Ok(forge_config::ForgeConfig { max_read_lines: 2000, ..Default::default() })
        }

        async fn update_environment(&self, _ops: Vec<ConfigOperation>) -> anyhow::Result<()> {
            unimplemented!()
        }
    }

    impl MockFileService {
        pub fn new() -> Self {
            let mut files = HashMap::new();
            // Add some mock files
            files.insert(
                PathBuf::from("/test/file1.txt"),
                "This is a text file content".to_string(),
            );
            files.insert(
                PathBuf::from("/test/image.png"),
                "mock-binary-content".to_string(),
            );
            files.insert(
                PathBuf::from("/test/image with spaces.jpg"),
                "mock-jpeg-content".to_string(),
            );

            let binary_exts = [
                "exe", "dll", "so", "dylib", "bin", "obj", "o", "class", "pyc", "jar", "war",
                "ear", "zip", "tar", "gz", "rar", "7z", "iso", "img", "pdf", "doc", "docx", "xls",
                "xlsx", "ppt", "pptx", "bmp", "ico", "mp3", "mp4", "avi", "mov", "sqlite", "db",
                "bin",
            ];
            let binary_exts = binary_exts.into_iter().map(|s| s.to_string()).collect();

            Self {
                files: Mutex::new(
                    files
                        .into_iter()
                        .map(|(a, b)| (a, Bytes::from(b)))
                        .collect::<Vec<_>>(),
                ),
                binary_exts,
            }
        }

        pub fn add_file(&self, path: PathBuf, content: String) {
            let mut files = self.files.lock().unwrap();
            files.push((path, Bytes::from_owner(content)));
        }

        pub fn add_dir(&self, path: PathBuf) {
            let mut files = self.files.lock().unwrap();
            files.push((path, Bytes::new()));
        }
    }

    #[async_trait::async_trait]
    impl FileReaderInfra for MockFileService {
        async fn read_utf8(&self, path: &Path) -> anyhow::Result<String> {
            let files = self.files.lock().unwrap();
            match files.iter().find(|v| v.0 == path) {
                Some((_, content)) => {
                    let bytes = content.clone();
                    String::from_utf8(bytes.to_vec())
                        .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in file: {path:?}: {e}"))
                }
                None => Err(anyhow::anyhow!("File not found: {path:?}")),
            }
        }

        fn read_batch_utf8(
            &self,
            _: usize,
            _: Vec<PathBuf>,
        ) -> impl futures::Stream<Item = (PathBuf, anyhow::Result<String>)> + Send {
            stream::empty()
        }

        async fn read(&self, path: &Path) -> anyhow::Result<Vec<u8>> {
            let files = self.files.lock().unwrap();
            match files.iter().find(|v| v.0 == path) {
                Some((_, content)) => Ok(content.to_vec()),
                None => Err(anyhow::anyhow!("File not found: {path:?}")),
            }
        }

        async fn range_read_utf8(
            &self,
            path: &Path,
            start_line: u64,
            end_line: u64,
        ) -> anyhow::Result<(String, FileInfo)> {
            // Read the full content first
            let full_content = self.read_utf8(path).await?;
            let all_lines: Vec<&str> = full_content.lines().collect();

            // Apply range filtering based on parameters
            let start_idx = start_line.saturating_sub(1) as usize;
            let end_idx = if end_line > 0 {
                std::cmp::min(end_line as usize, all_lines.len())
            } else {
                all_lines.len()
            };

            let filtered_lines = if start_idx < all_lines.len() {
                &all_lines[start_idx..end_idx]
            } else {
                &[]
            };

            let filtered_content = filtered_lines.join("\n");
            let actual_start = if filtered_lines.is_empty() {
                0
            } else {
                start_line
            };
            let actual_end = if filtered_lines.is_empty() {
                0
            } else {
                start_idx as u64 + filtered_lines.len() as u64
            };

            // Compute hash from the full file content to match production behaviour
            let content_hash = compute_hash(&full_content);

            Ok((
                filtered_content,
                forge_domain::FileInfo::new(
                    actual_start,
                    actual_end,
                    all_lines.len() as u64,
                    content_hash,
                ),
            ))
        }
    }

    #[derive(Debug)]
    pub struct MockFileService {
        files: Mutex<Vec<(PathBuf, Bytes)>>,
        binary_exts: HashSet<String>,
    }

    #[async_trait::async_trait]
    impl FileRemoverInfra for MockFileService {
        async fn remove(&self, path: &Path) -> anyhow::Result<()> {
            if !self.exists(path).await? {
                return Err(anyhow::anyhow!("File not found: {path:?}"));
            }
            self.files.lock().unwrap().retain(|(p, _)| p != path);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl FileDirectoryInfra for MockFileService {
        async fn create_dirs(&self, path: &Path) -> anyhow::Result<()> {
            self.files
                .lock()
                .unwrap()
                .push((path.to_path_buf(), Bytes::new()));
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl DirectoryReaderInfra for MockFileService {
        async fn list_directory_entries(
            &self,
            directory: &Path,
        ) -> anyhow::Result<Vec<(PathBuf, bool)>> {
            let files = self.files.lock().unwrap();
            let mut results = Vec::new();

            for (path, content) in files.iter() {
                // Check if this entry is a direct child of the directory
                if let Some(parent) = path.parent()
                    && parent == directory
                {
                    // Check if it's a directory (empty bytes)
                    let is_dir = content.is_empty();
                    results.push((path.clone(), is_dir));
                }
            }

            Ok(results)
        }

        async fn read_directory_files(
            &self,
            directory: &Path,
            _pattern: Option<&str>,
        ) -> anyhow::Result<Vec<(PathBuf, String)>> {
            let files = self.files.lock().unwrap();
            let mut results = Vec::new();

            for (path, content) in files.iter() {
                // Check if this entry is a direct child of the directory
                if let Some(parent) = path.parent()
                    && parent == directory
                {
                    let content_str = String::from_utf8(content.to_vec()).unwrap_or_default();
                    results.push((path.clone(), content_str));
                }
            }

            Ok(results)
        }
    }

    #[async_trait::async_trait]
    impl FileWriterInfra for MockFileService {
        async fn write(&self, path: &Path, contents: Bytes) -> anyhow::Result<()> {
            let index = self.files.lock().unwrap().iter().position(|v| v.0 == path);
            if let Some(index) = index {
                self.files.lock().unwrap().remove(index);
            }
            self.files
                .lock()
                .unwrap()
                .push((path.to_path_buf(), contents));
            Ok(())
        }

        async fn append(&self, path: &Path, contents: Bytes) -> anyhow::Result<()> {
            let mut existing = bytes::Bytes::new();
            let index = self.files.lock().unwrap().iter().position(|v| v.0 == path);
            if let Some(index) = index {
                existing = self.files.lock().unwrap().remove(index).1;
            }
            let mut combined = existing.to_vec();
            combined.extend_from_slice(&contents);
            self.files
                .lock()
                .unwrap()
                .push((path.to_path_buf(), combined.into()));
            Ok(())
        }

        async fn write_temp(&self, _: &str, _: &str, content: &str) -> anyhow::Result<PathBuf> {
            let temp_dir = crate::utils::TempDir::new().unwrap();
            let path = temp_dir.path();

            self.write(&path, content.to_string().into()).await?;

            Ok(path)
        }
    }

    #[async_trait::async_trait]
    impl FileInfoInfra for MockFileService {
        async fn is_file(&self, path: &Path) -> anyhow::Result<bool> {
            Ok(self
                .files
                .lock()
                .unwrap()
                .iter()
                .filter(|v| v.0.extension().is_some())
                .any(|(p, _)| p == path))
        }

        async fn is_binary(&self, _path: &Path) -> anyhow::Result<bool> {
            let ext = _path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase());
            Ok(ext.map(|e| self.binary_exts.contains(&e)).unwrap_or(false))
        }

        async fn exists(&self, path: &Path) -> anyhow::Result<bool> {
            Ok(self.files.lock().unwrap().iter().any(|(p, _)| p == path))
        }

        async fn file_size(&self, path: &Path) -> anyhow::Result<u64> {
            let files = self.files.lock().unwrap();
            if let Some((_, content)) = files.iter().find(|(p, _)| p == path) {
                Ok(content.len() as u64)
            } else {
                Err(anyhow::anyhow!("File not found: {}", path.display()))
            }
        }
    }

    // Create a composite mock service that implements the required traits
    #[derive(Debug, Clone)]
    pub struct MockCompositeService {
        file_service: Arc<MockFileService>,
        env_service: Arc<MockEnvironmentInfra>,
    }

    impl MockCompositeService {
        pub fn new() -> Self {
            Self {
                file_service: Arc::new(MockFileService::new()),
                env_service: Arc::new(MockEnvironmentInfra {}),
            }
        }

        pub fn add_file(&self, path: PathBuf, content: String) {
            self.file_service.add_file(path, content);
        }
    }

    #[async_trait::async_trait]
    impl FileReaderInfra for MockCompositeService {
        async fn read_utf8(&self, path: &Path) -> anyhow::Result<String> {
            self.file_service.read_utf8(path).await
        }

        fn read_batch_utf8(
            &self,
            batch_size: usize,
            paths: Vec<PathBuf>,
        ) -> impl futures::Stream<Item = (PathBuf, anyhow::Result<String>)> + Send {
            self.file_service.read_batch_utf8(batch_size, paths)
        }

        async fn read(&self, path: &Path) -> anyhow::Result<Vec<u8>> {
            self.file_service.read(path).await
        }

        async fn range_read_utf8(
            &self,
            path: &Path,
            start_line: u64,
            end_line: u64,
        ) -> anyhow::Result<(String, forge_domain::FileInfo)> {
            self.file_service
                .range_read_utf8(path, start_line, end_line)
                .await
        }
    }

    impl EnvironmentInfra for MockCompositeService {
        type Config = forge_config::ForgeConfig;

        fn get_environment(&self) -> Environment {
            self.env_service.get_environment()
        }

        fn get_config(&self) -> anyhow::Result<forge_config::ForgeConfig> {
            self.env_service.get_config()
        }

        fn update_environment(
            &self,
            ops: Vec<ConfigOperation>,
        ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send {
            let env_service = self.env_service.clone();
            async move { env_service.update_environment(ops).await }
        }

        fn get_env_var(&self, key: &str) -> Option<String> {
            self.env_service.get_env_var(key)
        }

        fn get_env_vars(&self) -> BTreeMap<String, String> {
            self.env_service.get_env_vars()
        }
    }

    #[async_trait::async_trait]
    impl FileInfoInfra for MockCompositeService {
        async fn is_binary(&self, path: &Path) -> anyhow::Result<bool> {
            self.file_service.is_binary(path).await
        }

        async fn is_file(&self, path: &Path) -> anyhow::Result<bool> {
            self.file_service.is_file(path).await
        }

        async fn exists(&self, path: &Path) -> anyhow::Result<bool> {
            self.file_service.exists(path).await
        }

        async fn file_size(&self, path: &Path) -> anyhow::Result<u64> {
            self.file_service.file_size(path).await
        }
    }

    #[async_trait::async_trait]
    impl DirectoryReaderInfra for MockCompositeService {
        async fn list_directory_entries(
            &self,
            directory: &Path,
        ) -> anyhow::Result<Vec<(PathBuf, bool)>> {
            self.file_service.list_directory_entries(directory).await
        }

        async fn read_directory_files(
            &self,
            directory: &Path,
            pattern: Option<&str>,
        ) -> anyhow::Result<Vec<(PathBuf, String)>> {
            self.file_service
                .read_directory_files(directory, pattern)
                .await
        }
    }

    #[tokio::test]
    async fn test_add_url_with_text_file() {
        // Setup
        let infra = Arc::new(MockCompositeService::new());
        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test with a text file path in chat message
        let url = "@[/test/file1.txt]".to_string();

        // Execute
        let attachments = chat_request.attachments(&url).await.unwrap();

        // Assert
        // Text files should be included in the attachments
        assert_eq!(attachments.len(), 1);
        let attachment = attachments.first().unwrap();
        assert_eq!(attachment.path, "/test/file1.txt");

        // Check that the content contains our original text and has range information
        assert!(attachment.content.contains("This is a text file content"));
    }

    #[tokio::test]
    async fn test_add_url_with_image() {
        // Setup
        let infra = Arc::new(MockCompositeService::new());
        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test with an image file
        let url = "@[/test/image.png]".to_string();

        // Execute
        let attachments = chat_request.attachments(&url).await.unwrap();

        // Assert
        assert_eq!(attachments.len(), 1);
        let attachment = attachments.first().unwrap();
        assert_eq!(attachment.path, "/test/image.png");

        // Base64 content should be the encoded mock binary content with proper data URI
        // format
        let expected_base64 =
            base64::engine::general_purpose::STANDARD.encode("mock-binary-content");
        assert_eq!(
            attachment.content.as_image().unwrap().url().as_str(),
            format!("data:image/png;base64,{expected_base64}")
        );
    }

    #[tokio::test]
    async fn test_add_url_with_jpg_image_with_spaces() {
        // Setup
        let infra = Arc::new(MockCompositeService::new());
        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test with an image file that has spaces in the path
        let url = "@[/test/image with spaces.jpg]".to_string();

        // Execute
        let attachments = chat_request.attachments(&url).await.unwrap();

        // Assert
        assert_eq!(attachments.len(), 1);
        let attachment = attachments.first().unwrap();
        assert_eq!(attachment.path, "/test/image with spaces.jpg");

        // Base64 content should be the encoded mock jpeg content with proper data URI
        // format
        let expected_base64 = base64::engine::general_purpose::STANDARD.encode("mock-jpeg-content");
        assert_eq!(
            attachment.content.as_image().unwrap().url().as_str(),
            format!("data:image/jpeg;base64,{expected_base64}")
        );
    }

    #[tokio::test]
    async fn test_add_url_with_multiple_files() {
        // Setup
        let infra = Arc::new(MockCompositeService::new());

        // Add an extra file to our mock service
        infra.add_file(
            PathBuf::from("/test/file2.txt"),
            "This is another text file".to_string(),
        );

        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test with multiple files mentioned
        let url = "@[/test/file1.txt] @[/test/file2.txt] @[/test/image.png]".to_string();

        // Execute
        let attachments = chat_request.attachments(&url).await.unwrap();

        // Assert
        // All files should be included in the attachments
        assert_eq!(attachments.len(), 3);

        // Verify that each expected file is in the attachments
        let has_file1 = attachments.iter().any(|a| {
            a.path == "/test/file1.txt"
                && matches!(a.content, AttachmentContent::FileContent { .. })
        });
        let has_file2 = attachments.iter().any(|a| {
            a.path == "/test/file2.txt"
                && matches!(a.content, AttachmentContent::FileContent { .. })
        });
        let has_image = attachments.iter().any(|a| {
            a.path == "/test/image.png" && matches!(a.content, AttachmentContent::Image(_))
        });

        assert!(has_file1, "Missing file1.txt in attachments");
        assert!(has_file2, "Missing file2.txt in attachments");
        assert!(has_image, "Missing image.png in attachments");
    }

    #[tokio::test]
    async fn test_add_url_with_nonexistent_file() {
        // Setup
        let infra = Arc::new(MockCompositeService::new());
        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test with a file that doesn't exist
        let url = "@[/test/nonexistent.txt]".to_string();

        // Execute - Let's handle the error properly
        let result = chat_request.attachments(&url).await;

        // Assert - we expect an error for nonexistent files
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("File not found"));
    }

    #[tokio::test]
    async fn test_add_url_empty() {
        // Setup
        let infra = Arc::new(MockCompositeService::new());
        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test with an empty message
        let url = "".to_string();

        // Execute
        let attachments = chat_request.attachments(&url).await.unwrap();

        // Assert - no attachments
        assert_eq!(attachments.len(), 0);
    }

    #[tokio::test]
    async fn test_add_url_with_unsupported_extension() {
        // Setup
        let infra = Arc::new(MockCompositeService::new());

        // Add a file with unsupported extension
        infra.add_file(
            PathBuf::from("/test/unknown.xyz"),
            "Some content".to_string(),
        );

        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test with the file
        let url = "@[/test/unknown.xyz]".to_string();

        // Execute
        let attachments = chat_request.attachments(&url).await.unwrap();

        // Assert - should be treated as text
        assert_eq!(attachments.len(), 1);
        let attachment = attachments.first().unwrap();
        assert_eq!(attachment.path, "/test/unknown.xyz");

        // Check that the content contains our original text and has range information
        assert!(attachment.content.contains("Some content"));
    }

    #[tokio::test]
    async fn test_attachment_range_information() {
        // Setup
        let infra = Arc::new(MockCompositeService::new());

        // Add a multi-line file to test range information
        infra.add_file(
            PathBuf::from("/test/multiline.txt"),
            "Line 1\nLine 2\nLine 3\nLine 4\nLine 5".to_string(),
        );

        let chat_request = ForgeChatRequest::new(infra.clone());
        let url = "@[/test/multiline.txt]".to_string();

        // Execute
        let attachments = chat_request.attachments(&url).await.unwrap();

        // Assert
        assert_eq!(attachments.len(), 1);
        let attachment = attachments.first().unwrap();

        // Verify range information is populated
        let range_info = attachment.content.range_info();
        assert!(
            range_info.is_some(),
            "Range information should be present for file content"
        );

        let (start_line, end_line, total_lines) = range_info.unwrap();
        assert_eq!(start_line, 1, "Start line should be 1");
        assert!(end_line >= start_line, "End line should be >= start line");
        assert!(total_lines >= end_line, "Total lines should be >= end line");

        // Verify content is accessible through helper method
        let file_content = attachment.content.file_content();
        assert!(file_content.is_some(), "File content should be accessible");
        assert!(
            file_content.unwrap().contains("Line 1"),
            "Should contain file content"
        );
    }

    // Range functionality tests
    #[tokio::test]
    async fn test_range_functionality_single_line() {
        let infra = Arc::new(MockCompositeService::new());

        // Add a multi-line test file
        infra.add_file(
            PathBuf::from("/test/multiline.txt"),
            "Line 1\nLine 2\nLine 3\nLine 4\nLine 5".to_string(),
        );

        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test reading line 2 only
        let url = "@[/test/multiline.txt:2:2]";
        let attachments = chat_request.attachments(url).await.unwrap();

        assert_eq!(attachments.len(), 1);
        assert_eq!(
            attachments[0].content,
            AttachmentContent::FileContent {
                content: "2:Line 2".to_string(),
                info: FileInfo::new(
                    2,
                    2,
                    5,
                    compute_hash("Line 1\nLine 2\nLine 3\nLine 4\nLine 5")
                ),
            }
        );
    }

    #[tokio::test]
    async fn test_range_functionality_multiple_lines() {
        let infra = Arc::new(MockCompositeService::new());

        // Add a multi-line test file
        infra.add_file(
            PathBuf::from("/test/range_test.txt"),
            "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6".to_string(),
        );

        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test reading lines 2-4
        let url = "@[/test/range_test.txt:2:4]";
        let attachments = chat_request.attachments(url).await.unwrap();

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments.len(), 1);
        assert_eq!(
            attachments[0].content,
            AttachmentContent::FileContent {
                content: "2:Line 2\n3:Line 3\n4:Line 4".to_string(),
                info: FileInfo::new(
                    2,
                    4,
                    6,
                    compute_hash("Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6")
                ),
            }
        );
    }

    #[tokio::test]
    async fn test_range_functionality_from_start() {
        let infra = Arc::new(MockCompositeService::new());

        infra.add_file(
            PathBuf::from("/test/start_range.txt"),
            "First\nSecond\nThird\nFourth".to_string(),
        );

        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test reading from start to line 2
        let url = "@[/test/start_range.txt:1:2]";
        let attachments = chat_request.attachments(url).await.unwrap();
        assert_eq!(
            attachments[0].content,
            AttachmentContent::FileContent {
                content: "1:First\n2:Second".to_string(),
                info: FileInfo::new(1, 2, 4, compute_hash("First\nSecond\nThird\nFourth")),
            }
        );
    }

    #[tokio::test]
    async fn test_range_functionality_to_end() {
        let infra = Arc::new(MockCompositeService::new());

        infra.add_file(
            PathBuf::from("/test/end_range.txt"),
            "Alpha\nBeta\nGamma\nDelta\nEpsilon".to_string(),
        );

        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test reading from line 3 to end
        let url = "@[/test/end_range.txt:3:5]";
        let attachments = chat_request.attachments(url).await.unwrap();
        assert_eq!(
            attachments[0].content,
            AttachmentContent::FileContent {
                content: "3:Gamma\n4:Delta\n5:Epsilon".to_string(),
                info: FileInfo::new(3, 5, 5, compute_hash("Alpha\nBeta\nGamma\nDelta\nEpsilon")),
            }
        );
    }

    #[tokio::test]
    async fn test_range_functionality_edge_cases() {
        let infra = Arc::new(MockCompositeService::new());

        infra.add_file(
            PathBuf::from("/test/edge_case.txt"),
            "Only line".to_string(),
        );

        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test reading beyond file length
        let url = "@[/test/edge_case.txt:1:10]";
        let attachments = chat_request.attachments(url).await.unwrap();
        assert_eq!(
            attachments[0].content,
            AttachmentContent::FileContent {
                content: "1:Only line".to_string(),
                info: FileInfo::new(1, 1, 1, compute_hash("Only line")),
            }
        );
    }

    #[tokio::test]
    async fn test_range_functionality_combined_with_multiple_files() {
        let infra = Arc::new(MockCompositeService::new());

        infra.add_file(PathBuf::from("/test/file_a.txt"), "A1\nA2\nA3".to_string());
        infra.add_file(
            PathBuf::from("/test/file_b.txt"),
            "B1\nB2\nB3\nB4".to_string(),
        );

        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test multiple files with different ranges
        let url = "Check @[/test/file_a.txt:1:2] and @[/test/file_b.txt:3:4]";
        let attachments = chat_request.attachments(url).await.unwrap();

        assert_eq!(attachments.len(), 2);
        assert_eq!(
            attachments[0].content,
            AttachmentContent::FileContent {
                content: "1:A1\n2:A2".to_string(),
                info: FileInfo::new(1, 2, 3, compute_hash("A1\nA2\nA3")),
            }
        );
        assert_eq!(
            attachments[1].content,
            AttachmentContent::FileContent {
                content: "3:B3\n4:B4".to_string(),
                info: FileInfo::new(3, 4, 4, compute_hash("B1\nB2\nB3\nB4")),
            }
        );
    }

    #[tokio::test]
    async fn test_range_functionality_preserves_metadata() {
        let infra = Arc::new(MockCompositeService::new());

        infra.add_file(
            PathBuf::from("/test/metadata_test.txt"),
            "Meta1\nMeta2\nMeta3\nMeta4\nMeta5\nMeta6\nMeta7".to_string(),
        );

        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test that metadata is preserved correctly with ranges
        let url = "@[/test/metadata_test.txt:3:5]";
        let attachments = chat_request.attachments(url).await.unwrap();

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].path, "/test/metadata_test.txt");
        assert_eq!(
            attachments[0].content,
            AttachmentContent::FileContent {
                content: "3:Meta3\n4:Meta4\n5:Meta5".to_string(),
                info: FileInfo::new(
                    3,
                    5,
                    7,
                    compute_hash("Meta1\nMeta2\nMeta3\nMeta4\nMeta5\nMeta6\nMeta7")
                ),
            }
        );
    }

    #[tokio::test]
    async fn test_range_functionality_vs_full_file() {
        let infra = Arc::new(MockCompositeService::new());

        infra.add_file(
            PathBuf::from("/test/comparison.txt"),
            "Full1\nFull2\nFull3\nFull4\nFull5".to_string(),
        );

        let chat_request = ForgeChatRequest::new(infra.clone());

        // All reads of the same file (full or ranged) should produce the same
        // content_hash, so the external-change detector can correctly identify
        // that the file has not been modified between reads.
        let full_file_hash = compute_hash("Full1\nFull2\nFull3\nFull4\nFull5");

        let url_full = "@[/test/comparison.txt]";
        let url_range = "@[/test/comparison.txt:2:4]";
        let url_range_start = "@[/test/comparison.txt:2]";

        let attachments_full = chat_request.attachments(url_full).await.unwrap();
        let attachments_range = chat_request.attachments(url_range).await.unwrap();
        let attachments_range_start = chat_request.attachments(url_range_start).await.unwrap();

        assert_eq!(attachments_full.len(), 1);
        assert_eq!(
            attachments_full[0].content,
            AttachmentContent::FileContent {
                content: "1:Full1\n2:Full2\n3:Full3\n4:Full4\n5:Full5".to_string(),
                info: FileInfo::new(1, 5, 5, full_file_hash.clone()),
            }
        );

        assert_eq!(attachments_range.len(), 1);
        assert_eq!(
            attachments_range[0].content,
            AttachmentContent::FileContent {
                content: "2:Full2\n3:Full3\n4:Full4".to_string(),
                info: FileInfo::new(2, 4, 5, full_file_hash.clone()),
            }
        );

        assert_eq!(attachments_range_start.len(), 1);
        assert_eq!(
            attachments_range_start[0].content,
            AttachmentContent::FileContent {
                content: "2:Full2\n3:Full3\n4:Full4\n5:Full5".to_string(),
                info: FileInfo::new(2, 5, 5, full_file_hash),
            }
        );
    }

    #[tokio::test]
    async fn test_add_url_with_directory() {
        let infra = Arc::new(MockCompositeService::new());

        // Add directory, files, and subdirectory
        infra.file_service.add_dir(PathBuf::from("/test/mydir"));
        infra.add_file(
            PathBuf::from("/test/mydir/file1.txt"),
            "Content of file1".to_string(),
        );
        infra.add_file(
            PathBuf::from("/test/mydir/file2.txt"),
            "Content of file2".to_string(),
        );
        infra
            .file_service
            .add_dir(PathBuf::from("/test/mydir/subdir"));

        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test with directory path
        let url = "@[/test/mydir]";
        let attachments = chat_request.attachments(url).await.unwrap();

        // Should return a single DirectoryListing attachment
        assert_eq!(attachments.len(), 1);
        let attachment = attachments.first().unwrap();

        // Verify it's a directory listing with relative paths
        match &attachment.content {
            AttachmentContent::DirectoryListing { entries } => {
                // Should contain 2 files and 1 subdirectory (3 total)
                assert_eq!(entries.len(), 3);

                // Check for files (is_dir = false)
                let file1 = entries.iter().find(|e| e.path == "file1.txt").unwrap();
                assert!(!file1.is_dir);

                let file2 = entries.iter().find(|e| e.path == "file2.txt").unwrap();
                assert!(!file2.is_dir);

                // Check for subdirectory (is_dir = true)
                let subdir = entries.iter().find(|e| e.path == "subdir").unwrap();
                assert!(subdir.is_dir);
            }
            _ => panic!("Expected DirectoryListing attachment"),
        }

        // Path should be absolute (root level)
        assert_eq!(attachment.path, "/test/mydir");
    }

    #[tokio::test]
    async fn test_add_url_with_empty_directory() {
        let infra = Arc::new(MockCompositeService::new());

        // Add empty directory
        infra.file_service.add_dir(PathBuf::from("/test/emptydir"));

        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test with empty directory path
        let url = "@[/test/emptydir]";
        let attachments = chat_request.attachments(url).await.unwrap();

        // Should return a single DirectoryListing attachment with empty files list
        assert_eq!(attachments.len(), 1);
        let attachment = attachments.first().unwrap();

        match &attachment.content {
            AttachmentContent::DirectoryListing { entries } => {
                assert_eq!(entries.len(), 0);
            }
            _ => panic!("Expected DirectoryListing attachment"),
        }

        // Path should be absolute (root level)
        assert_eq!(attachment.path, "/test/emptydir");
    }

    #[tokio::test]
    async fn test_add_url_with_mixed_files_and_directory() {
        let infra = Arc::new(MockCompositeService::new());

        // Add directory with files
        infra.file_service.add_dir(PathBuf::from("/test/mixdir"));
        infra.add_file(
            PathBuf::from("/test/mixdir/dir_file.txt"),
            "File in directory".to_string(),
        );

        // Add standalone file
        infra.add_file(
            PathBuf::from("/test/standalone.txt"),
            "Standalone file".to_string(),
        );

        let chat_request = ForgeChatRequest::new(infra.clone());

        // Test with both file and directory
        let url = "@[/test/mixdir] @[/test/standalone.txt]";
        let attachments = chat_request.attachments(url).await.unwrap();

        // Should include both the directory listing and the standalone file
        assert_eq!(attachments.len(), 2);

        // Find directory listing (absolute path at root level)
        let dir_attachment = attachments
            .iter()
            .find(|a| a.path == "/test/mixdir")
            .unwrap();
        match &dir_attachment.content {
            AttachmentContent::DirectoryListing { entries } => {
                assert_eq!(entries.len(), 1);
                // File path should be relative to the directory being listed
                let dir_file = entries.iter().find(|e| e.path == "dir_file.txt").unwrap();
                assert!(!dir_file.is_dir);
            }
            _ => panic!("Expected DirectoryListing attachment"),
        }

        // Find file attachment (absolute path at root level)
        let file_attachment = attachments
            .iter()
            .find(|a| a.path == "/test/standalone.txt")
            .unwrap();
        assert!(matches!(
            &file_attachment.content,
            AttachmentContent::FileContent { .. }
        ));
    }

    #[tokio::test]
    async fn test_directory_sorting_dirs_first() {
        let infra = Arc::new(MockCompositeService::new());

        // Add directory with mixed files and subdirectories in random order
        infra.file_service.add_dir(PathBuf::from("/test/sortdir"));
        infra.add_file(
            PathBuf::from("/test/sortdir/zebra.txt"),
            "File Z".to_string(),
        );
        infra
            .file_service
            .add_dir(PathBuf::from("/test/sortdir/apple_dir"));
        infra.add_file(
            PathBuf::from("/test/sortdir/banana.txt"),
            "File B".to_string(),
        );
        infra
            .file_service
            .add_dir(PathBuf::from("/test/sortdir/zoo_dir"));
        infra.add_file(
            PathBuf::from("/test/sortdir/cherry.txt"),
            "File C".to_string(),
        );
        infra
            .file_service
            .add_dir(PathBuf::from("/test/sortdir/berry_dir"));

        let chat_request = ForgeChatRequest::new(infra.clone());
        let url = "@[/test/sortdir]";
        let attachments = chat_request.attachments(url).await.unwrap();

        // Verify directory listing
        assert_eq!(attachments.len(), 1);
        let attachment = attachments.first().unwrap();

        match &attachment.content {
            AttachmentContent::DirectoryListing { entries } => {
                assert_eq!(entries.len(), 6);

                // Verify directories come first, sorted alphabetically
                assert!(entries[0].is_dir);
                assert_eq!(entries[0].path, "apple_dir");

                assert!(entries[1].is_dir);
                assert_eq!(entries[1].path, "berry_dir");

                assert!(entries[2].is_dir);
                assert_eq!(entries[2].path, "zoo_dir");

                // Verify files come after, sorted alphabetically
                assert!(!entries[3].is_dir);
                assert_eq!(entries[3].path, "banana.txt");

                assert!(!entries[4].is_dir);
                assert_eq!(entries[4].path, "cherry.txt");

                assert!(!entries[5].is_dir);
                assert_eq!(entries[5].path, "zebra.txt");
            }
            _ => panic!("Expected DirectoryListing attachment"),
        }
    }

    #[tokio::test]
    async fn test_directory_sorting_only_directories() {
        let infra = Arc::new(MockCompositeService::new());

        // Add directory with only subdirectories
        infra.file_service.add_dir(PathBuf::from("/test/onlydirs"));
        infra
            .file_service
            .add_dir(PathBuf::from("/test/onlydirs/zebra_dir"));
        infra
            .file_service
            .add_dir(PathBuf::from("/test/onlydirs/alpha_dir"));
        infra
            .file_service
            .add_dir(PathBuf::from("/test/onlydirs/middle_dir"));

        let chat_request = ForgeChatRequest::new(infra.clone());
        let url = "@[/test/onlydirs]";
        let attachments = chat_request.attachments(url).await.unwrap();

        match &attachments[0].content {
            AttachmentContent::DirectoryListing { entries } => {
                assert_eq!(entries.len(), 3);

                // All should be directories, sorted alphabetically
                assert!(entries[0].is_dir);
                assert_eq!(entries[0].path, "alpha_dir");

                assert!(entries[1].is_dir);
                assert_eq!(entries[1].path, "middle_dir");

                assert!(entries[2].is_dir);
                assert_eq!(entries[2].path, "zebra_dir");
            }
            _ => panic!("Expected DirectoryListing attachment"),
        }
    }

    #[tokio::test]
    async fn test_directory_sorting_only_files() {
        let infra = Arc::new(MockCompositeService::new());

        // Add directory with only files
        infra.file_service.add_dir(PathBuf::from("/test/onlyfiles"));
        infra.add_file(PathBuf::from("/test/onlyfiles/zebra.txt"), "Z".to_string());
        infra.add_file(PathBuf::from("/test/onlyfiles/alpha.txt"), "A".to_string());
        infra.add_file(PathBuf::from("/test/onlyfiles/middle.txt"), "M".to_string());

        let chat_request = ForgeChatRequest::new(infra.clone());
        let url = "@[/test/onlyfiles]";
        let attachments = chat_request.attachments(url).await.unwrap();

        match &attachments[0].content {
            AttachmentContent::DirectoryListing { entries } => {
                assert_eq!(entries.len(), 3);

                // All should be files, sorted alphabetically
                assert!(!entries[0].is_dir);
                assert_eq!(entries[0].path, "alpha.txt");

                assert!(!entries[1].is_dir);
                assert_eq!(entries[1].path, "middle.txt");

                assert!(!entries[2].is_dir);
                assert_eq!(entries[2].path, "zebra.txt");
            }
            _ => panic!("Expected DirectoryListing attachment"),
        }
    }

    #[tokio::test]
    async fn test_directory_sorting_case_insensitive() {
        let infra = Arc::new(MockCompositeService::new());

        // Add directory with mixed case names
        infra.file_service.add_dir(PathBuf::from("/test/casetest"));
        infra
            .file_service
            .add_dir(PathBuf::from("/test/casetest/Zebra_dir"));
        infra
            .file_service
            .add_dir(PathBuf::from("/test/casetest/apple_dir"));
        infra.add_file(PathBuf::from("/test/casetest/Zebra.txt"), "Z".to_string());
        infra.add_file(PathBuf::from("/test/casetest/apple.txt"), "A".to_string());

        let chat_request = ForgeChatRequest::new(infra.clone());
        let url = "@[/test/casetest]";
        let attachments = chat_request.attachments(url).await.unwrap();

        match &attachments[0].content {
            AttachmentContent::DirectoryListing { entries } => {
                assert_eq!(entries.len(), 4);

                // Directories first
                assert!(entries[0].is_dir);
                assert!(entries[1].is_dir);

                // Files after
                assert!(!entries[2].is_dir);
                assert!(!entries[3].is_dir);

                // Note: Rust's default string comparison is case-sensitive
                // so "Zebra_dir" < "apple_dir" (uppercase comes before
                // lowercase) This documents the current
                // behavior
            }
            _ => panic!("Expected DirectoryListing attachment"),
        }
    }
}
