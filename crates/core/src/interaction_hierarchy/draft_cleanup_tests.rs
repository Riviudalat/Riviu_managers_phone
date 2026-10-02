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
        let text = if self.foreign.load(Ordering::Relaxed) {
            "foreign".to_owned()
        } else {
            self.text.lock().clone()
        };
        let generation = if self.stale.load(Ordering::Relaxed) {
            1
        } else {
            self.generation.fetch_add(1, Ordering::Relaxed) + 1
        };
        Ok(crate::HierarchySourceSnapshot {
            generation,
            xml: format!(
                r#"<hierarchy><node package="com.ss.android.ugc.trill" class="android.widget.EditText" text="{text}" visible-to-user="true" enabled="true" focused="true" showing-hint="false" password="false" bounds="[10,100][800,200]"/><node package="com.ss.android.ugc.trill" class="android.widget.Button" content-desc="@2131823284" text="" visible-to-user="true" enabled="{}" clickable="true" bounds="[900,100][1000,200]"/></hierarchy>"#,
                !text.is_empty() && !self.never_armed.load(Ordering::Relaxed)
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
