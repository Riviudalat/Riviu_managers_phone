use super::*;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

struct Phone {
    text: Mutex<String>,
    epoch: Mutex<String>,
    generation: AtomicU64,
    typed: Mutex<Vec<String>>,
    foreign: AtomicBool,
    clear_ack_lost: AtomicBool,
    stale: AtomicBool,
    body_ack_lost: AtomicBool,
    never_armed: AtomicBool,
    cancel_after_type: AtomicBool,
    stop: AtomicBool,
    public_taps: AtomicU64,
    readback_fails_after_clear: AtomicBool,
    focused: AtomicBool,
    ignore_focus: AtomicBool,
    collapsed_send: AtomicBool,
    focus_taps: AtomicU64,
    epoch_change_after_focus: AtomicBool,
    reply_open: AtomicBool,
    reply_hint: Mutex<String>,
    reply_snapshot_hint: Mutex<Option<String>>,
    literal_hint: AtomicBool,
    armed_hint: AtomicBool,
    clear_changes_hint: AtomicBool,
    final_text: Mutex<Option<String>>,
    final_hint_unknown: AtomicBool,
    focus_foreign: AtomicBool,
    settling_focus: AtomicBool,
}
impl Phone {
    fn new() -> Self {
        Self {
            text: Mutex::new(String::new()),
            epoch: Mutex::new("epoch-A".into()),
            generation: AtomicU64::new(0),
            typed: Mutex::new(vec![]),
            foreign: AtomicBool::new(false),
            clear_ack_lost: AtomicBool::new(false),
            stale: AtomicBool::new(false),
            body_ack_lost: AtomicBool::new(false),
            never_armed: AtomicBool::new(false),
            cancel_after_type: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            public_taps: AtomicU64::new(0),
            readback_fails_after_clear: AtomicBool::new(false),
            focused: AtomicBool::new(true),
            ignore_focus: AtomicBool::new(false),
            collapsed_send: AtomicBool::new(false),
            focus_taps: AtomicU64::new(0),
            epoch_change_after_focus: AtomicBool::new(false),
            reply_open: AtomicBool::new(false),
            reply_hint: Mutex::new("Replying to parent.author".into()),
            reply_snapshot_hint: Mutex::new(None),
            literal_hint: AtomicBool::new(false),
            armed_hint: AtomicBool::new(false),
            clear_changes_hint: AtomicBool::new(false),
            final_text: Mutex::new(None),
            final_hint_unknown: AtomicBool::new(false),
            focus_foreign: AtomicBool::new(false),
            settling_focus: AtomicBool::new(false),
        }
    }
}
#[async_trait::async_trait]
impl UiSession for Phone {
    async fn tap(&self, point: crate::TapPoint) -> anyhow::Result<()> {
        if point.x >= 900. && (100. ..=200.).contains(&point.y) {
            self.public_taps.fetch_add(1, Ordering::Relaxed);
            panic!("fixture must never dispatch Send");
        }
        if (300. ..=450.).contains(&point.x) && (1070. ..=1118.).contains(&point.y) {
            self.reply_open.store(true, Ordering::Relaxed);
        } else if (10. ..=800.).contains(&point.x) && (100. ..=200.).contains(&point.y) {
            self.focus_taps.fetch_add(1, Ordering::Relaxed);
            if !self.ignore_focus.load(Ordering::Relaxed) {
                self.focused.store(true, Ordering::Relaxed);
            }
        }
        Ok(())
    }
    async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
        Ok(())
    }
    async fn type_text(&self, text: &str) -> anyhow::Result<()> {
        self.typed.lock().push(text.into());
        *self.text.lock() = text.into();
        if text.is_empty() && self.clear_ack_lost.load(Ordering::Relaxed) {
            anyhow::bail!("lost clear ACK")
        }
        if !text.is_empty() {
            if self.cancel_after_type.load(Ordering::Relaxed) {
                self.stop.store(true, Ordering::Relaxed);
            }
            if self.body_ack_lost.load(Ordering::Relaxed) {
                anyhow::bail!("lost body ACK")
            }
        }
        Ok(())
    }
    async fn home(&self) -> anyhow::Result<()> {
        Ok(())
    }
    async fn back(&self) -> anyhow::Result<()> {
        panic!("cleanup must not navigate")
    }
    async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
        panic!("cleanup must not send")
    }
    async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
    fn stream_url(&self) -> Option<String> {
        None
    }
    fn supports_accessibility_readback(&self) -> bool {
        true
    }
    fn gui_session_epoch(&self) -> String {
        self.epoch.lock().clone()
    }
    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        Ok("com.ss.android.ugc.trill".into())
    }
    async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
        if matches!(
            query,
            ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT)
        ) {
            return Ok(Some(ElementBox {
                x: 10.,
                y: 100.,
                width: 790.,
                height: 100.,
                description: Some(self.text.lock().clone()),
                enabled: true,
                clickable: true,
            }));
        }
        if self.epoch_change_after_focus.load(Ordering::Relaxed)
            && self.focus_taps.load(Ordering::Relaxed) > 0
        {
            *self.epoch.lock() = "epoch-B".into();
        }
        Ok(Some(ElementBox {
            x: 1.,
            y: 1.,
            width: 10.,
            height: 10.,
            description: Some("@author".into()),
            enabled: true,
            clickable: true,
        }))
    }
    async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
        let (x, y, text) = match query {
            ElementQuery::Text { value: "parent body", .. } => (174., 1000., "parent body"),
            ElementQuery::Text { value: "Trả lời", .. }
            | ElementQuery::Description { value: "Trả lời", .. } => (315., 1070., "Trả lời"),
            _ => return Ok(vec![]),
        };
        Ok(vec![ElementBox {
            x, y, width: 130., height: 48., description: Some(text.into()),
            enabled: true, clickable: true,
        }])
    }
    async fn locate_all_described(
        &self,
        query: ElementQuery<'_>,
    ) -> anyhow::Result<Vec<ElementBox>> {
        if matches!(query, ElementQuery::ResourceIdSuffix(":id/desc")) {
            Ok(vec![ElementBox {
                x: 1.,
                y: 1.,
                width: 10.,
                height: 10.,
                description: Some("exact target caption".into()),
                enabled: true,
                clickable: false,
            }])
        } else if matches!(query, ElementQuery::ClassName(COMMENT_AUTHOR_CLASS)) {
            Ok(vec![ElementBox {
                x: 174., y: 940., width: 400., height: 48.,
                description: Some("parent.author".into()), enabled: true, clickable: true,
            }])
        } else if matches!(query, ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT)) {
            let text = if self.reply_open.load(Ordering::Relaxed) {
                self.reply_hint.lock().clone()
            } else {
                "Add comment...".into()
            };
            Ok(vec![ElementBox {
                x: 10., y: 100., width: 790., height: 100.,
                description: Some(text), enabled: true, clickable: true,
            }])
        } else {
            Ok(vec![])
        }
    }
    async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> {
        if self.readback_fails_after_clear.load(Ordering::Relaxed)
            && self.typed.lock().last().is_some_and(|text| text.is_empty())
        {
            anyhow::bail!("readback unavailable after clear");
        }
        let generation = if self.stale.load(Ordering::Relaxed) {
            1
        } else {
            self.generation.fetch_add(1, Ordering::Relaxed) + 1
        };
        let before_type = self.typed.lock().is_empty();
        let final_read = before_type && generation >= 4;
        let reply_empty = self.reply_open.load(Ordering::Relaxed) && self.text.lock().is_empty();
        let text = if self.focus_foreign.load(Ordering::Relaxed) && before_type
            && (generation == 1 || final_read)
        {
            "foreign".to_owned()
        } else if final_read && self.final_text.lock().is_some() {
            self.final_text.lock().clone().unwrap()
        } else if self.foreign.load(Ordering::Relaxed) {
            "foreign".to_owned()
        } else if reply_empty {
            if self.clear_changes_hint.load(Ordering::Relaxed)
                && self.typed.lock().last().is_some_and(|text| text.is_empty())
            {
                "Add comment...".into()
            } else {
                self.reply_snapshot_hint.lock().clone()
                    .unwrap_or_else(|| self.reply_hint.lock().clone())
            }
        } else {
            self.text.lock().clone()
        };
        let focused = self.focused.load(Ordering::Relaxed)
            && !self.settling_focus.swap(false, Ordering::Relaxed);
        let hint = if final_read && self.final_hint_unknown.load(Ordering::Relaxed) {
            "unknown"
        } else if reply_empty && !self.literal_hint.load(Ordering::Relaxed) {
            "true"
        } else {
            "false"
        };
        let armed = if reply_empty {
            self.armed_hint.load(Ordering::Relaxed)
        } else {
            !text.is_empty() && !self.never_armed.load(Ordering::Relaxed)
        };
        let send = if !focused && self.collapsed_send.load(Ordering::Relaxed) {
            String::new()
        } else {
            format!(r#"<node package="com.ss.android.ugc.trill" class="android.widget.Button" content-desc="@2131823284" text="" visible-to-user="true" enabled="{armed}" clickable="true" bounds="[900,100][1000,200]"/>"#)
        };
        Ok(crate::HierarchySourceSnapshot {
            generation,
            xml: format!(
                r#"<hierarchy><node package="com.ss.android.ugc.trill" class="android.widget.EditText" text="{text}" visible-to-user="true" enabled="true" focused="{focused}" showing-hint="{hint}" password="false" bounds="[10,100][800,200]"/>{send}</hierarchy>"#,
            ),
        })
    }
}
fn labels() -> TikTokControls {
    crate::tiktok_labels::controls_for("com.ss.android.ugc.trill", "vi", "46.3.3").unwrap()
}
async fn guard(phone: &Phone) -> DraftGuard<'_> {
    DraftGuard::capture(phone, labels(), "owned draft")
        .await
        .expect("fixture exact empty binding")
}

#[tokio::test]
async fn production_draft_cleanup_lost_clear_ack_is_settled_by_fresh_read() {
    let phone = Phone::new();
    let guard = guard(&phone).await;
    *phone.text.lock() = "owned draft".into();
    phone.clear_ack_lost.store(true, Ordering::Relaxed);
    assert_eq!(
        guard.cleanup(&phone, labels()).await,
        DraftCleanup::ClearedAndVerified
    );
    assert_eq!(phone.typed.lock().as_slice(), &[""]);
}
#[tokio::test]
async fn production_draft_cleanup_foreign_or_epoch_changed_never_erases() {
    let phone = Phone::new();
    let guard = guard(&phone).await;
    *phone.text.lock() = "owned draft".into();
    phone.foreign.store(true, Ordering::Relaxed);
    assert_eq!(
        guard.cleanup(&phone, labels()).await,
        DraftCleanup::RefusedDraftChanged
    );
    assert!(phone.typed.lock().is_empty());
    phone.foreign.store(false, Ordering::Relaxed);
    *phone.epoch.lock() = "epoch-B".into();
    assert_eq!(
        guard.cleanup(&phone, labels()).await,
        DraftCleanup::RefusedBindingChanged
    );
    assert!(phone.typed.lock().is_empty());
}
#[tokio::test]
async fn production_draft_cleanup_stale_generation_never_erases() {
    let phone = Phone::new();
    let guard = guard(&phone).await;
    *phone.text.lock() = "owned draft".into();
    phone.stale.store(true, Ordering::Relaxed);
    assert_eq!(
        guard.cleanup(&phone, labels()).await,
        DraftCleanup::FailedReadback
    );
    assert!(phone.typed.lock().is_empty());
}
#[tokio::test]
async fn production_draft_cleanup_ownership_loss_avoids_all_writes() {
    let phone = Phone::new();
    let guard = guard(&phone).await;
    *phone.text.lock() = "owned draft".into();
    assert_eq!(
        settle(&phone, labels(), Some(&guard), true, true).await,
        DraftCleanup::RefusedBindingChanged
    );
    assert!(phone.typed.lock().is_empty());
}
#[tokio::test]
async fn production_draft_cleanup_recorder_notarmed_cancel_all_use_same_settlement() {
    for reason in ["type_ack_lost", "not_armed", "recorder_failed", "cancelled"] {
        let phone = Phone::new();
        let guard = guard(&phone).await;
        *phone.text.lock() = "owned draft".into();
        let failure: Result<(), HierarchySendFailure> =
            Err(HierarchySendFailure::before(anyhow::anyhow!(reason)));
        let result = super::super::finish_draft_settlement(
            &phone,
            labels(),
            Some(&guard),
            true,
            false,
            failure,
        )
        .await;
        assert!(
            matches!(result, Err(HierarchySendFailure::BeforeEffect(_))),
            "{reason}"
        );
        assert_eq!(phone.typed.lock().as_slice(), &[""], "{reason}");
    }
}
#[tokio::test]
async fn production_draft_cleanup_unbound_is_pending_not_public_effect() {
    let phone = Phone::new();
    let result =
        super::super::finish_draft_settlement(&phone, labels(), None, true, false, Ok(())).await;
    assert!(matches!(
        result,
        Err(HierarchySendFailure::DraftCleanupPending {
            cleanup: DraftCleanup::RefusedBindingChanged,
            ..
        })
    ));
    assert!(phone.typed.lock().is_empty());
}

#[tokio::test(start_paused = true)]
async fn production_draft_cleanup_actual_root_lost_type_ack_cleans_without_send() {
    let phone = Phone::new();
    phone.focused.store(false, Ordering::Relaxed);
    phone.collapsed_send.store(true, Ordering::Relaxed);
    phone.settling_focus.store(true, Ordering::Relaxed);
    phone.body_ack_lost.store(true, Ordering::Relaxed);
    let mut gate = crate::interaction_target::EffectGate::new(|| {
        panic!("type ACK failure must not cross Send gate")
    });
    let result = super::super::send_root_by_hierarchy_with_gate(
        &phone,
        labels(),
        (1080., 2400.),
        "owned draft",
        &[],
        &phone.stop,
        String::new,
        &mut gate,
    )
    .await
    .unwrap();
    assert_eq!(
        result.verdict,
        crate::tiktok_drawer::CommentVerdict::SendFlowInterrupted
    );
    assert_eq!(result.cleanup, DraftCleanup::ClearedAndVerified);
    assert_eq!(phone.typed.lock().as_slice(), &["owned draft", ""]);
    assert_eq!(phone.public_taps.load(Ordering::Relaxed), 0);
    assert_eq!(phone.focus_taps.load(Ordering::Relaxed), 1, "never re-tap collapsed geometry after proof");
}

#[tokio::test(start_paused = true)]
async fn production_draft_cleanup_actual_root_unfocused_or_changed_epoch_never_types() {
    for changed_epoch in [false, true] {
        let phone = Phone::new();
        phone.focused.store(false, Ordering::Relaxed);
        phone.ignore_focus.store(!changed_epoch, Ordering::Relaxed);
        phone.epoch_change_after_focus.store(changed_epoch, Ordering::Relaxed);
        phone.body_ack_lost.store(true, Ordering::Relaxed);
        let mut gate = crate::interaction_target::EffectGate::new(|| panic!("unproved focus cannot cross Send"));
        let result = super::super::send_root_by_hierarchy_with_gate(
            &phone, labels(), (1080., 2400.), "owned draft", &[], &phone.stop,
            String::new, &mut gate,
        ).await;
        assert!(phone.typed.lock().is_empty(), "changed_epoch={changed_epoch}");
        assert_eq!(phone.public_taps.load(Ordering::Relaxed), 0);
        if !changed_epoch {
            assert_eq!(result.expect("no typing means no cleanup pending").cleanup, DraftCleanup::NotTyped);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn production_draft_cleanup_actual_reply_exact_hint_and_cleanup_stay_parent_bound() {
    for case in ["exact", "wrong", "root", "literal", "armed", "root_after_clear"] {
        let phone = Phone::new();
        phone.body_ack_lost.store(true, Ordering::Relaxed);
        match case {
            "wrong" => *phone.reply_snapshot_hint.lock() = Some("Replying to other.author".into()),
            "root" => *phone.reply_snapshot_hint.lock() = Some("Add comment...".into()),
            "literal" => phone.literal_hint.store(true, Ordering::Relaxed),
            "armed" => phone.armed_hint.store(true, Ordering::Relaxed),
            "root_after_clear" => phone.clear_changes_hint.store(true, Ordering::Relaxed),
            _ => {}
        }
        let parent = CommentLocatorIdentity {
            comment_link: None, author_label: "parent.author".into(), text: "parent body".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(), frame_sha256: "fixture-parent".into(),
        };
        let mut gate = crate::interaction_target::EffectGate::new(|| panic!("lost typing ACK cannot cross Send"));
        let result = super::super::send_reply_by_hierarchy_with_gate(
            &phone, labels(), (1080., 2400.), &parent, "owned draft", &phone.stop,
            String::new, &mut gate,
        ).await;
        if case == "exact" {
            let outcome = result.expect("exact parent hint is an empty reply draft")
                .expect("fixture parent row is proved");
            assert_eq!(outcome.cleanup, DraftCleanup::ClearedAndVerified);
            assert_eq!(phone.typed.lock().as_slice(), &["owned draft", ""]);
            assert_eq!(phone.focus_taps.load(Ordering::Relaxed), 1);
        } else if case == "root_after_clear" {
            assert!(matches!(result, Err(HierarchySendFailure::DraftCleanupPending {
                cleanup: DraftCleanup::FailedReadback, ..
            })), "{case}: root hint cannot settle the reply draft");
            assert_eq!(phone.typed.lock().as_slice(), &["owned draft", ""]);
        } else {
            assert!(matches!(result, Err(HierarchySendFailure::DraftCleanupPending {
                cleanup: DraftCleanup::RefusedDraftChanged, ..
            })), "{case}: refuse before typing");
            assert!(phone.typed.lock().is_empty(), "{case}");
        }
        assert_eq!(phone.public_taps.load(Ordering::Relaxed), 0, "{case}");
    }
}

#[tokio::test(start_paused = true)]
async fn production_draft_cleanup_actual_final_snapshot_change_never_types() {
    for case in ["final_foreign", "final_unknown", "reply_root", "focus_foreign_then_empty_then_foreign"] {
        let phone = Phone::new();
        phone.never_armed.store(true, Ordering::Relaxed);
        phone.body_ack_lost.store(true, Ordering::Relaxed);
        match case {
            "final_foreign" => *phone.final_text.lock() = Some("foreign draft".into()),
            "final_unknown" => phone.final_hint_unknown.store(true, Ordering::Relaxed),
            "reply_root" => *phone.final_text.lock() = Some("Add comment...".into()),
            _ => phone.focus_foreign.store(true, Ordering::Relaxed),
        }
        let mut gate = crate::interaction_target::EffectGate::new(|| panic!("changed draft cannot cross Send"));
        let result = if case == "reply_root" {
            let parent = CommentLocatorIdentity {
                comment_link: None, author_label: "parent.author".into(), text: "parent body".into(),
                locator_version: HIERARCHY_LOCATOR_VERSION.into(), frame_sha256: "fixture-parent".into(),
            };
            super::super::send_reply_by_hierarchy_with_gate(
                &phone, labels(), (1080., 2400.), &parent, "owned draft", &phone.stop,
                String::new, &mut gate,
            ).await.map(|value| value.expect("fixture parent is proved"))
        } else {
            super::super::send_root_by_hierarchy_with_gate(
                &phone, labels(), (1080., 2400.), "owned draft", &[], &phone.stop,
                String::new, &mut gate,
            ).await
        };
        let expected = if case == "final_unknown" {
            DraftCleanup::FailedReadback
        } else {
            DraftCleanup::RefusedDraftChanged
        };
        assert!(matches!(result, Err(HierarchySendFailure::DraftCleanupPending { cleanup, .. })
            if cleanup == expected), "{case}: changed or unknown draft must block retry");
        assert!(phone.typed.lock().is_empty(), "{case}");
        assert_eq!(phone.public_taps.load(Ordering::Relaxed), 0, "{case}");
    }
}
#[tokio::test(start_paused = true)]
async fn production_draft_cleanup_actual_root_notarmed_cleans_without_send() {
    let phone = Phone::new();
    phone.never_armed.store(true, Ordering::Relaxed);
    let mut gate =
        crate::interaction_target::EffectGate::new(|| panic!("NotArmed must not cross Send gate"));
    let result = super::super::send_root_by_hierarchy_with_gate(
        &phone,
        labels(),
        (1080., 2400.),
        "owned draft",
        &[],
        &phone.stop,
        String::new,
        &mut gate,
    )
    .await
    .unwrap();
    assert_eq!(
        result.verdict,
        crate::tiktok_drawer::CommentVerdict::NotArmed
    );
    assert_eq!(result.cleanup, DraftCleanup::ClearedAndVerified);
    assert_eq!(phone.typed.lock().as_slice(), &["owned draft", ""]);
    assert_eq!(phone.public_taps.load(Ordering::Relaxed), 0);
}
#[tokio::test(start_paused = true)]
async fn production_draft_cleanup_actual_root_recorder_failure_cleans_without_send() {
    let phone = Phone::new();
    let mut gate = crate::interaction_target::EffectGate::new(|| {
        panic!("recorder failure must not cross Send gate")
    });
    gate.record_draft_with(|_| anyhow::bail!("recorder unavailable"));
    let result = super::super::send_root_by_hierarchy_with_gate(
        &phone,
        labels(),
        (1080., 2400.),
        "owned draft",
        &[],
        &phone.stop,
        String::new,
        &mut gate,
    )
    .await;
    assert!(matches!(result, Err(HierarchySendFailure::BeforeEffect(_))));
    assert_eq!(phone.typed.lock().as_slice(), &["owned draft", ""]);
    assert_eq!(phone.public_taps.load(Ordering::Relaxed), 0);
}
#[tokio::test(start_paused = true)]
async fn production_draft_cleanup_actual_root_cancel_after_type_still_cleans() {
    let phone = Phone::new();
    phone.cancel_after_type.store(true, Ordering::Relaxed);
    let mut gate =
        crate::interaction_target::EffectGate::new(|| panic!("cancel must not cross Send gate"));
    let result = super::super::send_root_by_hierarchy_with_gate(
        &phone,
        labels(),
        (1080., 2400.),
        "owned draft",
        &[],
        &phone.stop,
        String::new,
        &mut gate,
    )
    .await
    .unwrap();
    assert!(phone.stop.load(Ordering::Relaxed));
    assert_eq!(result.cleanup, DraftCleanup::ClearedAndVerified);
    assert_eq!(phone.typed.lock().as_slice(), &["owned draft", ""]);
    assert_eq!(phone.public_taps.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn production_draft_cleanup_readback_failure_keeps_attention_and_never_replays_clear() {
    let phone = Phone::new();
    let guard = guard(&phone).await;
    *phone.text.lock() = "owned draft".into();
    phone
        .readback_fails_after_clear
        .store(true, Ordering::Relaxed);
    assert_eq!(
        guard.cleanup(&phone, labels()).await,
        DraftCleanup::FailedReadback
    );
    assert_eq!(phone.typed.lock().as_slice(), &[""]);
    assert!(!DraftCleanup::FailedReadback.retry_safe());
}
#[tokio::test]
async fn production_draft_cleanup_same_epoch_different_session_never_erases() {
    let owner = Phone::new();
    let guard = guard(&owner).await;
    let foreign = Phone::new();
    *foreign.text.lock() = "owned draft".into();
    assert_eq!(
        guard.cleanup(&foreign, labels()).await,
        DraftCleanup::RefusedBindingChanged
    );
    assert!(owner.typed.lock().is_empty() && foreign.typed.lock().is_empty());
}

#[tokio::test(start_paused = true)]
async fn production_draft_cleanup_actual_root_foreign_disarmed_baseline_never_overwrites() {
    let phone = Phone::new();
    *phone.text.lock() = "foreign draft".into();
    phone.never_armed.store(true, Ordering::Relaxed);
    let mut gate =
        crate::interaction_target::EffectGate::new(|| panic!("foreign draft cannot cross Send"));
    let result = super::super::send_root_by_hierarchy_with_gate(
        &phone,
        labels(),
        (1080., 2400.),
        "owned draft",
        &[],
        &phone.stop,
        String::new,
        &mut gate,
    )
    .await;
    assert!(matches!(
        result,
        Err(HierarchySendFailure::DraftCleanupPending {
            cleanup: DraftCleanup::RefusedDraftChanged,
            ..
        })
    ));
    assert!(phone.typed.lock().is_empty());
    assert_eq!(phone.text.lock().as_str(), "foreign draft");
    assert_eq!(phone.public_taps.load(Ordering::Relaxed), 0);
}
