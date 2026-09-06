//! Bounded chronological evidence. Elapsed capture time is not semantic coverage.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::driver::UiSession;
use crate::interaction_hierarchy::{read_post_caption, SlideCamera};
use crate::tiktok_labels::{controls_for, TikTokControl};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCardIdentity {
    pub author: String,
    pub caption: String,
}

impl VideoCardIdentity {
    pub fn matches(&self, other: &Self) -> bool {
        !self.author.trim().is_empty() && !self.caption.trim().is_empty() && self == other
    }
}

pub async fn read_video_identity(
    session: &dyn UiSession,
    package: &str,
) -> anyhow::Result<VideoCardIdentity> {
    let version = session
        .app_version(package)
        .await
        .context("video version unavailable")?;
    let language = session
        .ui_language()
        .await
        .context("video locale unavailable")?;
    let labels =
        controls_for(package, &language, &version).context("video_identity: unmeasured build")?;
    let author_label = labels
        .label(TikTokControl::AuthorProfileLink)
        .context("video_identity: no author locator")?;
    let author = session
        .locate(author_label.to_query())
        .await?
        .and_then(|element| element.description)
        .filter(|value| !value.trim().is_empty())
        .context("video_identity: author unreadable")?;
    let caption = read_post_caption(session)
        .await
        .filter(|value| !value.trim().is_empty())
        .context("video_identity: caption unreadable")?;
    Ok(VideoCardIdentity { author, caption })
}

#[derive(Clone)]
pub struct VideoSample {
    pub observed_ms: u64,
    pub frame: Vec<u8>,
}

pub struct VideoEvidence {
    pub identity: VideoCardIdentity,
    pub samples: Vec<VideoSample>,
    pub requested_ms: u64,
}

impl VideoEvidence {
    pub fn span_ms(&self) -> u64 {
        self.samples
            .last()
            .zip(self.samples.first())
            .map(|(last, first)| last.observed_ms.saturating_sub(first.observed_ms))
            .unwrap_or(0)
    }
    pub fn frames(&self) -> Vec<Vec<u8>> {
        self.samples
            .iter()
            .map(|sample| sample.frame.clone())
            .collect()
    }
}

pub fn sample_offsets(window: Duration, count: usize) -> anyhow::Result<Vec<Duration>> {
    anyhow::ensure!(
        (2..=12).contains(&count),
        "video evidence requires 2..12 samples"
    );
    anyhow::ensure!(
        (Duration::from_secs(1)..=Duration::from_secs(60)).contains(&window),
        "video evidence window must be 1..60 seconds"
    );
    Ok((0..count)
        .map(|index| window.mul_f64(index as f64 / (count - 1) as f64))
        .collect())
}

/// Samples are accepted only when identity matches both sides of capture. A caller
/// never receives a partial mixed-card set, including a changed final frame.
pub async fn collect_video_evidence(
    session: &dyn UiSession,
    camera: &dyn SlideCamera,
    package: &str,
    window: Duration,
    count: usize,
    stop: &AtomicBool,
) -> anyhow::Result<VideoEvidence> {
    sample_offsets(window, count)?;
    anyhow::ensure!(!stop.load(Ordering::Relaxed), "video evidence stopped");
    tokio::time::timeout(
        window.saturating_add(Duration::from_secs(30)),
        collect_video_evidence_inner(session, camera, package, window, count, stop),
    )
    .await
    .context("video evidence deadline exceeded")?
}

async fn collect_video_evidence_inner(
    session: &dyn UiSession,
    camera: &dyn SlideCamera,
    package: &str,
    window: Duration,
    count: usize,
    stop: &AtomicBool,
) -> anyhow::Result<VideoEvidence> {
    let offsets = sample_offsets(window, count)?;
    let identity = read_video_identity(session, package).await?;
    let started = tokio::time::Instant::now();
    let mut samples = Vec::with_capacity(count);
    for offset in offsets {
        while started.elapsed() < offset {
            anyhow::ensure!(!stop.load(Ordering::Relaxed), "video evidence stopped");
            tokio::time::sleep(
                offset
                    .saturating_sub(started.elapsed())
                    .min(Duration::from_millis(100)),
            )
            .await;
        }
        anyhow::ensure!(!stop.load(Ordering::Relaxed), "video evidence stopped");
        let before = read_video_identity(session, package).await?;
        anyhow::ensure!(identity.matches(&before), "video evidence card_changed");
        let frame = camera
            .capture()
            .await
            .context("video evidence frame unavailable")?;
        let observed_ms = started.elapsed().as_millis() as u64;
        let after = read_video_identity(session, package).await?;
        anyhow::ensure!(identity.matches(&after), "video evidence card_changed");
        anyhow::ensure!(!stop.load(Ordering::Relaxed), "video evidence stopped");
        samples.push(VideoSample { observed_ms, frame });
    }
    Ok(VideoEvidence {
        identity,
        samples,
        requested_ms: window.as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{ElementBox, ElementQuery};
    use std::sync::atomic::AtomicUsize;

    struct Session {
        reads: AtomicUsize,
        change_at: usize,
    }
    #[async_trait::async_trait]
    impl UiSession for Session {
        async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
            panic!("read only")
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            panic!("read only")
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("read only")
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("read only")
        }
        async fn back(&self) -> anyhow::Result<()> {
            panic!("read only")
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("read only")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            panic!("read only")
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        async fn app_version(&self, _: &str) -> Option<String> {
            Some("38.3.2".into())
        }
        async fn ui_language(&self) -> Option<String> {
            Some("en".into())
        }
        async fn locate(&self, _: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let n = self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(Some(ElementBox {
                x: 1.0,
                y: 1.0,
                width: 20.0,
                height: 20.0,
                description: Some(
                    if n >= self.change_at {
                        "B profile"
                    } else {
                        "A profile"
                    }
                    .into(),
                ),
                enabled: true,
                clickable: true,
            }))
        }
        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            Ok(if matches!(query, ElementQuery::ResourceIdSuffix(_)) {
                vec![ElementBox {
                    x: 1.0,
                    y: 1.0,
                    width: 20.0,
                    height: 20.0,
                    description: Some(
                        "A stable source caption long enough to identify the video".into(),
                    ),
                    enabled: true,
                    clickable: false,
                }]
            } else {
                vec![]
            })
        }
    }
    struct Camera(AtomicUsize);
    #[async_trait::async_trait]
    impl SlideCamera for Camera {
        async fn capture(&self) -> Option<Vec<u8>> {
            Some(vec![self.0.fetch_add(1, Ordering::Relaxed) as u8])
        }
    }

    #[tokio::test(start_paused = true)]
    async fn collects_in_order_and_reproves_every_sample() {
        let session = Session {
            reads: AtomicUsize::new(0),
            change_at: usize::MAX,
        };
        let camera = Camera(AtomicUsize::new(0));
        let result = collect_video_evidence(
            &session,
            &camera,
            "com.ss.android.ugc.trill",
            Duration::from_secs(12),
            4,
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
        assert_eq!(result.samples.len(), 4);
        assert_eq!(result.span_ms(), 12000);
        assert_eq!(session.reads.load(Ordering::Relaxed), 9);
    }

    #[tokio::test(start_paused = true)]
    async fn rejects_card_change_during_capture_without_returning_mixed_frames() {
        let session = Session {
            reads: AtomicUsize::new(0),
            change_at: 2,
        };
        let camera = Camera(AtomicUsize::new(0));
        let result = collect_video_evidence(
            &session,
            &camera,
            "com.ss.android.ugc.trill",
            Duration::from_secs(12),
            4,
            &AtomicBool::new(false),
        )
        .await;
        assert!(result.err().unwrap().to_string().contains("card_changed"));
        assert_eq!(camera.0.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn stop_before_capture_reads_nothing() {
        let session = Session {
            reads: AtomicUsize::new(0),
            change_at: usize::MAX,
        };
        let camera = Camera(AtomicUsize::new(0));
        assert!(collect_video_evidence(
            &session,
            &camera,
            "com.ss.android.ugc.trill",
            Duration::from_secs(12),
            4,
            &AtomicBool::new(true)
        )
        .await
        .is_err());
        assert_eq!(session.reads.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn schedule_spans_the_window_and_preserves_endpoints() {
        assert_eq!(
            sample_offsets(Duration::from_secs(12), 4).unwrap(),
            [0, 4, 8, 12].map(Duration::from_secs)
        );
        let full = sample_offsets(Duration::from_secs(48), 12).unwrap();
        assert_eq!(full.first(), Some(&Duration::ZERO));
        assert_eq!(full.last(), Some(&Duration::from_secs(48)));
    }

    #[test]
    fn unknown_or_changed_identity_never_matches() {
        let a = VideoCardIdentity {
            author: "a".into(),
            caption: "caption".into(),
        };
        assert!(a.matches(&a));
        assert!(!a.matches(&VideoCardIdentity {
            author: "b".into(),
            ..a.clone()
        }));
        assert!(!a.matches(&VideoCardIdentity {
            caption: "other".into(),
            ..a.clone()
        }));
        let empty = VideoCardIdentity {
            author: "a".into(),
            caption: String::new(),
        };
        assert!(!empty.matches(&empty));
    }

    #[test]
    fn malformed_or_unbounded_sampling_is_rejected() {
        for (seconds, count) in [(0, 4), (61, 4), (12, 1), (12, 13)] {
            assert!(sample_offsets(Duration::from_secs(seconds), count).is_err());
        }
    }
}
