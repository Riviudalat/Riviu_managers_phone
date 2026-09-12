//! Read the active composer, never the collapsed "Replying to ..." placeholder.
use super::*;
use crate::ui_automation::tree::Tree;

const READ_WINDOW: Duration = Duration::from_secs(4);

pub(super) fn from_snapshot(
    snapshot: crate::HierarchySourceSnapshot,
) -> anyhow::Result<Option<String>> {
    let tree = Tree::parse(snapshot).context("composer_snapshot_invalid")?;
    let focused: Vec<_> = tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            matches!(
                node.attr("package"),
                "com.ss.android.ugc.trill" | "com.zhiliaoapp.musically"
            ) && node.visible(node.attr("package"))
                && tree.ancestors_visible(*index)
                && node.attr("class") == crate::tiktok_drawer::EDIT_TEXT
                && node.attr("focused") == "true"
                && node.attr("enabled") == "true"
                && node.rect().is_some()
        })
        .map(|(_, node)| node)
        .collect();
    // Measured on SEA38.3.2 / SM-G955U1: identical IDs, two displayed EditTexts,
    // first showing-hint=true/focused=false, second carrying the linked token.
    anyhow::ensure!(
        focused.len() <= 1,
        "composer_ambiguous: {} ô nhập đang có focus",
        focused.len()
    );
    let Some(node) = focused.first() else {
        return Ok(None);
    };
    if node.attr("showing-hint") == "true" {
        return Ok(None);
    }
    let text = node.attr("text").trim();
    Ok((!text.is_empty()).then(|| text.to_owned()))
}

pub(super) async fn read(session: &dyn UiSession) -> anyhow::Result<Option<String>> {
    if session.supports_accessibility_readback() {
        return from_snapshot(
            session
                .hierarchy_source_snapshot()
                .await
                .context("composer_read_failed")?,
        );
    }
    // Non-Android/test sessions retain their documented element API. Android never
    // falls back after a snapshot failure, which would erase transport evidence.
    let fields = session
        .locate_all_described(ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT))
        .await?;
    let values: Vec<_> = fields
        .into_iter()
        .filter_map(|f| f.description)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    anyhow::ensure!(values.len() <= 1, "composer_ambiguous: nhiều ô nhập");
    Ok(values.into_iter().next())
}

pub(super) fn has_handle(text: &str, handle: &str) -> bool {
    text.split('@').skip(1).any(|part| {
        let token: String = part
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            .collect();
        token.eq_ignore_ascii_case(handle.trim_start_matches('@'))
    })
}

fn ensure_running(session: &dyn UiSession, stop: &AtomicBool) -> anyhow::Result<()> {
    anyhow::ensure!(
        !stop.load(Ordering::Relaxed),
        "composer_cancelled: phiên đã dừng"
    );
    anyhow::ensure!(
        session
            .gui_scope()
            .and_then(|s| s.deadline_ms)
            .is_none_or(|end| chrono::Utc::now().timestamp_millis() < end),
        "composer_deadline: phiên đã hết giờ"
    );
    Ok(())
}

pub(super) async fn after_pick(
    session: &dyn UiSession,
    stop: &AtomicBool,
    handle: &str,
    body: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let deadline = Instant::now() + READ_WINDOW;
    let epoch = session.gui_session_epoch();
    let mut last = "composer_missing";
    loop {
        ensure_running(session, stop)?;
        anyhow::ensure!(
            session.gui_session_epoch() == epoch,
            "composer_session_changed: phiên thiết bị đã đổi"
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("{last}: chưa xác nhận ô soạn sau 4 giây");
        }
        // Cancellation/deadline also apply while a device HTTP request is outstanding.
        let interrupted = async {
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                ensure_running(session, stop)?;
            }
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        };
        let value = tokio::select! {
            value = tokio::time::timeout(remaining, read(session)) => value.context("composer_read_timeout")??,
            result = interrupted => { result?; unreachable!() },
        };
        ensure_running(session, stop)?;
        anyhow::ensure!(
            session.gui_session_epoch() == epoch,
            "composer_session_changed: phiên thiết bị đã đổi"
        );
        if let Some(value) = value {
            anyhow::ensure!(
                body.is_none_or(|body| value.contains(body)),
                "composer_content_changed: nội dung đã đổi sau chọn @{handle}"
            );
            // The caller records an exact-handle mismatch as unverified, never linked.
            return Ok(Some(value));
        }
        last = "composer_missing_or_empty";
        tokio::time::sleep(
            MENTION_PICKER_POLL.min(deadline.saturating_duration_since(Instant::now())),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::VecDeque;

    fn xml(fields: &str) -> crate::HierarchySourceSnapshot {
        crate::HierarchySourceSnapshot {
            generation: 1,
            xml: format!(
                "<hierarchy><node package=\"com.ss.android.ugc.trill\">{fields}</node></hierarchy>"
            ),
        }
    }
    fn field(text: &str, focused: bool) -> String {
        format!(
            r#"<node package="com.ss.android.ugc.trill" class="android.widget.EditText" resource-id="com.ss.android.ugc.trill:id/cnd" text="{text}" focused="{focused}" displayed="true" enabled="true" showing-hint="{}" bounds="[216,797][1002,954]"/>"#,
            !focused
        )
    }
    struct Session {
        observations: Mutex<VecDeque<anyhow::Result<crate::HierarchySourceSnapshot>>>,
        deadline_ms: Option<i64>,
        stalled: bool,
    }
    impl Session {
        fn new(observations: Vec<crate::HierarchySourceSnapshot>) -> Self {
            Self {
                observations: Mutex::new(observations.into_iter().map(Ok).collect()),
                deadline_ms: None,
                stalled: false,
            }
        }
    }
    #[async_trait::async_trait]
    impl UiSession for Session {
        async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
            panic!("readback must never tap")
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            panic!("readback must never swipe")
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("readback must never replace text")
        }
        async fn type_keys(&self, _: &str) -> anyhow::Result<()> {
            panic!("readback must never type a second tag")
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("readback must never navigate")
        }
        async fn back(&self) -> anyhow::Result<()> {
            panic!("readback must never navigate")
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("readback must never tap")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            panic!("readback must use its snapshot")
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        fn supports_accessibility_readback(&self) -> bool {
            true
        }
        fn gui_scope(&self) -> Option<crate::ui_automation::GuiScope> {
            Some(crate::ui_automation::GuiScope {
                run_id: "test".into(),
                assignment_id: None,
                device_id: "test".into(),
                deadline_ms: self.deadline_ms,
            })
        }
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            if self.stalled {
                std::future::pending::<()>().await;
            }
            self.observations
                .lock()
                .pop_front()
                .unwrap_or_else(|| Ok(xml("")))
        }
    }

    #[tokio::test]
    async fn duplicate_id_reads_focused_reply_body_instead_of_collapsed_hint() {
        let active = "Kiem thu @test.a ";
        let snapshot = xml(&(field("Replying to test.a", false) + &field(active, true)));
        assert_eq!(
            read(&Session::new(vec![snapshot]))
                .await
                .unwrap()
                .as_deref(),
            Some(active.trim())
        );
    }
    #[test]
    fn hidden_foreign_hint_and_ambiguous_fields_cannot_confirm_a_composer() {
        for fields in [
            field("hint", false),
            field("body @a", true).replace("com.ss.android.ugc.trill", "other.app"),
            field("body @a", true).replace("displayed=\"true\"", "displayed=\"false\""),
            format!(
                "<node displayed=\"false\">{}</node>",
                field("body @a", true)
            ),
        ] {
            assert!(from_snapshot(xml(&fields)).unwrap().is_none());
        }
        assert!(
            from_snapshot(xml(&(field("one @a", true) + &field("two @a", true))))
                .unwrap_err()
                .to_string()
                .contains("composer_ambiguous")
        );
        assert!(!has_handle("body @a.someone", "a"));
        assert!(has_handle("body @Test.A ", "test.a"));
    }
    #[tokio::test(start_paused = true)]
    async fn delayed_snapshot_settles_without_retyping_or_retapping() {
        let session = Session::new(vec![
            xml(""),
            xml(&field("Replying to a", false)),
            xml(&field("body @a", true)),
        ]);
        assert_eq!(
            after_pick(&session, &AtomicBool::new(false), "a", Some("body"))
                .await
                .unwrap()
                .as_deref(),
            Some("body @a")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn changed_body_is_rejected_even_when_the_tag_survives() {
        let session = Session::new(vec![xml(&field("different body @a", true))]);
        let error = after_pick(
            &session,
            &AtomicBool::new(false),
            "a",
            Some("approved text"),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("composer_content_changed"));
    }
    #[tokio::test(start_paused = true)]
    async fn transport_failure_is_preserved_and_never_becomes_empty() {
        let session = Session::new(vec![]);
        session
            .observations
            .lock()
            .push_back(Err(anyhow::anyhow!("transport disconnected")));
        let error = after_pick(&session, &AtomicBool::new(false), "a", None)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("transport disconnected"));
    }
    #[tokio::test(start_paused = true)]
    async fn absent_or_stalled_composer_stops_within_four_seconds() {
        for stalled in [false, true] {
            let mut session = Session::new(vec![]);
            session.stalled = stalled;
            let start = Instant::now();
            assert!(after_pick(&session, &AtomicBool::new(false), "a", None)
                .await
                .is_err());
            assert!(start.elapsed() <= READ_WINDOW);
        }
    }
    #[tokio::test(start_paused = true)]
    async fn cancelled_or_expired_session_does_not_read_and_cancel_interrupts_http() {
        let mut session = Session::new(vec![]);
        session.stalled = true;
        assert!(after_pick(&session, &AtomicBool::new(true), "a", None)
            .await
            .unwrap_err()
            .to_string()
            .contains("composer_cancelled"));
        session.deadline_ms = Some(0);
        assert!(after_pick(&session, &AtomicBool::new(false), "a", None)
            .await
            .unwrap_err()
            .to_string()
            .contains("composer_deadline"));
        session.deadline_ms = None;
        let stop = AtomicBool::new(false);
        let task = after_pick(&session, &stop, "a", None);
        let cancel = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            stop.store(true, Ordering::Relaxed);
        };
        let (result, ()) = tokio::join!(task, cancel);
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("composer_cancelled"));
    }
}
