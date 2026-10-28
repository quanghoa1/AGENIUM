//! `view_image`: an image of the worktree, shown to a model that can see.

use super::*;

/// Largest image `view_image` sends: the providers take about 5 MB of base64 per image.
pub const MAX_IMAGE_BYTES: u64 = 3_750_000;

/// Look at an image file of your worktree.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewImageArgs {
    /// Path relative to the worktree root (PNG, JPEG, GIF or WebP).
    pub path: String,
}

pub struct ViewImage;

/// The image type of `bytes` by their signature (an extension can lie).
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

#[async_trait]
impl Tool for ViewImage {
    fn name(&self) -> &'static str {
        "view_image"
    }
    fn description(&self) -> &'static str {
        "Look at an image file in your worktree (PNG, JPEG, GIF or WebP, up to 3.75 MB): a screenshot you took, a design, a chart. The image is shown to you after the tool results."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<ViewImageArgs>()
    }
    fn read_only(&self) -> bool {
        true
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        use base64::Engine;
        let a: ViewImageArgs = args(a)?;
        let p = ctx.resolve(&a.path, false)?;
        let rel = display(&ctx.root, &p);
        let meta = std::fs::metadata(&p).map_err(|e| io_err(&rel, e))?;
        if meta.is_dir() {
            return Err(ToolError::new(
                "is_directory",
                format!("{rel} is a directory"),
            ));
        }
        if meta.len() > MAX_IMAGE_BYTES {
            return Err(ToolError::new(
                "too_large",
                format!(
                    "{rel} is {} KB, more than {} KB: save a smaller image (a lower resolution, or JPEG)",
                    meta.len() >> 10,
                    MAX_IMAGE_BYTES >> 10
                ),
            ));
        }
        let bytes = std::fs::read(&p).map_err(|e| io_err(&rel, e))?;
        let media_type = image_type(&bytes).ok_or_else(|| {
            ToolError::new(
                "not_an_image",
                format!("{rel} is not a PNG, JPEG, GIF or WebP image"),
            )
        })?;
        let mut out = ToolOutput::text(format!(
            "{rel}: {media_type}, {} KB; the image follows the tool results",
            (bytes.len() as u64).div_ceil(1024)
        ));
        out.images.push(crate::ImageData {
            media_type: media_type.into(),
            data_b64: base64::engine::general_purpose::STANDARD.encode(&bytes),
            label: rel,
        });
        Ok(out)
    }
}
