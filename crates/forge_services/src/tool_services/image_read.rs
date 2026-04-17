use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use forge_app::domain::Image;
use forge_app::{EnvironmentInfra, FileInfoInfra, ImageReadService};
use strum_macros::{Display, EnumString};

use crate::utils::assert_absolute_path;

pub struct ForgeImageRead<F> {
    infra: Arc<F>,
}

/// Supported image formats for binary file reading
#[derive(Debug, Clone, Copy, EnumString, Display)]
#[strum(serialize_all = "lowercase")]
enum ImageFormat {
    #[strum(serialize = "jpg", serialize = "jpeg")]
    Jpeg,
    Png,
    Webp,
    Gif,
}

impl ImageFormat {
    /// Returns the MIME type for this image format
    fn mime_type(&self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
            Self::Gif => "image/gif",
        }
    }

    /// Returns a comma-separated list of supported formats
    fn supported_formats() -> &'static str {
        "JPEG, PNG, WebP, GIF"
    }
}

impl<F> ForgeImageRead<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<
    F: FileInfoInfra
        + EnvironmentInfra<Config = forge_config::ForgeConfig>
        + forge_app::FileReaderInfra,
> ImageReadService for ForgeImageRead<F>
{
    async fn read_image(&self, path: String) -> anyhow::Result<Image> {
        let path = Path::new(&path);
        assert_absolute_path(path)?;

        let max_image_size_bytes = self.infra.get_config()?.max_image_size_bytes;

        // Validate file size before reading content using image-specific file size
        // limit
        crate::tool_services::fs_read::assert_file_size(&*self.infra, path, max_image_size_bytes)
            .await
            .with_context(
                || "Image exceeds size limit. Compress the image or increase FORGE_MAX_IMAGE_SIZE.",
            )?;

        // Determine image format from file extension
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .ok_or_else(|| {
                anyhow::anyhow!("File has no extension. Cannot determine image format.")
            })?;

        let format = extension.parse::<ImageFormat>().map_err(|_| {
            anyhow::anyhow!(
                "Unsupported image format: {}. Supported formats: {}",
                extension,
                ImageFormat::supported_formats()
            )
        })?;

        // Read the binary content
        let content = self
            .infra
            .read(path)
            .await
            .with_context(|| format!("Failed to read binary file from {}", path.display()))?;

        let image = Image::new_bytes(content, format.mime_type());

        Ok(image)
    }
}
