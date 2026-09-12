use super::{model::*, runtime::resolve_navigation};
use crate::{SwipeGesture, TapPoint, UiSession};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

struct Session {
    reads: AtomicUsize,
    change: bool,
}
#[async_trait::async_trait]
impl UiSession for Session {
    fn stream_url(&self) -> Option<String> {
        None
    }
    fn gui_session_epoch(&self) -> String {
        "epoch".into()
    }
    fn supports_element_bounds(&self) -> bool {
        true
    }
    async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        Ok((400., 800.))
    }
    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        Ok("com.android.settings".into())
    }
    async fn ui_language(&self) -> Option<String> {
        Some("en-US".into())
    }
    async fn app_version(&self, _: &str) -> Option<String> {
        Some("unknown".into())
    }
    async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> {
        let read = self.reads.fetch_add(1, Ordering::SeqCst);
        let text = if self.change && read > 0 {
            "Factory reset"
        } else {
            "About phone"
        };
        Ok(crate::HierarchySourceSnapshot{generation:read as u64+1,xml:format!("<hierarchy><node package=\"com.android.settings\" text=\"Settings\" bounds=\"[0,0][400,50]\"/><node package=\"com.android.settings\" text=\"{text}\" bounds=\"[10,100][200,150]\" enabled=\"true\" clickable=\"true\"/></hierarchy>")})
    }
    async fn tap(&self, _: TapPoint) -> anyhow::Result<()> {
        panic!("resolver must never tap")
    }
    async fn swipe(&self, _: SwipeGesture) -> anyhow::Result<()> {
        panic!("resolver must never swipe")
    }
    async fn type_text(&self, _: &str) -> anyhow::Result<()> {
        panic!("resolver must never type")
    }
    async fn home(&self) -> anyhow::Result<()> {
        panic!("resolver must never navigate")
    }
    async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
        panic!("resolver must never tap")
    }
    async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
        unreachable!()
    }
}
#[tokio::test]
async fn target_is_reobserved_and_a_changed_target_is_rejected() {
    for change in [false, true] {
        let session = Session {
            reads: AtomicUsize::new(0),
            change,
        };
        let result = resolve_navigation(&session, "aboutDevice", Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(result.is_some(), !change);
        assert_eq!(session.reads.load(Ordering::SeqCst), 2);
    }
}

#[test]
fn rust_contract_matches_python_wire_fixture() {
    let request: GuiRequest =
        serde_json::from_str(include_str!("../../fixtures/gui-request.json")).unwrap();
    assert_eq!(request.protocol_version, 1);
    assert_eq!(request.app.system_locale, "en-US");
    assert_eq!(request.nodes[0].id, 3);
    let encoded = serde_json::to_value(request).unwrap();
    assert!(encoded.get("observationId").is_some());
}
