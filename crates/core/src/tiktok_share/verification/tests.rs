use super::*;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

const PACKAGE: &str = "com.zhiliaoapp.musically";
const TRILL: &str = "com.ss.android.ugc.trill";
const CAPTION: &str =
    "Fixture caption identifies this new publication with sufficient unique detail";
const URL: &str = "https://www.tiktok.com/@fixture.account/photo/123456789";

#[test]
fn measured_global_45_4_3_expanded_photo_is_a_safe_back_surface() {
    let plan = PublishVerificationPlan::for_build(PACKAGE, "en", "45.4.3").unwrap();
    let xml = include_str!("../../../fixtures/tiktok-publish/expanded-photo-45.4.3.fixture");
    assert_eq!(classify(&tree(xml.into()), &plan), Screen::Post);
    let other = PublishVerificationPlan::for_build(PACKAGE, "en", "46.0.41").unwrap();
    assert_eq!(classify(&tree(xml.into()), &other), Screen::Unknown);
}

#[test]
fn measured_global_46_0_41_expanded_photo_is_a_safe_back_surface() {
    let plan = PublishVerificationPlan::for_build(PACKAGE, "en", "46.0.41").unwrap();
    let xml = include_str!("../../../fixtures/tiktok-publish/expanded-photo-46.0.41.fixture");
    assert_eq!(classify(&tree(xml.into()), &plan), Screen::Post);
    for changed in [
        xml.replace(":id/rki", ":id/other"),
        xml.replace(":id/rjy", ":id/other"),
        xml.replace("displayed=\"true\"", "displayed=\"false\""),
        xml.replace(PACKAGE, TRILL),
    ] {
        assert_eq!(classify(&tree(changed), &plan), Screen::Unknown);
    }
    let other = PublishVerificationPlan::for_build(PACKAGE, "en", "46.2.1").unwrap();
    assert_eq!(classify(&tree(xml.into()), &other), Screen::Unknown);
}

#[test]
fn facebook_permission_only_selects_the_measured_decline_button() {
    let plan =
        PublishVerificationPlan::for_build("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
    let xml = r#"<hierarchy><node package="com.ss.android.ugc.trill" resource-id="com.ss.android.ugc.trill:id/d2o" text="Give TikTok access to your Facebook friends list and email?" bounds="[100,900][900,1200]" displayed="true"/>
    <node package="com.ss.android.ugc.trill" class="android.widget.Button" text="Don’t allow" clickable="true" enabled="true" bounds="[172,1299][539,1424]" displayed="true"/>
    <node package="com.ss.android.ugc.trill" class="android.widget.Button" text="OK" clickable="true" enabled="true" bounds="[540,1299][905,1424]" displayed="true"/></hierarchy>"#;
    let observed = tree(xml.into());
    assert_eq!(
        decline_facebook_permission(&observed, &plan).unwrap().x,
        172.0
    );
    assert_eq!(classify(&observed, &plan), Screen::Dialog);
    assert!(decline_facebook_permission(
        &tree(xml.replace("Facebook friends list and email", "payment details")),
        &plan
    )
    .is_none());
    assert!(
        decline_facebook_permission(&tree(xml.replace("Don’t allow", "Allow")), &plan).is_none()
    );
}

#[test]
fn short_ellipsized_caption_requests_expansion_without_accepting_a_prefix() {
    let session = Session::default();
    let plan = plan();
    let identity = identity();
    let mut capture = Capture {
        session: &session,
        plan: &plan,
        caption: CAPTION,
        identity: &identity,
        started: Instant::now(),
        caption_expanded: false,
        public_link: None,
        diagnostic: VerificationDiagnostic {
            expanded_photo_error: None,
            contract_version: 1,
            package: PACKAGE.into(),
            locale: "en".into(),
            version: "46.2.1".into(),
            stage: "postProof",
            reason_code: VerificationReason::Verified,
            snapshot_generation: 1,
            caption_candidates: 0,
            time_candidates: 0,
            time_label: None,
            navigation_actions: 0,
            candidates_visited: 0,
            viewports_visited: 0,
            copy_attempts: 0,
            elapsed_ms: 0,
            navigation_matches: 0,
            navigation_enabled: 0,
            navigation_clickable: 0,
            screen_state: String::new(),
        },
    };
    let truncated = format!("{}...", CAPTION.chars().take(35).collect::<String>());
    let observed = tree(format!(
        "<hierarchy>{}{}</hierarchy>",
        node(":id/desc", &truncated, "", "[0,800][600,900]", true),
        node(":id/tv_post_time", "5m ago", "", "[0,910][600,960]", false)
    ));
    assert_eq!(
        capture.post_proof(&observed),
        Err(VerificationReason::CaptionTruncated)
    );
    let failure = anyhow::Error::new(super::super::photo_proof::MatchedPhotoCopyFailure(
        LinkCapture::Processing(ProcessingNotice {
            text: "Post is being processed".into(),
            before_sha256: "before".into(),
            after_sha256: "after".into(),
        }),
    ));
    assert_eq!(
        capture.matched_photo_failure(&failure),
        Some(VerificationReason::Processing)
    );
    assert!(capture
        .diagnostic
        .expanded_photo_error
        .as_deref()
        .unwrap()
        .contains("after"));
    let failure = anyhow::Error::new(super::super::photo_proof::MatchedPhotoCopyFailure(
        LinkCapture::CopyDidNotLand,
    ));
    assert_eq!(
        capture.matched_photo_failure(&failure),
        Some(VerificationReason::ClipboardUnchanged)
    );
    assert!(session.actions.lock().is_empty());
}

fn node(id: &str, text: &str, description: &str, bounds: &str, clickable: bool) -> String {
    format!(
        r#"<node package="{PACKAGE}" class="android.widget.Button" resource-id="{PACKAGE}{id}" text="{text}" content-desc="{description}" bounds="{bounds}" enabled="true" clickable="{clickable}" displayed="true"/>"#
    )
}

fn tree(xml: String) -> Tree {
    Tree::parse(crate::HierarchySourceSnapshot { generation: 1, xml }).unwrap()
}

fn comments_drawer_xml() -> String {
    // Sanitized semantic/geometry extract of phone 4's retained 15/09/2026
    // Trill 38.3.2 observation: the two header tabs and bottom comment input.
    format!(
        r#"<node package="{TRILL}" class="android.widget.TextView" text="Comments 0" bounds="[63,710][282,757]" enabled="true" displayed="true"/>
        <node package="{TRILL}" class="android.widget.TextView" text="Likes 0" bounds="[324,710][444,757]" enabled="true" displayed="true"/>
        <node package="{TRILL}" class="android.widget.TextView" text="Creator" bounds="[292,829][417,871]" enabled="true" displayed="true"/>
        <node package="{TRILL}" class="android.widget.EditText" resource-id="{TRILL}:id/cnd" text="Add comment..." bounds="[189,1961][721,2039]" enabled="true" clickable="true" displayed="true"/>"#
    )
}

#[test]
fn measured_comments_drawer_requires_both_headers_input_and_exact_build() {
    let plan = PublishVerificationPlan::for_build(TRILL, "en", "38.3.2").unwrap();
    let xml = format!("<hierarchy>{}</hierarchy>", comments_drawer_xml());
    assert_eq!(classify(&tree(xml.clone()), &plan), Screen::Comments);
    for changed in [
        xml.replace("Comments 0", "Comments will appear here"),
        xml.replace("Likes 0", "Liked videos"),
        xml.replace(":id/cnd", ":id/other"),
        xml.replace("android.widget.EditText", "android.widget.TextView"),
        xml.replace("displayed=\"true\"", "displayed=\"false\""),
        xml.replace(TRILL, PACKAGE),
        format!(
            "<hierarchy><node displayed=\"false\">{}</node></hierarchy>",
            comments_drawer_xml()
        ),
    ] {
        assert_eq!(classify(&tree(changed), &plan), Screen::Unknown);
    }
    let different_build = PublishVerificationPlan::for_runtime(TRILL, "en", "99.9.9").unwrap();
    assert_eq!(classify(&tree(xml), &different_build), Screen::Unknown);
    let composer = format!(
        "<hierarchy>{}{}</hierarchy>",
        comments_drawer_xml(),
        node(":id/eej", "Prepared post", "", "[42,652][1038,986]", true).replace(PACKAGE, TRILL)
    );
    assert_eq!(classify(&tree(composer), &plan), Screen::Composer);
}

#[tokio::test(start_paused = true)]
async fn comments_recovery_observes_post_before_grid_and_never_backs_from_composer_or_unknown() {
    for (target, expected, backs) in [
        ("post", Ok(()), 2),
        ("composer", Err(VerificationReason::ComposerOrUpload), 1),
        ("unknown", Err(VerificationReason::GridNotRestored), 1),
        (
            "comments",
            Err(VerificationReason::NavigationBudgetExhausted),
            5,
        ),
    ] {
        let session = Session {
            trill: true,
            page: Mutex::new("comments"),
            comments_back_target: target,
            ..Default::default()
        };
        let plan = PublishVerificationPlan::for_build(TRILL, "en", "38.3.2").unwrap();
        let identity = identity();
        let mut capture = Capture {
            session: &session,
            plan: &plan,
            caption: CAPTION,
            identity: &identity,
            started: Instant::now(),
            caption_expanded: false,
            public_link: None,
            diagnostic: VerificationDiagnostic {
                contract_version: 1,
                package: TRILL.into(),
                locale: "en".into(),
                version: "38.3.2".into(),
                stage: "restoreProfile",
                reason_code: VerificationReason::Verified,
                snapshot_generation: 0,
                caption_candidates: 0,
                time_candidates: 0,
                time_label: None,
                navigation_actions: 0,
                candidates_visited: 0,
                viewports_visited: 0,
                copy_attempts: 0,
                expanded_photo_error: None,
                elapsed_ms: 0,
                navigation_matches: 0,
                navigation_enabled: 0,
                navigation_clickable: 0,
                screen_state: String::new(),
            },
        };
        assert_eq!(
            capture.profile(true).await.map(|_| ()),
            expected,
            "{target}"
        );
        assert_eq!(
            session.actions.lock().as_slice(),
            vec!["back"; backs],
            "{target}"
        );
        assert!(session.writes.lock().is_empty());
        if target == "post" {
            assert_eq!(*session.page.lock(), "profile");
        }
    }
}

fn plan() -> PublishVerificationPlan {
    PublishVerificationPlan::for_build(PACKAGE, "en", "46.2.1").unwrap()
}

fn identity() -> SubmissionIdentity {
    SubmissionIdentity {
        prepared_at: None,
        account: "fixture.account".into(),
        submitted_at: (chrono::Utc::now() - chrono::Duration::minutes(6)).to_rfc3339(),
    }
}

struct Session {
    trill: bool,
    comments_back_target: &'static str,
    page: Mutex<&'static str>,
    page_number: Mutex<u32>,
    current_tile: Mutex<u32>,
    generation: AtomicU64,
    clipboard: Mutex<Vec<u8>>,
    writes: Mutex<Vec<Vec<u8>>>,
    actions: Mutex<Vec<String>>,
    copy_misses: u32,
    copies: AtomicU64,
    matching_page: u32,
    clipboard_fault: Option<&'static str>,
    malformed_time: bool,
    duplicate_caption: bool,
    stalled_profile: bool,
    snapshot_delay: Duration,
    multiple_matches: bool,
    other_caption_same_but_old: bool,
    rendered_caption: Option<String>,
    video_surface: bool,
    tile_opens_later: bool,
    pending_post_reads: AtomicU64,
    expand_to: Option<String>,
    caption_clickable: bool,
    expanded: Mutex<bool>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            trill: false,
            comments_back_target: "post",
            page: Mutex::new("feed"),
            page_number: Mutex::new(0),
            current_tile: Mutex::new(0),
            generation: AtomicU64::new(0),
            clipboard: Mutex::new(b"prior clipboard".to_vec()),
            writes: Mutex::new(Vec::new()),
            actions: Mutex::new(Vec::new()),
            copy_misses: 0,
            copies: AtomicU64::new(0),
            matching_page: 0,
            clipboard_fault: None,
            malformed_time: false,
            duplicate_caption: false,
            stalled_profile: false,
            snapshot_delay: Duration::ZERO,
            multiple_matches: false,
            other_caption_same_but_old: false,
            rendered_caption: None,
            video_surface: false,
            tile_opens_later: false,
            pending_post_reads: AtomicU64::new(0),
            expand_to: None,
            caption_clickable: false,
            expanded: Mutex::new(false),
        }
    }
}

impl Session {
    fn package(&self) -> &'static str {
        if self.trill {
            TRILL
        } else {
            PACKAGE
        }
    }

    fn xml(&self) -> String {
        self.xml_for(*self.page.lock())
    }

    fn xml_for(&self, page: &str) -> String {
        let caption = node(
            ":id/desc",
            if (self.matching_page == 0 || *self.page_number.lock() == self.matching_page)
                && (*self.current_tile.lock() == 0
                    || self.multiple_matches
                    || self.other_caption_same_but_old)
            {
                if *self.expanded.lock() {
                    self.expand_to
                        .as_deref()
                        .unwrap_or(self.rendered_caption.as_deref().unwrap_or(CAPTION))
                } else {
                    self.rendered_caption.as_deref().unwrap_or(CAPTION)
                }
            } else {
                "Another publication with an unrelated caption"
            },
            "",
            "[0,800][600,900]",
            self.caption_clickable,
        );
        let content = match page {
            "feed" => node(":id/profile", "", "Profile", "[800,1800][1000,1900]", true),
            "dialog" => node("", "Not now", "", "[0,1200][300,1300]", true),
            "composer" => node(":id/gx_", "Draft text", "", "[0,500][600,650]", true),
            "login" => node("", "Log in", "", "[0,500][600,650]", true),
            "comments" => comments_drawer_xml(),
            "profile" => {
                // Account controls reproduce the already retained 46.2.1 fixture;
                // the scroll container is an explicit synthetic observation.
                format!("{}{}{}<node package=\"{PACKAGE}\" enabled=\"true\" scrollable=\"true\" bounds=\"[0,500][900,1750]\">{}{}</node>", node("", "Edit", "", "[600,100][750,150]", true), node(":id/scn", "@fixture.account", "", "[300,160][600,210]", true), node("", "", "Profile menu", "[800,100][900,150]", true), node(":id/cover", "", "", "[0,550][400,1000]", true), node(":id/cover", "", "", "[0,1050][400,1500]", true))
            }
            "post" => format!(
                "{}{}{}{}{}",
                caption,
                if self.duplicate_caption {
                    node(":id/desc", CAPTION, "", "[0,910][600,960]", false)
                } else {
                    String::new()
                },
                node(
                    ":id/tv_post_time",
                    if self.other_caption_same_but_old && *self.current_tile.lock() == 1 {
                        "2h ago"
                    } else if self.malformed_time {
                        "yesterday"
                    } else {
                        "5m ago"
                    },
                    "",
                    "[0,970][600,1010]",
                    false
                ),
                node("", "", "Share video", "[800,1000][950,1100]", true),
                if self.video_surface {
                    node(
                        ":id/long_press_layout",
                        "",
                        "Video",
                        "[0,0][1080,1800]",
                        false,
                    )
                } else {
                    String::new()
                }
            ),
            "share" => format!(
                r#"<node package="{PACKAGE}" class="android.widget.Button" content-desc="Copy link" enabled="true" clickable="true" bounds="[100,1300][300,1450]">{}</node>"#,
                node("", "Copy link", "", "[100,1480][300,1540]", false)
            ),
            _ => node("", "No recognized screen", "", "[0,0][500,500]", false),
        };
        let xml = format!("<hierarchy>{content}</hierarchy>");
        if self.trill {
            xml.replace(PACKAGE, TRILL)
                .replace(":id/scn", ":id/mjf")
                .replace(":id/gx_", ":id/eej")
                .replace("text=\"Edit\"", "text=\"Edit profile\"")
                .replace(":id/desc", ":id/dmk")
                .replace(":id/tv_post_time", ":id/qrp")
        } else {
            xml
        }
    }
}

#[async_trait::async_trait]
impl UiSession for Session {
    async fn locate(
        &self,
        query: crate::ElementQuery<'_>,
    ) -> anyhow::Result<Option<crate::ElementBox>> {
        let found = self.locate_all_described(query).await?;
        anyhow::ensure!(found.len() <= 1, "fixture locate target not unique");
        Ok(found.into_iter().next())
    }

    async fn locate_all(
        &self,
        query: crate::ElementQuery<'_>,
    ) -> anyhow::Result<Vec<crate::ElementBox>> {
        self.locate_all_described(query).await
    }

    async fn locate_all_described(
        &self,
        query: crate::ElementQuery<'_>,
    ) -> anyhow::Result<Vec<crate::ElementBox>> {
        let snapshot = tree(self.xml());
        Ok(snapshot
            .matching(self.package(), query)
            .into_iter()
            .filter_map(|index| snapshot.nodes[index].rect())
            .collect())
    }

    async fn activate_element(&self, query: crate::ElementQuery<'_>) -> anyhow::Result<()> {
        let snapshot = tree(self.xml());
        let found = snapshot.matching(self.package(), query);
        let [index] = found.as_slice() else {
            anyhow::bail!("fixture target not unique");
        };
        self.tap(snapshot.nodes[*index].rect().unwrap().centre())
            .await
    }
    async fn tap(&self, point: crate::TapPoint) -> anyhow::Result<()> {
        let mut page = self.page.lock();
        let target = match *page {
            "feed" if point.x > 800.0 => "profile",
            "dialog" => "feed",
            "profile" => {
                if self.tile_opens_later {
                    self.pending_post_reads.store(1, Ordering::Relaxed);
                }
                *self.current_tile.lock() = u32::from(point.y > 1000.0);
                *self.expanded.lock() = false;
                "post"
            }
            "post"
                if self.caption_clickable
                    && point.x < 600.0
                    && (800.0..=900.0).contains(&point.y) =>
            {
                *self.expanded.lock() = true;
                self.actions.lock().push("expandCaption".into());
                "post"
            }
            "post" if point.x > 800.0 => "share",
            "share"
                if (100.0..=300.0).contains(&point.x) && (1300.0..=1450.0).contains(&point.y) =>
            {
                let attempt = self.copies.fetch_add(1, Ordering::Relaxed) + 1;
                if attempt > u64::from(self.copy_misses) {
                    *self.clipboard.lock() = URL.as_bytes().to_vec();
                }
                "post"
            }
            other => anyhow::bail!("unexpected tap on {other}"),
        };
        self.actions.lock().push(format!("tap:{target}"));
        if !self.stalled_profile || target != "profile" {
            *page = target;
        }
        Ok(())
    }
    async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
        assert_eq!(*self.page.lock(), "profile");
        *self.page_number.lock() += 1;
        self.actions.lock().push("scroll".into());
        Ok(())
    }
    async fn back(&self) -> anyhow::Result<()> {
        let mut page = self.page.lock();
        *page = match *page {
            "post" => "profile",
            "share" => "post",
            "comments" => self.comments_back_target,
            _ => anyhow::bail!("unproved Back"),
        };
        self.actions.lock().push("back".into());
        Ok(())
    }
    async fn home(&self) -> anyhow::Result<()> {
        panic!("no hardware Home")
    }
    async fn type_text(&self, _: &str) -> anyhow::Result<()> {
        panic!("no typing")
    }
    async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
        panic!("no blind tap")
    }
    async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
    fn stream_url(&self) -> Option<String> {
        None
    }
    fn supports_element_bounds(&self) -> bool {
        true
    }
    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        Ok(self.package().into())
    }
    async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> {
        tokio::time::sleep(self.snapshot_delay).await;
        let pending = self
            .pending_post_reads
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(1)
            })
            .is_ok();
        Ok(crate::HierarchySourceSnapshot {
            generation: self.generation.fetch_add(1, Ordering::Relaxed) + 1,
            xml: if pending {
                self.xml_for("profile")
            } else {
                self.xml()
            },
        })
    }
    async fn set_clipboard(&self, _: &str, bytes: &[u8]) -> anyhow::Result<()> {
        self.writes.lock().push(bytes.to_vec());
        if self.clipboard_fault == Some("restore") && bytes == b"prior clipboard" {
            anyhow::bail!("IME restore failed");
        }
        *self.clipboard.lock() = bytes.to_vec();
        if self.clipboard_fault == Some("write") && bytes.starts_with(b"riviu-") {
            anyhow::bail!("IME restore failed after write");
        }
        Ok(())
    }
    async fn get_clipboard(&self, _: usize) -> anyhow::Result<(String, Vec<u8>)> {
        if self.clipboard_fault == Some("unreadable") {
            anyhow::bail!("clipboard unavailable");
        }
        if self.clipboard_fault == Some("readback") && self.clipboard.lock().starts_with(b"riviu-")
        {
            anyhow::bail!("readback failed");
        }
        Ok(("plaintext".into(), self.clipboard.lock().clone()))
    }
}

#[tokio::test(start_paused = true)]
async fn delayed_video_viewer_copies_before_caption_drawer_and_preserves_copy_failure() {
    let session = Session {
        trill: true,
        video_surface: true,
        tile_opens_later: true,
        rendered_caption: Some(format!(
            "{}...",
            CAPTION.chars().take(35).collect::<String>()
        )),
        caption_clickable: true,
        copy_misses: u32::MAX,
        ..Default::default()
    };
    let plan = PublishVerificationPlan::for_build(TRILL, "en", "38.3.2").unwrap();
    let captured = capture_submission_link(&session, &plan, CAPTION, &identity()).await;
    assert_eq!(
        captured.diagnostic.reason_code,
        VerificationReason::ClipboardUnchanged,
        "diagnostic={:?}, actions={:?}",
        captured.diagnostic,
        session.actions.lock()
    );
    assert_eq!(captured.diagnostic.candidates_visited, 1);
    assert_eq!(session.copies.load(Ordering::Relaxed), 1);
    assert_eq!(captured.diagnostic.copy_attempts, 1);
    assert!(!session
        .actions
        .lock()
        .iter()
        .any(|action| action == "expandCaption" || action == "scroll"));
    assert_eq!(
        session
            .actions
            .lock()
            .iter()
            .filter(|action| action.as_str() == "tap:post")
            .count(),
        2
    );
}

#[tokio::test(start_paused = true)]
async fn delayed_video_with_wrong_caption_never_opens_copy_or_caption_drawer() {
    let session = Session {
        trill: true,
        video_surface: true,
        tile_opens_later: true,
        rendered_caption: Some("Different caption belongs to an older video...".into()),
        caption_clickable: true,
        ..Default::default()
    };
    let plan = PublishVerificationPlan::for_build(TRILL, "en", "38.3.2").unwrap();
    let captured = capture_submission_link(&session, &plan, CAPTION, &identity()).await;
    assert_ne!(
        captured.diagnostic.reason_code,
        VerificationReason::Verified
    );
    assert_eq!(session.copies.load(Ordering::Relaxed), 0);
    assert!(session.writes.lock().is_empty());
    assert!(!session
        .actions
        .lock()
        .iter()
        .any(|action| action == "expandCaption" || action == "tap:share"));
}

#[tokio::test(start_paused = true)]
async fn snapshot_finishing_after_deadline_never_authorizes_a_navigation_tap() {
    let session = Session {
        snapshot_delay: CAPTURE_WINDOW + Duration::from_secs(1),
        ..Default::default()
    };
    let result = capture_submission_link(&session, &plan(), CAPTION, &identity()).await;
    assert_eq!(
        result.diagnostic.reason_code,
        VerificationReason::SearchBudgetExhausted
    );
    assert!(session.actions.lock().is_empty());
    assert!(session.writes.lock().is_empty());
}

#[test]
fn measured_verification_plan_rejects_unknown_tuple_and_shares_time_aliases() {
    for version in [
        "45.4.3", "45.7.3", "46.0.41", "46.1.3", "46.2.1", "46.2.42", "46.4.3",
    ] {
        assert_eq!(
            PublishVerificationPlan::for_build(PACKAGE, "en", version)
                .unwrap()
                .contract_version(),
            1
        );
    }
    assert!(PublishVerificationPlan::for_build(PACKAGE, "en", "99.1").is_err());
    assert!(PublishVerificationPlan::for_build(PACKAGE, "vi", "46.2.1").is_err());
    let trill =
        PublishVerificationPlan::for_build("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
    assert_eq!((trill.caption_id, trill.time_id), (":id/dmk", ":id/qrp"));
}

#[test]
fn regional_english_locales_resolve_the_same_verification_contract() {
    for version in [
        "45.4.3", "45.7.3", "46.0.41", "46.1.3", "46.2.1", "46.2.42", "46.4.3",
    ] {
        for locale in ["en", "en-US", "en_US", "en-GB", " EN-us "] {
            let plan = PublishVerificationPlan::for_build(PACKAGE, locale, version).unwrap();
            assert_eq!(plan.contract_version(), 1);
        }
    }
    assert!(PublishVerificationPlan::for_build(PACKAGE, "vi-VN", "46.0.41").is_err());
}

#[tokio::test]
async fn clipboard_probe_restores_after_success_and_failed_write_or_readback() {
    for fault in [
        None,
        Some("write"),
        Some("readback"),
        Some("restore"),
        Some("unreadable"),
    ] {
        let session = Session {
            clipboard_fault: fault,
            ..Default::default()
        };
        assert_eq!(
            probe_clipboard_restore(&session).await.is_ok(),
            fault.is_none()
        );
        if fault != Some("restore") {
            assert_eq!(*session.clipboard.lock(), b"prior clipboard");
        }
        if fault == Some("unreadable") {
            assert!(session.writes.lock().is_empty());
        }
    }
}

#[tokio::test(start_paused = true)]
async fn recovery_dismisses_only_measured_dialog_and_copies_exact_post() {
    let session = Session {
        page: Mutex::new("dialog"),
        ..Default::default()
    };
    let captured = capture_submission_link(&session, &plan(), CAPTION, &identity()).await;
    assert_eq!(captured.outcome, OwnPostLink::Captured(URL.into()));
    assert_eq!(
        captured.diagnostic.reason_code,
        VerificationReason::Verified
    );
    assert_eq!(captured.diagnostic.navigation_actions, 4);
    assert_eq!(captured.diagnostic.copy_attempts, 1);
    assert_eq!(
        *session.actions.lock(),
        [
            "tap:feed",
            "tap:profile",
            "tap:post",
            "tap:share",
            "tap:post",
            "back",
            "tap:post",
            "back"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn copy_retry_uses_fresh_proof_and_sentinel_without_leaving_post() {
    let session = Session {
        copy_misses: 1,
        ..Default::default()
    };
    let captured = capture_submission_link(&session, &plan(), CAPTION, &identity()).await;
    assert_eq!(captured.outcome, OwnPostLink::Captured(URL.into()));
    assert_eq!(captured.diagnostic.copy_attempts, 2);
    let writes = session.writes.lock();
    assert_eq!(writes.len(), 2);
    assert_ne!(writes[0], writes[1]);
    let actions = session.actions.lock();
    let shares: Vec<_> = actions
        .iter()
        .enumerate()
        .filter(|(_, action)| *action == "tap:share")
        .map(|(i, _)| i)
        .collect();
    assert!(!actions[shares[0]..shares[1]]
        .iter()
        .any(|action| action == "back"));
}

#[tokio::test(start_paused = true)]
async fn two_missed_copy_attempts_remain_missing_link() {
    let session = Session {
        copy_misses: 2,
        ..Default::default()
    };
    let captured = capture_submission_link(&session, &plan(), CAPTION, &identity()).await;
    assert_eq!(
        captured.outcome,
        OwnPostLink::Sheet(LinkCapture::CopyDidNotLand)
    );
    assert_eq!(captured.diagnostic.copy_attempts, 2);
}

#[tokio::test(start_paused = true)]
async fn two_current_matching_posts_cannot_select_the_first_link() {
    let session = Session {
        multiple_matches: true,
        ..Default::default()
    };
    let captured = capture_submission_link(&session, &plan(), CAPTION, &identity()).await;
    assert_eq!(
        captured.diagnostic.reason_code,
        VerificationReason::MultipleMatchingPosts
    );
    assert!(captured.outcome.link().is_none());
    assert_eq!(captured.diagnostic.candidates_visited, 2);
    assert_eq!(captured.diagnostic.copy_attempts, 1);
}

#[tokio::test(start_paused = true)]
async fn older_same_caption_post_does_not_invalidate_single_new_match() {
    let session = Session {
        other_caption_same_but_old: true,
        ..Default::default()
    };
    let captured = capture_submission_link(&session, &plan(), CAPTION, &identity()).await;
    assert_eq!(captured.outcome, OwnPostLink::Captured(URL.into()));
    assert_eq!(captured.diagnostic.candidates_visited, 2);
}

#[tokio::test(start_paused = true)]
async fn two_captions_with_same_visible_prefix_never_verify_a_truncated_post() {
    let prefix = "A".repeat(64);
    let published = format!("{prefix} destination OLD");
    let expected = format!("{prefix} destination NEW");
    assert_ne!(published, expected);
    let truncated = format!("{prefix}…more");
    assert!(
        visible_caption_matches(&truncated, &expected),
        "legacy prefix predicate demonstrates the regression"
    );
    let session = Session {
        rendered_caption: Some(truncated),
        ..Default::default()
    };
    let result = capture_submission_link(&session, &plan(), &expected, &identity()).await;
    assert_eq!(
        result.diagnostic.reason_code,
        VerificationReason::CaptionTruncated
    );
    assert!(result.outcome.link().is_none());
    assert_eq!(result.diagnostic.copy_attempts, 0);
    assert!(session.writes.lock().is_empty());
    assert!(!session
        .actions
        .lock()
        .iter()
        .any(|action| action == "tap:share"));
}

#[tokio::test(start_paused = true)]
async fn complete_caption_proof_normalizes_whitespace_without_ignoring_changed_suffix() {
    let expected = format!("{} destination NEW", "A".repeat(64));
    let visible = format!("\u{200e}{}\u{200f}", expected.replace(' ', "  \n "));
    let matching = Session {
        rendered_caption: Some(visible),
        ..Default::default()
    };
    assert_eq!(
        capture_submission_link(&matching, &plan(), &expected, &identity())
            .await
            .outcome,
        OwnPostLink::Captured(URL.into())
    );
    let other = Session {
        rendered_caption: Some(expected.replace("NEW", "OLD")),
        ..Default::default()
    };
    let result = capture_submission_link(&other, &plan(), &expected, &identity()).await;
    assert_eq!(
        result.diagnostic.reason_code,
        VerificationReason::CaptionMismatch
    );
    assert!(result.outcome.link().is_none());
    assert!(other.writes.lock().is_empty());
}

#[tokio::test(start_paused = true)]
async fn caption_expansion_requires_actual_full_text_change_before_copy() {
    let expected = format!("{} destination NEW", "A".repeat(64));
    let prefix = format!("{}…more", "A".repeat(64));
    for (expanded, reason) in [
        (Some(expected.clone()), VerificationReason::Verified),
        (None, VerificationReason::CaptionTruncated),
        (
            Some(expected.replace("NEW", "OLD")),
            VerificationReason::CaptionMismatch,
        ),
    ] {
        let session = Session {
            rendered_caption: Some(prefix.clone()),
            caption_clickable: true,
            expand_to: expanded,
            ..Default::default()
        };
        let result = capture_submission_link(&session, &plan(), &expected, &identity()).await;
        assert_eq!(result.diagnostic.reason_code, reason);
        assert_eq!(
            result.outcome.link().is_some(),
            reason == VerificationReason::Verified
        );
        // At most once for each opened candidate, including any paginated visit.
        assert!(
            session
                .actions
                .lock()
                .iter()
                .filter(|action| *action == "expandCaption")
                .count()
                <= result.diagnostic.candidates_visited as usize
        );
        if reason != VerificationReason::Verified {
            assert!(session.writes.lock().is_empty());
        }
    }
}

#[tokio::test(start_paused = true)]
async fn winning_diagnostic_keeps_verified_post_time_after_checking_old_same_caption() {
    let session = Session {
        other_caption_same_but_old: true,
        ..Default::default()
    };
    let result = capture_submission_link(&session, &plan(), CAPTION, &identity()).await;
    assert_eq!(result.outcome, OwnPostLink::Captured(URL.into()));
    assert_eq!(result.diagnostic.time_label.as_deref(), Some("5m ago"));
    assert_eq!(result.diagnostic.candidates_visited, 2);
    assert!(result.diagnostic.snapshot_generation < session.generation.load(Ordering::Relaxed));
}

#[tokio::test(start_paused = true)]
async fn composer_and_login_never_receive_recovery_navigation() {
    for (page, reason) in [
        ("composer", VerificationReason::ComposerOrUpload),
        ("login", VerificationReason::LoginRequired),
    ] {
        let session = Session {
            page: Mutex::new(page),
            ..Default::default()
        };
        let captured = capture_submission_link(&session, &plan(), CAPTION, &identity()).await;
        assert_eq!(captured.diagnostic.reason_code, reason);
        assert!(session.actions.lock().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn unknown_or_stalled_profile_stops_after_bounded_fresh_attempts() {
    for page in ["unknown", "feed"] {
        let session = Session {
            page: Mutex::new(page),
            stalled_profile: true,
            ..Default::default()
        };
        let start = Instant::now();
        let result = capture_submission_link(&session, &plan(), CAPTION, &identity()).await;
        assert_eq!(result.outcome, OwnPostLink::ProfileTabMissing);
        assert!(start.elapsed() <= RECOVERY_WINDOW + POLL);
        assert!(session.actions.lock().len() <= MAX_RECOVERY_ACTIONS as usize);
    }
}

#[tokio::test(start_paused = true)]
async fn matching_post_on_second_viewport_requires_grid_restoration() {
    let session = Session {
        matching_page: 1,
        ..Default::default()
    };
    let captured = capture_submission_link(&session, &plan(), CAPTION, &identity()).await;
    assert_eq!(captured.outcome, OwnPostLink::Captured(URL.into()));
    assert_eq!(captured.diagnostic.candidates_visited, 4);
    assert_eq!(captured.diagnostic.viewports_visited, 2);
    assert_eq!(
        session
            .actions
            .lock()
            .iter()
            .filter(|action| *action == "back")
            .count(),
        4
    );
}

#[tokio::test(start_paused = true)]
async fn ambiguous_caption_or_unmeasured_time_never_reaches_share() {
    for (duplicate_caption, malformed_time, reason) in [
        (true, false, VerificationReason::CaptionAmbiguous),
        (false, true, VerificationReason::TimestampUnmeasured),
    ] {
        let session = Session {
            duplicate_caption,
            malformed_time,
            ..Default::default()
        };
        let captured = capture_submission_link(&session, &plan(), CAPTION, &identity()).await;
        assert_eq!(captured.diagnostic.reason_code, reason);
        assert_eq!(captured.diagnostic.copy_attempts, 0);
        assert!(!session
            .actions
            .lock()
            .iter()
            .any(|action| action == "tap:share"));
        assert!(captured.diagnostic.viewports_visited <= 3);
        assert!(captured.diagnostic.candidates_visited <= 12);
    }
}

#[test]
fn copy_caption_uses_its_parent_and_rejects_disabled_or_ambiguous_controls() {
    let fixture = Session {
        page: Mutex::new("share"),
        ..Default::default()
    }
    .xml();
    let copy = tree(fixture.clone())
        .copy_control(PACKAGE)
        .unwrap()
        .unwrap();
    assert_eq!(copy.centre().y, 1375.0);
    assert!(
        tree(fixture.replace("enabled=\"true\"", "enabled=\"false\""))
            .copy_control(PACKAGE)
            .unwrap()
            .is_none()
    );
    let second = node("", "Copy link", "", "[400,1300][600,1450]", true);
    assert!(
        tree(fixture.replace("</hierarchy>", &format!("{second}</hierarchy>")))
            .copy_control(PACKAGE)
            .is_err()
    );
}

#[test]
fn measured_draft_cover_is_excluded_and_without_ancestor_grid_does_not_scroll() {
    let plan = plan();
    let fixture =
        include_str!("../../../fixtures/tiktok-publish/own-profile-musically-46.2.1-en.xml");
    let original = tree(fixture.to_owned());
    assert!(original.grid_scroll(&plan).is_none());
    let tile = original.grid(&plan)[0].clone();
    let draft = node(
        ":id/zq_",
        "Drafts: 1",
        "",
        &format!(
            "[{},{}][{},{}]",
            tile.x,
            tile.y,
            tile.x + tile.width,
            tile.y + 20.0
        ),
        false,
    );
    let with_draft = tree(fixture.replace("</hierarchy>", &format!("{draft}</hierarchy>")));
    assert_eq!(with_draft.grid(&plan).len() + 1, original.grid(&plan).len());
}

#[test]
fn profile_covers_and_scroll_never_touch_observed_bottom_navigation() {
    // The overlapping 1942..2076 cover and tabs starting at 1929 reproduce the
    // machine 1 observation; no screen-size constant is added to the runtime.
    let observed = tree(format!("<hierarchy><node package=\"{PACKAGE}\" enabled=\"true\" scrollable=\"true\" bounds=\"[0,1200][1080,2076]\">{}{}</node>{}</hierarchy>",
        node(":id/cover", "", "", "[0,1462][358,1939]", false),
        node(":id/cover", "", "", "[0,1942][358,2076]", false),
        node(":id/profile", "", "Profile", "[864,1929][1080,2076]", true)));
    let covers = observed.grid(&plan());
    assert_eq!(covers.len(), 1);
    assert!(covers[0].centre().y < 1929.0);
    assert_eq!(covers[0].y + covers[0].height, 1929.0);
    assert!(observed.grid_scroll(&plan()).is_none());
}

#[test]
fn removed_post_banner_prevents_share_capture() {
    let observed = tree(format!(
        "<hierarchy>{}</hierarchy>",
        node(
            ":id/ssu",
            "Removed for violating Community Guidelines",
            "",
            "[0,1000][1000,1100]",
            false
        )
    ));
    assert!(observed.publication_removed(PACKAGE));
    assert!(!observed.publication_removed("another.package"));
    let hidden = tree(format!(
        "<hierarchy>{}</hierarchy>",
        node(
            ":id/ssu",
            "Removed for violating Community Guidelines",
            "",
            "[0,1000][1000,1100]",
            false
        )
        .replace("displayed=\"true\"", "displayed=\"false\"")
    ));
    assert!(!hidden.publication_removed(PACKAGE));
}

#[test]
fn snapshot_parser_rejects_invalid_unbounded_or_stale_identity() {
    assert!(Tree::parse(crate::HierarchySourceSnapshot {
        generation: 0,
        xml: "<hierarchy/>".into()
    })
    .is_err());
    assert!(Tree::parse(crate::HierarchySourceSnapshot {
        generation: 1,
        xml: "<hierarchy>".into()
    })
    .is_err());
    assert!(Tree::parse(crate::HierarchySourceSnapshot {
        generation: 1,
        xml: "<!DOCTYPE x><hierarchy/>".into()
    })
    .is_err());
}

#[tokio::test]
async fn missing_or_invalid_identity_stops_before_any_read_or_action() {
    for account in ["", " ", "bad handle", "fixture."] {
        let session = Session::default();
        let identity = SubmissionIdentity {
            prepared_at: None,
            account: account.into(),
            submitted_at: chrono::Utc::now().to_rfc3339(),
        };
        let result = capture_submission_link(&session, &plan(), CAPTION, &identity).await;
        assert_eq!(
            result.diagnostic.reason_code,
            VerificationReason::IdentityMissing
        );
        assert_eq!(session.generation.load(Ordering::Relaxed), 0);
        assert!(session.actions.lock().is_empty());
    }
}

#[test]
fn a_changed_post_snapshot_cannot_combine_prior_caption_with_new_time() {
    let plan = plan();
    let session = Session {
        page: Mutex::new("post"),
        ..Default::default()
    };
    let identity = identity();
    let mut capture = Capture {
        session: &session,
        plan: &plan,
        caption: CAPTION,
        identity: &identity,
        started: Instant::now(),
        caption_expanded: false,
        public_link: None,
        diagnostic: VerificationDiagnostic {
            expanded_photo_error: None,
            navigation_matches: 0,
            navigation_enabled: 0,
            navigation_clickable: 0,
            screen_state: String::new(),
            contract_version: 1,
            package: PACKAGE.into(),
            locale: "en".into(),
            version: "46.2.1".into(),
            stage: "postProof",
            reason_code: VerificationReason::IdentityMissing,
            snapshot_generation: 0,
            caption_candidates: 0,
            time_candidates: 0,
            time_label: None,
            navigation_actions: 0,
            candidates_visited: 0,
            viewports_visited: 0,
            copy_attempts: 0,
            elapsed_ms: 0,
        },
    };
    let snapshot = session.xml();
    assert!(capture.post_proof(&tree(snapshot.clone())).is_ok());
    assert_eq!(
        capture.post_proof(&tree(snapshot.replace(CAPTION, "Another post"))),
        Err(VerificationReason::CaptionMismatch)
    );
    assert_eq!(
        capture.post_proof(&tree(snapshot.replace("5m ago", "2h ago"))),
        Err(VerificationReason::SubmissionTooOld)
    );
    assert_eq!(
        capture.post_proof(&tree(snapshot.replace(":id/tv_post_time", ":id/not-time"))),
        Err(VerificationReason::TimestampMissing)
    );
}
