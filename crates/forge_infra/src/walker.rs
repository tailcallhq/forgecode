use anyhow::Result;
use forge_app::{WalkedFile, Walker};

pub struct ForgeWalkerService;

impl ForgeWalkerService {
    pub fn new() -> Self {
        Self
    }

    pub async fn walk(&self, config: Walker) -> Result<Vec<WalkedFile>> {
        // Convert domain config to forge_walker config
        let mut walker = if config.max_depth.is_none()
            && config.max_breadth.is_none()
            && config.max_file_size.is_none()
            && config.max_files.is_none()
            && config.max_total_size.is_none()
        {
            // Agent-facing walker: keep hidden files excluded by default.
            forge_walker::Walker::max_all().hidden(true)
        } else {
            forge_walker::Walker::min_all()
        };

        walker = walker.cwd(config.cwd);

        if let Some(depth) = config.max_depth {
            walker = walker.max_depth(depth);
        }
        if let Some(breadth) = config.max_breadth {
            walker = walker.max_breadth(breadth);
        }
        if let Some(file_size) = config.max_file_size {
            walker = walker.max_file_size(file_size);
        }
        if let Some(files) = config.max_files {
            walker = walker.max_files(files);
        }
        if let Some(total_size) = config.max_total_size {
            walker = walker.max_total_size(total_size);
        }
        walker = walker.skip_binary(config.skip_binary);

        // Execute the walker and convert results
        let files = walker.get().await?;
        let walked_files = files
            .into_iter()
            .map(|f| WalkedFile { path: f.path, file_name: f.file_name, size: f.size })
            .collect();

        Ok(walked_files)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn test_walker_service_basic_functionality() {
        let fixture = tempdir().unwrap();
        std::fs::write(fixture.path().join("test.txt"), "test content").unwrap();

        let service = ForgeWalkerService::new();
        let config = Walker::conservative().cwd(fixture.path().to_path_buf());

        let actual = service.walk(config).await.unwrap();

        let expected = 1; // Should find the test file
        let file_count = actual.iter().filter(|f| !f.is_dir()).count();
        assert_eq!(file_count, expected);
    }

    #[tokio::test]
    async fn test_walker_service_unlimited_config() {
        let fixture = tempdir().unwrap();
        std::fs::write(fixture.path().join("test.txt"), "test content").unwrap();

        let service = ForgeWalkerService::new();
        let config = Walker::unlimited().cwd(fixture.path().to_path_buf());

        let actual = service.walk(config).await.unwrap();

        let expected = 1; // Should find the test file
        let file_count = actual.iter().filter(|f| !f.is_dir()).count();
        assert_eq!(file_count, expected);
    }
}
