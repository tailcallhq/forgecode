use std::path::Path;

use anyhow::bail;

/// Ensures that the given path is absolute
///
/// # Arguments
/// * `path` - The path to validate
///
/// # Returns
/// * `Ok(())` if the path is absolute
/// * `Err(String)` with an error message if the path is relative
pub fn assert_absolute_path(path: &Path) -> anyhow::Result<()> {
    if !path.is_absolute() {
        bail!("Path must be absolute. Please provide an absolute path starting with '/' (Unix) or 'C:\\' (Windows)".to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn test_unix_absolute_path() {
        let path = Path::new("/absolute/path");
        assert!(assert_absolute_path(path).is_ok());
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_windows_absolute_path() {
        let path = Path::new("C:\\Windows\\Path");
        assert!(assert_absolute_path(path).is_ok());
    }

    #[test]
    fn test_basic_relative_path() {
        let path = Path::new("relative/path");
        assert!(assert_absolute_path(path).is_err());
    }

    #[test]
    fn test_current_dir_relative_path() {
        let path = Path::new("./current/path");
        assert!(assert_absolute_path(path).is_err());
    }

    #[test]
    fn test_parent_dir_relative_path() {
        let path = Path::new("../parent/path");
        assert!(assert_absolute_path(path).is_err());
    }
}
