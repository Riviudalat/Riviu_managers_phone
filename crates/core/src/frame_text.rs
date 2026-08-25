use async_trait::async_trait;

use crate::CommentOcrObservation;

/// Text evidence extracted from a stream frame. The desktop supplies the
/// platform adapter (Vision on macOS, pinned OCR on Windows); core only sees
/// this contract so the production and API-test comment paths stay identical.
#[async_trait]
pub trait FrameTextSource: Send + Sync {
    async fn recognize(&self, frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>>;

    /// Which recogniser produced these observations, recorded with the evidence.
    ///
    /// Filed alongside a comment's locator identity, so a stored identity can be read back
    /// knowing whether Vision or Windows Media OCR drew the boxes -- the two do not agree on
    /// character-level bounds, and an identity is only comparable against its own version.
    /// It belongs on the source rather than on a free function because it *is* the source's
    /// identity; a free function is what tied the interaction campaign to a desktop module.
    fn locator_version(&self) -> &'static str {
        "none"
    }
}

#[derive(Debug, Default)]
pub struct NullFrameTextSource;

#[async_trait]
impl FrameTextSource for NullFrameTextSource {
    async fn recognize(&self, _frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>> {
        anyhow::bail!("frame text source is not configured")
    }
}
