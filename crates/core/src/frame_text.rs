use async_trait::async_trait;

use crate::CommentOcrObservation;

/// Text evidence extracted from a stream frame. The desktop supplies the
/// platform adapter (Vision on macOS, pinned OCR on Windows); core only sees
/// this contract so the production and API-test comment paths stay identical.
#[async_trait]
pub trait FrameTextSource: Send + Sync {
    async fn recognize(&self, frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>>;
}

#[derive(Debug, Default)]
pub struct NullFrameTextSource;

#[async_trait]
impl FrameTextSource for NullFrameTextSource {
    async fn recognize(&self, _frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>> {
        anyhow::bail!("frame text source is not configured")
    }
}
