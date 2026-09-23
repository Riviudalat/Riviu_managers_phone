//! Fail-closed contract for publishing through the Threads mobile app.

use serde::{Deserialize, Serialize};

pub const ANDROID_PACKAGE: &str = "com.instagram.barcelona";
pub const MAX_CAPTION_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadsPublishStep {
    ResolveApp,
    OpenComposer,
    SelectMedia,
    EnterCaption,
    ArmPublish,
    RecordIntent,
    DispatchOnce,
    VerifyOwnPost,
    CapturePermalink,
}

pub const THREADS_PUBLISH_FLOW: &[ThreadsPublishStep] = &[
    ThreadsPublishStep::ResolveApp,
    ThreadsPublishStep::OpenComposer,
    ThreadsPublishStep::SelectMedia,
    ThreadsPublishStep::EnterCaption,
    ThreadsPublishStep::ArmPublish,
    ThreadsPublishStep::RecordIntent,
    ThreadsPublishStep::DispatchOnce,
    ThreadsPublishStep::VerifyOwnPost,
    ThreadsPublishStep::CapturePermalink,
];

pub fn validate_caption(caption: &str) -> anyhow::Result<()> {
    let length = caption.chars().count();
    anyhow::ensure!(!caption.trim().is_empty(), "nội dung Threads đang trống");
    anyhow::ensure!(
        length <= MAX_CAPTION_CHARS,
        "nội dung Threads có {length} ký tự; tối đa {MAX_CAPTION_CHARS}"
    );
    Ok(())
}

/// No tuple is admitted until its accessibility hierarchy and post-link verifier
/// have been captured as fixtures. Package detection alone is not automation support.
pub fn measured_mobile_build(_package: &str, _version: &str, _locale: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_records_intent_before_the_only_public_effect() {
        let intent = THREADS_PUBLISH_FLOW
            .iter()
            .position(|step| *step == ThreadsPublishStep::RecordIntent)
            .unwrap();
        let dispatch = THREADS_PUBLISH_FLOW
            .iter()
            .position(|step| *step == ThreadsPublishStep::DispatchOnce)
            .unwrap();
        assert!(intent < dispatch);
        assert_eq!(
            THREADS_PUBLISH_FLOW
                .iter()
                .filter(|step| **step == ThreadsPublishStep::DispatchOnce)
                .count(),
            1
        );
    }

    #[test]
    fn caption_uses_the_threads_character_limit() {
        assert!(validate_caption(&"x".repeat(500)).is_ok());
        assert!(validate_caption(&"x".repeat(501)).is_err());
        assert!(validate_caption("  ").is_err());
    }
}
