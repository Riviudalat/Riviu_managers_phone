use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Context;
use tokio::time::Instant;

use super::{
    confirm_target_from_share_link, read_author_label, read_target_identity_caption, TargetArrival,
    TargetLinkMismatch,
};
use crate::driver::{ElementQuery, UiSession};
use crate::tiktok_labels::{TikTokControl, TikTokControls};
use crate::ResolvedTikTokTarget;

const CARD_WINDOW: Duration = Duration::from_secs(14);
const CARD_POLL: Duration = Duration::from_millis(350);
const DISPATCH_SETTLE: Duration = Duration::from_millis(900);
const UNAVAILABLE_TEXT: &str = "This video is unavailable";

#[derive(Debug, thiserror::Error)]
enum CardObservationError {
    #[error("target_post_unavailable: TikTok reported 'This video is unavailable'")]
    PostUnavailable,
    #[error("foreground: package unreadable")]
    ForegroundUnreadable,
    #[error("foreground: expected target app, observed {0}")]
    WrongForeground(String),
    #[error("Comments: measured control missing or unreadable")]
    CommentsUnreadable,
    #[error("author: label missing or unreadable")]
    AuthorUnreadable,
    #[error("caption: measured identity missing, empty, ambiguous or unreadable")]
    CaptionUnreadable,
}

/// Prove the current post by its own copied URL, or open the pinned URL once and prove it.
/// Author/caption are continuity evidence, never authority for a particular post. Navigation
/// uses only Share/Copy link and the pinned URL; it never taps a public-action control.
pub async fn open_exact_target_by_hierarchy(
    session: &dyn UiSession,
    labels: TikTokControls,
    target_package: &str,
    expected: &ResolvedTikTokTarget,
    stop: &AtomicBool,
) -> anyhow::Result<TargetArrival> {
    ensure_not_cancelled(stop)?;
    anyhow::ensure!(
        labels.label(TikTokControl::Comments).is_some(),
        "target_exact_open: comments control unmeasured"
    );

    if readable_card_in_foreground(session, labels, target_package, false)
        .await
        .is_ok()
    {
        ensure_not_cancelled(stop)?;
        if let Ok(arrival) = confirm_target_from_share_link(session, labels, expected).await {
            ensure_not_cancelled(stop)?;
            ensure_foreground(session, target_package).await?;
            return Ok(arrival);
        }
    }

    ensure_not_cancelled(stop)?;
    session
        .open_url_in_app(&expected.normalized_url, target_package)
        .await
        .context("target_exact_open: pinned URL dispatch failed")?;

    // Do not require a Home/feed baseline: cold launches and an already-open target can
    // both lack a changed author. The URL readback below is the only arrival authority.
    let deadline = Instant::now() + CARD_WINDOW;
    tokio::time::sleep(DISPATCH_SETTLE).await;
    loop {
        ensure_not_cancelled(stop)?;
        let observation = readable_card_in_foreground(session, labels, target_package, true).await;
        if matches!(observation, Err(CardObservationError::PostUnavailable)) {
            return Err(CardObservationError::PostUnavailable.into());
        }
        if observation.is_ok() {
            ensure_not_cancelled(stop)?;
            let proof = confirm_target_from_share_link(session, labels, expected).await;
            ensure_not_cancelled(stop)?;
            ensure_foreground(session, target_package).await?;
            match proof {
                Ok(arrival) => return Ok(arrival),
                Err(error)
                    if error.downcast_ref::<TargetLinkMismatch>().is_some()
                        && Instant::now() < deadline => {}
                Err(error) => {
                    return Err(error.context("target_exact_open: exact post proof failed"));
                }
            }
        }
        if Instant::now() >= deadline {
            let last_check = observation
                .err()
                .map(|error| error.to_string())
                .unwrap_or_else(|| "copied URL did not identify the target".into());
            anyhow::bail!(
                "target_exact_open: no readable post in the target app within 14 seconds; last check: {last_check}"
            );
        }
        tokio::time::sleep(CARD_POLL.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
}

fn ensure_not_cancelled(stop: &AtomicBool) -> anyhow::Result<()> {
    anyhow::ensure!(
        !stop.load(Ordering::Relaxed),
        "target_exact_open: cancelled"
    );
    Ok(())
}

async fn ensure_foreground(session: &dyn UiSession, target_package: &str) -> anyhow::Result<()> {
    let foreground = session
        .active_app_bundle()
        .await
        .context("target_exact_open: foreground unreadable after proof")?;
    anyhow::ensure!(
        foreground == target_package,
        "target_exact_open: target app left foreground during proof"
    );
    Ok(())
}

async fn readable_card_in_foreground(
    session: &dyn UiSession,
    labels: TikTokControls,
    target_package: &str,
    after_dispatch: bool,
) -> Result<(), CardObservationError> {
    match session.active_app_bundle().await {
        Ok(package) if package == target_package => {}
        Ok(package) => {
            return Err(CardObservationError::WrongForeground(
                package.chars().take(128).collect(),
            ));
        }
        Err(_) => return Err(CardObservationError::ForegroundUnreadable),
    }
    if after_dispatch
        && matches!(
            session
                .locate(ElementQuery::Text {
                    value: UNAVAILABLE_TEXT,
                    exact: true,
                })
                .await,
            Ok(Some(_))
        )
    {
        return Err(CardObservationError::PostUnavailable);
    }
    let Some(comments) = labels.label(TikTokControl::Comments) else {
        return Err(CardObservationError::CommentsUnreadable);
    };
    if !matches!(session.locate(comments.to_query()).await, Ok(Some(_))) {
        return Err(CardObservationError::CommentsUnreadable);
    }
    if read_author_label(session, labels)
        .await
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err(CardObservationError::AuthorUnreadable);
    }
    if read_target_identity_caption(session)
        .await
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err(CardObservationError::CaptionUnreadable);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use parking_lot::Mutex;

    use super::*;
    use crate::driver::{ElementBox, ElementQuery};
    use crate::tiktok_labels::controls_for;
    use crate::{SwipeGesture, TapPoint};

    const PACKAGE: &str = "com.ss.android.ugc.trill";
    const TARGET_URL: &str = "https://www.tiktok.com/@fixture/photo/12345";
    const WRONG_URL: &str = "https://www.tiktok.com/@fixture/photo/54321";
    const CAPTION: &str = "A stable caption long enough to identify this one post without guessing";

    struct ExactSession {
        card_visible: AtomicBool,
        clipboard: Mutex<String>,
        sheet_open: AtomicBool,
        opens: Mutex<Vec<(String, String)>>,
        dispatched_at: Mutex<Option<Instant>>,
        tapped_at: Mutex<Vec<Instant>>,
        navigation_taps: AtomicUsize,
        foreground: bool,
        copied_url: &'static str,
        card_changes_after_copy: bool,
        copies: AtomicUsize,
        caption_nodes: Vec<&'static str>,
        caption_query_fails: bool,
        fallback_caption: Option<&'static str>,
        target_load_delay: Option<Duration>,
        copied_urls: Mutex<Vec<&'static str>>,
        clipboard_write_fails: bool,
        clipboard_writes: AtomicUsize,
        unavailable_text: Option<&'static str>,
        comments_missing: bool,
        author_missing: bool,
    }

    impl ExactSession {
        fn new(card_visible: bool, copied_url: &'static str) -> Self {
            Self {
                card_visible: AtomicBool::new(card_visible),
                clipboard: Mutex::new(String::new()),
                sheet_open: AtomicBool::new(false),
                opens: Mutex::new(Vec::new()),
                dispatched_at: Mutex::new(None),
                tapped_at: Mutex::new(Vec::new()),
                navigation_taps: AtomicUsize::new(0),
                foreground: true,
                copied_url,
                card_changes_after_copy: false,
                copies: AtomicUsize::new(0),
                caption_nodes: vec![CAPTION],
                caption_query_fails: false,
                fallback_caption: None,
                target_load_delay: None,
                copied_urls: Mutex::new(Vec::new()),
                clipboard_write_fails: false,
                clipboard_writes: AtomicUsize::new(0),
                unavailable_text: None,
                comments_missing: false,
                author_missing: false,
            }
        }

        fn is_visible(&self) -> bool {
            self.card_visible.load(Ordering::Relaxed)
        }
    }

    fn labels() -> TikTokControls {
        controls_for(PACKAGE, "en", "38.3.2").unwrap()
    }

    fn node(description: &str, y: f64) -> ElementBox {
        ElementBox {
            x: 0.0,
            y,
            width: 100.0,
            height: 100.0,
            description: Some(description.into()),
            enabled: true,
            clickable: true,
        }
    }

    #[async_trait::async_trait]
    impl UiSession for ExactSession {
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(if self.foreground {
                PACKAGE.into()
            } else {
                "com.android.launcher".into()
            })
        }

        async fn open_url_in_app(&self, url: &str, bundle: &str) -> anyhow::Result<()> {
            self.opens.lock().push((url.into(), bundle.into()));
            *self.dispatched_at.lock() = Some(Instant::now());
            self.card_visible.store(true, Ordering::Relaxed);
            Ok(())
        }

        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            if let Some(dispatched_at) = *self.dispatched_at.lock() {
                assert!(
                    dispatched_at.elapsed() >= DISPATCH_SETTLE,
                    "the URL intent has not had time to replace the previous card"
                );
            }
            self.tapped_at.lock().push(Instant::now());
            // The fixture authorizes only these two navigation rectangles. Any public
            // tap, Home tap or guessed coordinate fails the test rather than succeeding.
            if point.x == 50.0 && point.y == 550.0 && !self.sheet_open.load(Ordering::Relaxed) {
                self.sheet_open.store(true, Ordering::Relaxed);
            } else if point.x == 50.0
                && point.y == 1050.0
                && self.sheet_open.load(Ordering::Relaxed)
            {
                let copied_url = match (self.target_load_delay, *self.dispatched_at.lock()) {
                    (Some(delay), Some(dispatched_at)) if dispatched_at.elapsed() < delay => {
                        WRONG_URL
                    }
                    _ => self.copied_url,
                };
                *self.clipboard.lock() = copied_url.into();
                self.copied_urls.lock().push(copied_url);
                self.copies.fetch_add(1, Ordering::Relaxed);
                self.sheet_open.store(false, Ordering::Relaxed);
            } else {
                panic!("unexpected/public tap: {point:?}");
            }
            self.navigation_taps.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn swipe(&self, _: SwipeGesture) -> anyhow::Result<()> {
            panic!("exact target proof must not swipe")
        }

        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("exact target proof must not type")
        }

        async fn home(&self) -> anyhow::Result<()> {
            panic!("exact target proof has no Home prerequisite")
        }

        async fn back(&self) -> anyhow::Result<()> {
            panic!("copy auto-dismissed; Back would leave the post")
        }

        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("exact target proof uses measured rectangles")
        }

        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            panic!("exact target proof uses measured rectangles")
        }

        fn stream_url(&self) -> Option<String> {
            None
        }

        async fn set_clipboard(&self, _: &str, bytes: &[u8]) -> anyhow::Result<()> {
            self.clipboard_writes.fetch_add(1, Ordering::Relaxed);
            anyhow::ensure!(!self.clipboard_write_fails, "clipboard transport failed");
            *self.clipboard.lock() = String::from_utf8(bytes.to_vec())?;
            Ok(())
        }

        async fn get_clipboard(&self, _: usize) -> anyhow::Result<(String, Vec<u8>)> {
            Ok((
                "plaintext".into(),
                self.clipboard.lock().as_bytes().to_vec(),
            ))
        }

        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            if !self.is_visible() {
                return Ok(None);
            }
            if let ElementQuery::Text { value, exact: true } = query {
                return Ok(self
                    .unavailable_text
                    .filter(|text| *text == value)
                    .map(|text| node(text, 1600.0)));
            }
            let ElementQuery::Description { value, .. } = query else {
                return Ok(None);
            };
            if value == labels().label(TikTokControl::Share).unwrap().value() {
                return Ok(Some(node("Share", 500.0)));
            }
            if value == labels().label(TikTokControl::Comments).unwrap().value() {
                if self.comments_missing {
                    return Ok(None);
                }
                return Ok(Some(node(value, 300.0)));
            }
            if value
                == labels()
                    .label(TikTokControl::AuthorProfileLink)
                    .unwrap()
                    .value()
                || value == labels().label(TikTokControl::Follow).unwrap().value()
            {
                if self.author_missing {
                    return Ok(None);
                }
                let author = if self.card_changes_after_copy
                    && self.copies.load(Ordering::Relaxed) % 2 == 1
                {
                    "Different Author"
                } else {
                    "fixture"
                };
                let description = if value
                    == labels()
                        .label(TikTokControl::AuthorProfileLink)
                        .unwrap()
                        .value()
                {
                    format!("{author} profile")
                } else {
                    format!("Follow {author}")
                };
                return Ok(Some(node(&description, 100.0)));
            }
            Ok(None)
        }

        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            Ok(match query {
                ElementQuery::Description {
                    value: "Copy link",
                    exact: true,
                } if self.sheet_open.load(Ordering::Relaxed) => {
                    vec![node("Copy link", 1000.0)]
                }
                _ => vec![],
            })
        }

        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            Ok(match query {
                ElementQuery::ResourceIdSuffix(":id/desc") if self.is_visible() => {
                    anyhow::ensure!(!self.caption_query_fails, "measured caption read failed");
                    self.caption_nodes
                        .iter()
                        .map(|caption| node(caption, 1400.0))
                        .collect()
                }
                ElementQuery::ClassName("android.widget.TextView") if self.is_visible() => self
                    .fallback_caption
                    .map(|caption| vec![node(caption, 1400.0)])
                    .unwrap_or_default(),
                _ => vec![],
            })
        }
    }

    async fn run(session: &ExactSession, stop: &AtomicBool) -> anyhow::Result<TargetArrival> {
        let target = crate::parse_tiktok_links(TARGET_URL)
            .remove(0)
            .target
            .unwrap();
        open_exact_target_by_hierarchy(session, labels(), PACKAGE, &target, stop).await
    }

    #[tokio::test(start_paused = true)]
    async fn already_target_requires_exact_link_and_dispatches_no_url() {
        let session = ExactSession::new(true, TARGET_URL);
        assert!(matches!(
            run(&session, &AtomicBool::new(false)).await.unwrap(),
            TargetArrival::Identified { .. }
        ));
        assert!(session.opens.lock().is_empty());
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn cold_start_needs_no_baseline_and_opens_pinned_target_once() {
        let session = ExactSession::new(false, TARGET_URL);
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());
        assert_eq!(
            *session.opens.lock(),
            vec![(TARGET_URL.into(), PACKAGE.into())]
        );
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn pinned_dispatch_settles_before_copied_link_probe() {
        let session = ExactSession::new(false, TARGET_URL);
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());
        let dispatched_at = session.dispatched_at.lock().unwrap();
        let first_tap = session.tapped_at.lock()[0];
        assert_eq!(first_tap.duration_since(dispatched_at), DISPATCH_SETTLE);
        assert_eq!(session.copies.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn same_author_but_wrong_post_never_authorizes_public_actions() {
        let session = ExactSession::new(true, WRONG_URL);
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(format!("{error:#}").contains("different post"));
        let mismatch = error.downcast_ref::<TargetLinkMismatch>().unwrap();
        assert_eq!(mismatch.expected_url, TARGET_URL);
        assert_eq!(mismatch.observed_url, WRONG_URL);
        assert_eq!(session.opens.lock().len(), 1);
        assert!(session.navigation_taps.load(Ordering::Relaxed) >= 4);
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_before_dispatch_does_not_open_or_tap() {
        let session = ExactSession::new(true, TARGET_URL);
        let error = run(&session, &AtomicBool::new(true)).await.unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert!(session.opens.lock().is_empty());
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn no_foreground_times_out_without_public_tap_or_redispatch() {
        let mut session = ExactSession::new(false, TARGET_URL);
        session.foreground = false;
        let before = Instant::now();
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(error.to_string().contains("no readable post"));
        assert_eq!(before.elapsed(), CARD_WINDOW);
        assert_eq!(session.opens.lock().len(), 1);
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn changing_card_during_copy_remains_a_refusal() {
        let mut session = ExactSession::new(true, TARGET_URL);
        session.card_changes_after_copy = true;
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(format!("{error:#}").contains("card changed"));
        assert_eq!(session.opens.lock().len(), 1);
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn short_measured_caption_still_requires_an_exact_copied_url() {
        let mut session = ExactSession::new(true, TARGET_URL);
        session.caption_nodes = vec!["Short"];
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());
        assert!(session.opens.lock().is_empty());
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 2);
        session.copied_url = WRONG_URL;
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(format!("{error:#}").contains("different post"));
    }

    #[tokio::test(start_paused = true)]
    async fn duplicate_measured_caption_nodes_never_open_share_or_use_fallback() {
        let mut session = ExactSession::new(true, TARGET_URL);
        session.caption_nodes = vec!["Short", "Short"];
        session.fallback_caption = Some(CAPTION);
        assert!(run(&session, &AtomicBool::new(false)).await.is_err());
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
        assert_eq!(session.opens.lock().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn no_measured_caption_node_preserves_long_class_fallback() {
        let mut session = ExactSession::new(true, TARGET_URL);
        session.caption_nodes.clear();
        session.fallback_caption = Some(CAPTION);
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());
        assert!(session.opens.lock().is_empty());
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn empty_or_failed_measured_caption_read_cannot_use_class_fallback() {
        for query_fails in [false, true] {
            let mut session = ExactSession::new(true, TARGET_URL);
            session.caption_nodes = vec!["   "];
            session.caption_query_fails = query_fails;
            session.fallback_caption = Some(CAPTION);
            assert!(run(&session, &AtomicBool::new(false)).await.is_err());
            assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn short_class_text_is_not_a_measured_caption() {
        let mut session = ExactSession::new(true, TARGET_URL);
        session.caption_nodes.clear();
        session.fallback_caption = Some("Short");
        assert!(run(&session, &AtomicBool::new(false)).await.is_err());
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_old_card_waits_for_exact_url_without_redispatching() {
        let mut session = ExactSession::new(false, TARGET_URL);
        session.target_load_delay = Some(Duration::from_millis(1600));
        let before = Instant::now();
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());
        assert_eq!(session.opens.lock().len(), 1);
        let copied = session.copied_urls.lock();
        assert_eq!(copied.first(), Some(&WRONG_URL));
        assert_eq!(copied.last(), Some(&TARGET_URL));
        assert!(before.elapsed() >= Duration::from_millis(1600));
        assert!(before.elapsed() < CARD_WINDOW);
    }

    #[tokio::test(start_paused = true)]
    async fn proof_transport_failure_is_not_retried_as_a_loading_card() {
        let mut session = ExactSession::new(false, TARGET_URL);
        session.clipboard_write_fails = true;
        let before = Instant::now();
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(format!("{error:#}").contains("clipboard transport failed"));
        assert_eq!(session.opens.lock().len(), 1);
        assert_eq!(session.clipboard_writes.load(Ordering::Relaxed), 1);
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
        assert_eq!(before.elapsed(), DISPATCH_SETTLE);
    }

    #[tokio::test(start_paused = true)]
    async fn explicit_unavailable_notice_after_dispatch_stops_before_copying() {
        let mut session = ExactSession::new(false, TARGET_URL);
        session.unavailable_text = Some(UNAVAILABLE_TEXT);
        let before = Instant::now();
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<CardObservationError>(),
            Some(CardObservationError::PostUnavailable)
        ));
        assert!(error.to_string().starts_with("target_post_unavailable:"));
        assert_eq!(session.opens.lock().len(), 1);
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
        assert_eq!(session.clipboard_writes.load(Ordering::Relaxed), 0);
        assert_eq!(before.elapsed(), DISPATCH_SETTLE);
    }

    #[tokio::test(start_paused = true)]
    async fn unavailable_notice_must_match_exactly_and_target_app_must_be_foreground() {
        let mut session = ExactSession::new(false, TARGET_URL);
        session.unavailable_text = Some("This video is unavailable sometimes");
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());

        let mut session = ExactSession::new(false, TARGET_URL);
        session.foreground = false;
        session.unavailable_text = Some(UNAVAILABLE_TEXT);
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(error.to_string().contains("foreground:"));
        assert!(!error.to_string().contains("target_post_unavailable"));
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn missing_card_predicates_are_reported_individually() {
        for predicate in ["Comments:", "author:", "caption:"] {
            let mut session = ExactSession::new(false, TARGET_URL);
            match predicate {
                "Comments:" => session.comments_missing = true,
                "author:" => session.author_missing = true,
                "caption:" => session.caption_nodes = vec![""],
                _ => unreachable!(),
            }
            let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
            assert!(error.to_string().contains(predicate), "{error:#}");
            assert_eq!(session.opens.lock().len(), 1);
            assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn diagnostic_target_urls_remove_query_fragment_and_bound_length() {
        let diagnostic =
            super::super::target_diagnostic_url(&format!("{TARGET_URL}?token=private#private"));
        assert_eq!(diagnostic, TARGET_URL);
        assert_eq!(
            super::super::target_diagnostic_url("https://user:private@www.tiktok.com/@a/photo/1"),
            "<invalid target URL>"
        );
        let long_url = format!("https://www.tiktok.com/@{}/photo/1", "a".repeat(800));
        assert_eq!(
            super::super::target_diagnostic_url(&long_url)
                .chars()
                .count(),
            512
        );
    }
}
