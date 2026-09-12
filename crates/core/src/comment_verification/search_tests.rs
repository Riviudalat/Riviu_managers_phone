use super::*;
const PKG: &str = "com.zhiliaoapp.musically";

struct ClippedParent(std::sync::atomic::AtomicUsize);
#[async_trait::async_trait]
impl UiSession for ClippedParent {
    async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
        panic!("parent search does not tap Send")
    }
    async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    async fn type_text(&self, _: &str) -> anyhow::Result<()> {
        panic!("parent search must not type")
    }
    async fn home(&self) -> anyhow::Result<()> {
        unreachable!()
    }
    async fn back(&self) -> anyhow::Result<()> {
        unreachable!()
    }
    async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
        unreachable!()
    }
    async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
        unreachable!()
    }
    fn stream_url(&self) -> Option<String> {
        None
    }
    async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        Ok((1080., 2220.))
    }
    async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> {
        let reply = if self.0.load(Ordering::Relaxed) > 0 {
            format!(
                r#"<node package="{PKG}" class="android.widget.Button" text="Reply" bounds="[220,1120][360,1170]"/>"#
            )
        } else {
            String::new()
        };
        Ok(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: format!(
                r#"<hierarchy><node package="{PKG}"><node package="{PKG}" class="android.widget.Button" text="Alice" bounds="[155,1000][600,1050]"/><node package="{PKG}" class="android.widget.TextView" text="parent" bounds="[155,1055][1000,1110]"/>{reply}</node><node package="{PKG}" class="android.widget.EditText" bounds="[189,1961][721,2039]"/></hierarchy>"#
            ),
        })
    }
}
#[tokio::test(start_paused = true)]
async fn parent_with_reply_below_viewport_keeps_scrolling_without_typing() {
    let session = ClippedParent(std::sync::atomic::AtomicUsize::new(0));
    let parent = CommentLocatorIdentity {
        author_label: "Alice".into(),
        text: "parent".into(),
        locator_version: "test".into(),
        frame_sha256: "sha".into(),
    };
    let found = find_for_reply(
        &session,
        crate::tiktok_labels::controls_for(PKG, "en", "46.2.1").unwrap(),
        &parent,
        None,
        &AtomicBool::new(false),
    )
    .await
    .unwrap();
    assert!(found.reply.is_some());
    assert_eq!(session.0.load(Ordering::Relaxed), 1);
}
fn tree(xml: &str) -> Tree {
    Tree::parse(crate::HierarchySourceSnapshot {
        generation: 1,
        xml: format!("<hierarchy>{xml}</hierarchy>"),
    })
    .unwrap()
}
#[test]
fn hidden_label_uses_only_its_own_clickable_parent() {
    let raw = format!(
        r#"<node package="{PKG}" enabled="true" clickable="true" bounds="[0,1748][1080,1935]"><node package="{PKG}" text="Community-flagged comments" enabled="true" clickable="false" bounds="[147,1780][614,1822]"/><node package="{PKG}" text="These comments were flagged by our community" bounds="[147,1827][1048,1911]"/></node>"#
    );
    let control = hidden_control(&tree(&raw), PKG).unwrap().unwrap();
    assert_eq!(control.y, 1748.0);
    assert!(hidden_control(
        &tree(&raw.replace("clickable=\"true\"", "clickable=\"false\"")),
        PKG
    )
    .is_err());
    assert!(hidden_control(&tree(&(raw.clone() + &raw)), PKG).is_err());
    assert!(hidden_control(
        &tree(&raw.replace("package=\"com.zhiliaoapp.musically\"", "package=\"other\"")),
        PKG
    )
    .unwrap()
    .is_none());
}
#[test]
fn row_identity_stays_with_author_and_does_not_match_same_text_twice() {
    let raw = format!(
        r#"<node package="{PKG}"><node package="{PKG}" class="android.widget.Button" text="Alice" bounds="[155,1000][600,1050]"/><node package="{PKG}" class="android.widget.TextView" text="approved" bounds="[155,1055][1000,1110]"/><node package="{PKG}" class="android.widget.Button" text="Reply" bounds="[220,1120][360,1170]"/></node>"#
    );
    assert_eq!(
        row(&tree(&raw), PKG, "approved", Some("Alice"))
            .unwrap()
            .unwrap()
            .identity
            .author_label,
        "Alice"
    );
    assert!(row(&tree(&raw), PKG, "approved", Some("Bob"))
        .unwrap()
        .is_none());
    assert!(row(&tree(&(raw.clone() + &raw)), PKG, "approved", None).is_err());
}
#[test]
fn verification_worker_has_no_text_or_send_route() {
    let worker = include_str!("worker.rs");
    for forbidden in [
        ".type_text(",
        ".type_keys(",
        ".tap_send_and_confirm_disarm(",
        "begin_interaction_comment_action_effect(",
    ] {
        assert!(!worker.contains(forbidden));
    }
}

#[test]
fn parent_proof_rejects_a_reply_under_a_different_root() {
    let xml = format!(
        r#"<node package="{PKG}">
 <node package="{PKG}"><node package="{PKG}" class="android.widget.Button" text="Alice" bounds="[155,900][600,950]"/><node package="{PKG}" class="android.widget.TextView" resource-id="{PKG}:id/body" text="root" bounds="[155,955][1000,1000]"/></node>
 <node package="{PKG}"><node package="{PKG}" class="android.widget.Button" text="Bob" bounds="[230,1150][600,1200]"/><node package="{PKG}" class="android.widget.TextView" resource-id="{PKG}:id/body" text="reply" bounds="[230,1205][1000,1260]"/></node></node>"#
    );
    let t = tree(&xml);
    let mut found = row(&t, PKG, "reply", Some("Bob")).unwrap().unwrap();
    found.snapshot = format!("<hierarchy>{xml}</hierarchy>");
    let parent = CommentLocatorIdentity {
        author_label: "Alice".into(),
        text: "root".into(),
        locator_version: "test".into(),
        frame_sha256: "hash".into(),
    };
    assert!(parent_matches(&found, PKG, &parent).unwrap());
    let interloper = format!(
        r#"<node package="{PKG}" class="android.widget.TextView" resource-id="{PKG}:id/body" text="another root" bounds="[155,1060][1000,1120]"/>"#
    );
    found.snapshot = found
        .snapshot
        .replace("</hierarchy>", &format!("{interloper}</hierarchy>"));
    assert!(!parent_matches(&found, PKG, &parent).unwrap());
}
