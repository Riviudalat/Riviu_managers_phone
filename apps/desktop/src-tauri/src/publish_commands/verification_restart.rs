//! The submitted publication keeps its identity while TikTok restarts for Copy.
use riviu_core::{db::Database, DeviceControlPlane, PublishAssignmentRecord, UiSessionContext};

fn measured_empty_recipient_picker(tree: &riviu_core::ui_automation::tree::Tree) -> bool {
    const PACKAGE: &str = "com.samsung.android.messaging";
    let one = |id: &str| {
        let mut matches = tree.nodes.iter().enumerate().filter(|(index, node)| {
            node.visible(PACKAGE)
                && node.visibility() == Some(true)
                && tree.ancestors_visible(*index)
                && node.attr("resource-id") == id
        });
        let only = matches.next()?;
        matches.next().is_none().then_some(only.0)
    };
    let Some(root) = one("com.samsung.android.messaging:id/picker_activity_main") else {
        return false;
    };
    let Some(search) = one("com.samsung.android.messaging:id/search_src_text") else {
        return false;
    };
    let Some(empty) = one("com.samsung.android.messaging:id/empty_layout") else {
        return false;
    };
    let Some(title) = one("com.samsung.android.messaging:id/empty_title") else {
        return false;
    };
    if !tree.inside(search, root)
        || !tree.inside(empty, root)
        || !tree.inside(title, empty)
        || tree.nodes[search].attr("class") != "android.widget.EditText"
        || tree.nodes[search].attr("hint") != "Search Contacts or enter number"
        || tree.nodes[search].attr("text") != "Search Contacts or enter number"
        || tree.nodes[search].attr("showing-hint") != "true"
        || tree.nodes[search].attr("focused") != "true"
        || tree.nodes[title].attr("text") != "No contacts"
    {
        return false;
    }
    let tabs: Vec<_> = tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            node.visible(PACKAGE)
                && node.visibility() == Some(true)
                && tree.inside(*index, root)
                && node.attr("resource-id") == "com.samsung.android.messaging:id/title"
        })
        .map(|(_, node)| node)
        .collect();
    if tabs.len() != 2
        || !tabs.iter().any(|node| {
            node.attr("text") == "Contacts"
                && node.attr("content-desc") == "Contacts, Tab 2 of 2"
                && node.attr("selected") == "true"
        })
        || !tabs.iter().any(|node| {
            node.attr("text") == "Conversations"
                && node.attr("content-desc") == "Conversations, Tab 1 of 2"
                && node.attr("selected") == "false"
        })
    {
        return false;
    }
    tree.nodes.iter().enumerate().all(|(index, node)| {
        !node.visible(PACKAGE)
            || !tree.inside(index, root)
            || matches!(
                (node.attr("text"), node.attr("content-desc")),
                ("", "")
                    | ("Search Contacts or enter number", "")
                    | ("No contacts", "")
                    | ("Contacts", "Contacts, Tab 2 of 2")
                    | ("Conversations", "Conversations, Tab 1 of 2")
            )
    })
}

fn legacy_empty_recipient_picker(tree: &riviu_core::ui_automation::tree::Tree) -> bool {
    const PACKAGE: &str = "com.samsung.android.messaging";
    ["Select recipients", "No contacts"].iter().all(|text| {
        tree.matching(
            PACKAGE,
            riviu_core::ElementQuery::Text {
                value: text,
                exact: true,
            },
        )
        .len()
            == 1
    })
}

pub(super) fn requested(assignment: &PublishAssignmentRecord, package: &str) -> bool {
    if !matches!(
        package,
        "com.zhiliaoapp.musically" | "com.ss.android.ugc.trill"
    ) {
        return false;
    }
    // Older publication intents did not persist a package. They can still be
    // observed and matched by account/caption/time, but cannot authorize a
    // package restart. Missing provenance is not evidence that the app changed.
    let intent: serde_json::Value = assignment
        .effect_intent
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    if intent["package"].as_str().is_none_or(str::is_empty) {
        return false;
    }
    let evidence: serde_json::Value = assignment
        .evidence_json
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    let post = evidence.get("post").unwrap_or(&evidence);
    // Unknown Post outcomes and historical receipts stay observational. Only
    // an actual submitted/posted receipt can authorize interrupting this app.
    matches!(post["state"].as_str(), Some("submitted" | "posted"))
        && post["publicationVerified"] != true
        && post["postUrl"].as_str().is_none_or(str::is_empty)
}

pub(super) fn processing_restart_requested(
    assignment: &PublishAssignmentRecord,
    package: &str,
    locale: &str,
    version: &str,
) -> bool {
    if (package, locale, version) != ("com.zhiliaoapp.musically", "en", "45.7.3") {
        return false;
    }
    let Some(intent) = assignment.effect_intent.as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()) else { return false };
    let Some(evidence) = assignment.evidence_json.as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()) else { return false };
    let post = evidence.get("post").unwrap_or(&evidence);
    matches!(intent["effectIntent"].as_str(), Some("post" | "post_carousel"))
        && intent["package"] == package
        && intent["expectedAccount"].as_str().is_some_and(|account| !account.trim().is_empty())
        && intent["submittedAt"].as_str().is_some_and(|at| chrono::DateTime::parse_from_rfc3339(at).is_ok())
        && matches!(assignment.state, riviu_core::PublishCampaignState::Verifying | riviu_core::PublishCampaignState::Uncertain)
        && post["publicationVerified"] != true
        && post["postUrl"].as_str().is_none_or(str::is_empty)
        && evidence["verificationStatus"]["reasonCode"] == "tiktokProcessing"
        && evidence["verificationDiagnostic"]["package"] == package
        && evidence["verificationDiagnostic"]["locale"] == locale
        && evidence["verificationDiagnostic"]["version"] == version
        && evidence["verificationDiagnostic"]["copyAttempts"].as_u64().is_some_and(|count| count > 0)
        && evidence["verificationDiagnostic"]["expandedPhotoError"].as_str()
            .is_some_and(|error| error.contains("ProcessingNotice") && error.contains("Post is being processed"))
        && evidence["accountDiagnostic"]["state"] == "proved"
        && evidence["accountDiagnostic"]["package"] == package
        && evidence["accountDiagnostic"]["expectedAccount"].as_str().is_some_and(|account|
            account.trim_start_matches('@').eq_ignore_ascii_case(
                intent["expectedAccount"].as_str().unwrap_or_default().trim_start_matches('@')))
        && evidence["accountDiagnostic"]["observedAccount"].as_str().is_some_and(|account|
            account.trim_start_matches('@').eq_ignore_ascii_case(
                intent["expectedAccount"].as_str().unwrap_or_default().trim_start_matches('@')))
}

/// The measured expanded-photo surface exposes the complete caption. Home and
/// partially captioned feed cards are not evidence that an upload has settled.
pub(super) async fn admit_processing_restart(
    session: &dyn riviu_core::UiSession,
    package: &str,
    caption: &str,
) -> anyhow::Result<serde_json::Value> {
    anyhow::ensure!(!caption.trim().is_empty(), "processing restart caption missing");
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "processing restart session epoch missing");
    let mut previous = None;
    for _ in 0..2 {
        anyhow::ensure!(session.gui_session_epoch() == epoch
            && session.active_app_bundle().await? == package,
            "processing restart foreground changed");
        let tree = riviu_core::ui_automation::tree::Tree::parse(
            session.hierarchy_source_snapshot().await?)?;
        anyhow::ensure!(global_warm_surface(&tree) == Some("expandedPhotoViewer"),
            "processing restart requires settled measured photo viewer");
        let captions = tree.matching(package,
            riviu_core::ElementQuery::ResourceIdSuffix(":id/rey"));
        let [index] = captions.as_slice() else {
            anyhow::bail!("processing restart caption not unique");
        };
        let normalize = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        anyhow::ensure!(normalize(tree.nodes[*index].attr("text")) == normalize(caption),
            "processing restart caption differs from immutable bundle");
        if let Some(before) = previous {
            anyhow::ensure!(tree.generation > before,
                "processing restart viewer snapshot stale");
        }
        previous = Some(tree.generation);
    }
    anyhow::ensure!(session.gui_session_epoch() == epoch
        && session.active_app_bundle().await? == package,
        "processing restart foreground changed");
    Ok(serde_json::json!({"state":"measuredProcessingViewer","package":package,
        "snapshotGeneration":previous}))
}

fn authorize_current(
    db: &Database,
    assignment: &PublishAssignmentRecord,
    observer: Option<&riviu_core::db::PendingPublishVerification>,
) -> anyhow::Result<()> {
    if let Some(observer) = observer {
        anyhow::ensure!(
            db.publish_verification_is_current(observer)?,
            "Lượt xác minh đã thay đổi hoặc đã dừng"
        );
    } else {
        let current = db
            .get_publish_assignment_detail(&assignment.campaign_id, &assignment.id)?
            .and_then(|detail| {
                detail
                    .assignments
                    .into_iter()
                    .find(|row| row.id == assignment.id)
            });
        anyhow::ensure!(
            !db.publish_assignment_excluded(&assignment.id)?
                && !db.publish_operation_stopped(&assignment.campaign_id)?
                && current
                    .as_ref()
                    .is_some_and(|row| row.state == assignment.state
                        && row.effect_intent == assignment.effect_intent
                        && row.evidence_json == assignment.evidence_json),
            "Lượt xác minh đã thay đổi hoặc đã dừng"
        );
    }
    Ok(())
}

pub(super) fn shared_debt(
    db: &Database,
    assignment: &PublishAssignmentRecord,
) -> anyhow::Result<bool> {
    Ok(db
        .publish_device_guard(&assignment.udid)?
        .blocking
        .iter()
        .any(|hold| hold.assignment_id != assignment.id))
}

/// Destructive restart retains the original device-wide upload exclusion.
pub(super) fn authorize_restart(
    db: &Database,
    assignment: &PublishAssignmentRecord,
    observer: Option<&riviu_core::db::PendingPublishVerification>,
) -> anyhow::Result<()> {
    authorize_current(db, assignment, observer)?;
    let guard = db.publish_device_guard(&assignment.udid)?;
    anyhow::ensure!(
        guard
            .blocking
            .iter()
            .all(|hold| hold.assignment_id == assignment.id),
        "Máy còn bài khác chưa xác minh; chưa khởi động lại TikTok"
    );
    Ok(())
}

/// Only used by a warm verifier after measured surface admission. The caller
/// holds the existing per-device verify permit and exclusive control lease.
pub(super) fn authorize(
    db: &Database,
    assignment: &PublishAssignmentRecord,
    observer: Option<&riviu_core::db::PendingPublishVerification>,
) -> anyhow::Result<()> {
    authorize_current(db, assignment, observer)?;
    let identity: serde_json::Value = serde_json::from_str(
        assignment
            .effect_intent
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("submission identity missing"))?,
    )?;
    let package = identity["package"].as_str().unwrap_or_default();
    let account = identity["expectedAccount"].as_str().unwrap_or_default();
    anyhow::ensure!(
        !package.is_empty() && !account.trim().is_empty(),
        "submission identity missing"
    );
    let mut holds = db.publish_device_guard(&assignment.udid)?.blocking;
    // Also check this receipt even if its earlier stop-release has parked it.
    holds.push(riviu_core::db::PublishDeviceHold {
        assignment_id: assignment.id.clone(),
        campaign_id: assignment.campaign_id.clone(),
        updated_at: String::new(),
        reason: String::new(),
    });
    for hold in holds {
        anyhow::ensure!(
            !db.has_active_publish_pipeline(&hold.campaign_id)?,
            "active publish pipeline; keep app unchanged"
        );
        let row = db
            .get_publish_assignment_detail(&hold.campaign_id, &hold.assignment_id)?
            .and_then(|detail| {
                detail
                    .assignments
                    .into_iter()
                    .find(|row| row.id == hold.assignment_id)
            })
            .ok_or_else(|| anyhow::anyhow!("publication missing"))?;
        let intent: serde_json::Value = serde_json::from_str(
            row.effect_intent
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("peer submission identity missing"))?,
        )?;
        let evidence: serde_json::Value = serde_json::from_str(
            row.evidence_json
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("peer submission receipt missing"))?,
        )?;
        anyhow::ensure!(
            matches!(
                row.state,
                riviu_core::PublishCampaignState::Verifying
                    | riviu_core::PublishCampaignState::Uncertain
            ) && row.udid == assignment.udid
                && row
                    .dispatch
                    .as_ref()
                    .is_none_or(|job| job["state"] == "finished")
                && matches!(
                    intent["effectIntent"].as_str(),
                    Some("post" | "post_carousel")
                )
                && intent["package"].as_str() == Some(package)
                && intent["expectedAccount"].as_str().is_some_and(|a| a
                    .trim_start_matches('@')
                    .eq_ignore_ascii_case(account.trim_start_matches('@')))
                && intent["submittedAt"]
                    .as_str()
                    .is_some_and(|t| chrono::DateTime::parse_from_rfc3339(t).is_ok())
                && matches!(
                    evidence["post"]["state"].as_str(),
                    Some("submitted" | "posted")
                ),
            "other publication may still be composing or uploading; keep app unchanged"
        );
    }
    Ok(())
}

fn warm_surface(
    tree: &riviu_core::ui_automation::tree::Tree,
    account: &str,
) -> Option<&'static str> {
    use riviu_core::ElementQuery as Q;
    const PACKAGE: &str = "com.ss.android.ugc.trill";
    let labels = riviu_core::tiktok_labels::controls_for_runtime(PACKAGE, "en", "38.3.2")?;
    for control in [
        riviu_core::tiktok_labels::TikTokControl::ComposerCaption,
        riviu_core::tiktok_labels::TikTokControl::PostButton,
        riviu_core::tiktok_labels::TikTokControl::ComposerShutter,
    ] {
        if labels
            .label(control)
            .is_some_and(|label| !tree.matching(PACKAGE, label.to_query()).is_empty())
        {
            return None;
        }
    }
    for (i, node) in tree.nodes.iter().enumerate() {
        if node.visibility() == Some(false) || !tree.ancestors_visible(i) {
            continue;
        }
        if !node.attr("package").is_empty() && node.attr("package") != PACKAGE {
            return None;
        }
        if matches!(
            node.attr("class"),
            "android.widget.EditText" | "android.widget.ProgressBar"
        ) {
            return None;
        }
        let text = format!("{} {}", node.attr("text"), node.attr("content-desc")).to_lowercase();
        if [
            "uploading",
            "processing",
            "posting",
            "đang tải",
            "đang đăng",
            "đang xử lý",
        ]
        .iter()
        .any(|s| text.contains(s))
        {
            return None;
        }
    }
    let one = |query| {
        let indices = tree.matching(PACKAGE, query);
        let [index] = indices.as_slice() else {
            return None;
        };
        let node = &tree.nodes[*index];
        (node.visibility() == Some(true) && node.attr("enabled") == "true").then_some(node)
    };
    let back = one(Q::ResourceIdSuffix(":id/aur"));
    let caption = one(Q::ResourceIdSuffix(":id/dmk"));
    let time = one(Q::ResourceIdSuffix(":id/qrp"));
    let privacy = one(Q::ResourceIdSuffix(":id/ma9"));
    let shares = tree.matching(
        PACKAGE,
        Q::Description {
            value: "Share video.",
            exact: false,
        },
    );
    if back.is_some_and(|n| n.attr("content-desc") == "Back" && n.attr("clickable") == "true")
        && caption.is_some_and(|n| {
            n.attr("class") == "android.widget.TextView" && !n.attr("text").trim().is_empty()
        })
        && time.is_some_and(|n| !n.attr("text").trim().is_empty())
        && privacy
            .is_some_and(|n| n.attr("text") == "Privacy settings" && n.attr("clickable") == "true")
        && matches!(shares.as_slice(), [i] if tree.nodes[*i].visibility() == Some(true) && tree.nodes[*i].rect().is_some_and(|r| r.enabled && r.clickable))
    {
        return Some("ownPostViewer");
    }
    // Measured 28/09: after a second publication, Trill can remain on the
    // Home feed's own-photo card instead of the dedicated post viewer.
    // This only admits read-only navigation; it never proves publication identity.
    let home = one(Q::ResourceIdSuffix(":id/jmk"));
    let profile_tab = one(Q::ResourceIdSuffix(":id/jmm"));
    let feed_caption = one(Q::ResourceIdSuffix(":id/desc"));
    let feed_time = one(Q::ResourceIdSuffix(":id/sby"));
    if home.is_some_and(|n| {
        n.attr("content-desc") == "Home"
            && n.attr("selected") == "true"
            && n.attr("clickable") == "true"
    }) && profile_tab
        .is_some_and(|n| n.attr("content-desc") == "Profile" && n.attr("clickable") == "true")
        && feed_caption.is_some_and(|n| !n.attr("text").trim().is_empty())
        && feed_time.is_some_and(|n| !n.attr("text").trim().is_empty())
        && matches!(shares.as_slice(), [i] if tree.nodes[*i].visibility() == Some(true)
            && tree.nodes[*i].rect().is_some_and(|r| r.enabled && r.clickable))
    {
        return Some("postFeed");
    }
    let for_you = tree.matching(
        PACKAGE,
        Q::Text {
            value: "For You",
            exact: true,
        },
    );
    let video = one(Q::ResourceIdSuffix(":id/long_press_layout"));
    if home.is_some_and(|n| {
        n.attr("content-desc") == "Home"
            && n.attr("selected") == "true"
            && n.attr("clickable") == "true"
    }) && profile_tab
        .is_some_and(|n| n.attr("content-desc") == "Profile" && n.attr("clickable") == "true")
        && video.is_some_and(|n| n.attr("content-desc") == "Video")
        && matches!(for_you.as_slice(), [i] if tree.nodes[*i].visibility() == Some(true))
    {
        return Some("homeFeed");
    }
    let profile = one(Q::Text {
        value: "Edit profile",
        exact: true,
    });
    let username = one(Q::ResourceIdSuffix(":id/mjf"));
    if profile.is_some()
        && username.is_some_and(|n| {
            n.attr("text")
                .trim()
                .trim_start_matches('@')
                .eq_ignore_ascii_case(account.trim_start_matches('@'))
        })
    {
        return Some("ownProfile");
    }
    None
}

/// Two production captures (final-global-observation/global-expanded-second, 29/09)
/// show this expanded photo with an idle comment hint. Return only that input's index;
/// it is the sole EditText exemption and never a publication identity or tap authority.
fn global_expanded_photo_input(tree: &riviu_core::ui_automation::tree::Tree) -> Option<usize> {
    const PACKAGE: &str = "com.zhiliaoapp.musically";
    let one = |suffix, class| {
        let indices = tree.matching(PACKAGE, riviu_core::ElementQuery::ResourceIdSuffix(suffix));
        let [index] = indices.as_slice() else { return None };
        let node = &tree.nodes[*index];
        (node.visibility() == Some(true) && node.attr("enabled") == "true"
            && node.attr("class") == class).then_some(*index)
    };
    let pager = one(":id/view_pager", "X.18gp")?;
    let header = one(":id/s94", "android.view.ViewGroup")?;
    let back = one(":id/bjb", "android.widget.ImageView")?;
    let author = one(":id/jf_", "android.widget.Button")?;
    let photos = one(":id/q_g", "X.18gp")?;
    let image = one(":id/vgh", "android.widget.ImageView")?;
    let caption = one(":id/rey", "android.widget.TextView")?;
    let footer = one(":id/c8n", "android.widget.LinearLayout")?;
    let input = one(":id/re7", "android.widget.EditText")?;
    let share = one(":id/red", "android.widget.ImageView")?;
    let comments = one(":id/rdz", "android.widget.ImageView")?;
    if ![header, photos, caption, footer].iter().all(|index| tree.inside(*index, pager))
        || ![back, author].iter().all(|index| tree.inside(*index, header))
        || !tree.inside(image, photos)
        || ![input, share, comments].iter().all(|index| tree.inside(*index, footer))
    {
        return None;
    }
    let field = &tree.nodes[input];
    (tree.nodes[back].attr("clickable") == "true"
        && tree.nodes[author].attr("clickable") == "true"
        && !tree.nodes[author].attr("text").trim().is_empty()
        && tree.nodes[caption].attr("clickable") == "true"
        && !tree.nodes[caption].attr("text").trim().is_empty()
        && tree.nodes[share].attr("content-desc") == "Share"
        && tree.nodes[share].attr("clickable") == "true"
        && tree.nodes[comments].attr("content-desc") == "Comments"
        && tree.nodes[comments].attr("clickable") == "true"
        && field.attr("focused") == "false" && field.attr("a11y-focused") == "false"
        && field.attr("password") == "false" && field.attr("showing-hint") == "true"
        && field.attr("hint") == "Add comment..." && field.attr("text") == "Add comment..."
        && field.attr("clickable") == "true")
        .then_some(input)
}

/// Global 45.7.3/en viewer and Home measured in saved phone captures on 29/09.
/// Admission only: account, complete caption, submission time and canonical URL still
/// belong to the existing verifier. No Global profile or overlay is admitted here.
fn global_warm_surface(tree: &riviu_core::ui_automation::tree::Tree) -> Option<&'static str> {
    use riviu_core::ElementQuery as Q;
    const PACKAGE: &str = "com.zhiliaoapp.musically";
    let labels = riviu_core::tiktok_labels::controls_for_runtime(PACKAGE, "en", "45.7.3")?;
    let expanded_input = global_expanded_photo_input(tree);
    for control in [
        riviu_core::tiktok_labels::TikTokControl::ComposerCaption,
        riviu_core::tiktok_labels::TikTokControl::PostButton,
        riviu_core::tiktok_labels::TikTokControl::ComposerShutter,
    ] {
        if labels.label(control).is_some_and(|label| !tree.matching(PACKAGE, label.to_query()).is_empty()) {
            return None;
        }
    }
    for (index, node) in tree.nodes.iter().enumerate() {
        // The XML hierarchy wrapper is not a UI node. All actual visible nodes in
        // both measured captures belong to TikTok; no Samsung edge exception exists.
        if index == 0 && node.parent.is_none() && node.attr("class") == "hierarchy"
            && node.attr("package").is_empty() && node.attribute("resource-id").is_none()
        {
            continue;
        }
        if node.visibility() == Some(false) || !tree.ancestors_visible(index) {
            continue;
        }
        if node.visibility() != Some(true) || node.attr("package") != PACKAGE
            || node.attr("class") == "android.widget.ProgressBar"
            || (node.attr("class") == "android.widget.EditText" && expanded_input != Some(index))
        {
            return None;
        }
        let text = format!("{} {}", node.attr("text"), node.attr("content-desc")).to_lowercase();
        if ["uploading", "processing", "posting", "đang tải", "đang đăng", "đang xử lý"]
            .iter().any(|token| text.contains(token))
        {
            return None;
        }
    }
    let one = |suffix| {
        let indices = tree.matching(PACKAGE, Q::ResourceIdSuffix(suffix));
        let [index] = indices.as_slice() else { return None };
        let node = &tree.nodes[*index];
        (node.visibility() == Some(true) && node.attr("enabled") == "true").then_some(node)
    };
    if expanded_input.is_some() {
        return Some("expandedPhotoViewer");
    }
    // Measured global-ime-start: Home is only an entry for profile navigation,
    // never evidence that the visible feed card belongs to this publication.
    let home = one(":id/nr_");
    let profile = one(":id/nrb");
    let video = one(":id/long_press_layout");
    let for_you = tree.matching(PACKAGE, Q::Text { value: "For You", exact: true });
    if home.is_some_and(|node| node.attr("class") == "android.widget.FrameLayout"
        && node.attr("content-desc") == "Home" && node.attr("selected") == "true"
        && node.attr("clickable") == "true")
        && profile.is_some_and(|node| node.attr("class") == "android.widget.FrameLayout"
            && node.attr("content-desc") == "Profile" && node.attr("selected") == "false"
            && node.attr("clickable") == "true")
        && video.is_some_and(|node| node.attr("class") == "android.view.View"
            && node.attr("content-desc") == "Video" && node.attr("clickable") == "true")
        && matches!(for_you.as_slice(), [index] if tree.nodes[*index].visibility() == Some(true)
            && tree.nodes[*index].attr("resource-id") == "android:id/text1"
            && tree.nodes[*index].attr("class") == "android.widget.TextView"
            && tree.nodes[*index].attr("selected") == "true"
            && tree.nodes[*index].attr("enabled") == "true")
    {
        return Some("homeFeed");
    }
    let back = one(":id/bj1")?;
    let caption = one(":id/desc")?;
    let time = one(":id/zwj")?;
    let privacy = one(":id/ror")?;
    // fpv is shared with Like in these captures: uniqueness must use Share semantics,
    // not the resource ID alone, before checking the measured ID and class.
    let shares = tree.matching(PACKAGE, Q::Description { value: "Share video.", exact: false });
    let [share] = shares.as_slice() else { return None };
    let share = &tree.nodes[*share];
    (back.attr("class") == "android.widget.ImageView" && back.attr("content-desc") == "Back"
        && back.attr("clickable") == "true"
        && caption.attr("class") == "X.18oX" && !caption.attr("text").trim().is_empty()
        && time.attr("class") == "android.widget.TextView" && !time.attr("text").trim().is_empty()
        && privacy.attr("class") == "android.widget.Button" && privacy.attr("text") == "Privacy settings"
        && privacy.attr("clickable") == "true"
        && share.attr("resource-id") == "com.zhiliaoapp.musically:id/fpv"
        && share.attr("class") == "android.widget.Button" && share.visibility() == Some(true)
        && share.rect().is_some_and(|rect| rect.enabled && rect.clickable))
        .then_some("ownPostViewer")
}

/// Read-only admission, not proof that either pending publication succeeded.
pub(super) async fn admit_warm(
    session: &dyn riviu_core::UiSession,
    package: &str,
    locale: &str,
    version: &str,
    account: &str,
) -> anyhow::Result<serde_json::Value> {
    anyhow::ensure!(
        matches!((package, locale, version),
            ("com.ss.android.ugc.trill", "en", "38.3.2")
                | ("com.zhiliaoapp.musically", "en", "45.7.3")),
        "shared-debt warm surface not measured for this build"
    );
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "warm session epoch missing");
    let mut previous = None;
    for _ in 0..2 {
        anyhow::ensure!(session.gui_session_epoch() == epoch, "warm session changed");
        anyhow::ensure!(
            session.active_app_bundle().await? == package,
            "foreign foreground app; keep unchanged"
        );
        let snapshot = session.hierarchy_source_snapshot().await?;
        let tree = riviu_core::ui_automation::tree::Tree::parse(snapshot)?;
        anyhow::ensure!(session.gui_session_epoch() == epoch, "warm session changed");
        let surface = if package == "com.zhiliaoapp.musically" {
            global_warm_surface(&tree)
        } else {
            warm_surface(&tree, account)
        }.ok_or_else(|| {
            anyhow::anyhow!("composer/upload or unmeasured warm screen; keep unchanged")
        })?;
        if let Some((generation, before)) = previous {
            anyhow::ensure!(
                tree.generation > generation && surface == before,
                "warm screen changed or stale"
            );
        }
        previous = Some((tree.generation, surface));
    }
    anyhow::ensure!(
        session.active_app_bundle().await? == package,
        "foreground changed; keep unchanged"
    );
    anyhow::ensure!(session.gui_session_epoch() == epoch, "warm session changed");
    Ok(
        serde_json::json!({"state":"keptRunning","reason":"sharedPublicationDebt","surface":previous.map(|(_, s)| s),"package":package}),
    )
}

pub(super) async fn foreground(
    control: &DeviceControlPlane,
    context: &UiSessionContext,
    package: &str,
    restart: bool,
    mut authorize: impl FnMut() -> anyhow::Result<()>,
) -> anyhow::Result<Option<serde_json::Value>> {
    authorize()?;
    if !restart {
        control.foreground_session_app(context, package).await?;
        return Ok(None);
    }
    let started_at = chrono::Utc::now().to_rfc3339();
    let stopped = control.terminate_session_app(context, package).await?;
    anyhow::ensure!(
        stopped.bundle_id == package,
        "Bằng chứng tắt app không khớp TikTok của lượt đã gửi"
    );
    authorize()?;
    // Samsung Android 9 / Global 46.2.42: a Share recipient picker can remain on
    // top after TikTok is stopped. Launch receipts say OK but TikTok stays behind it.
    // Back only from the measured empty recipient picker; never select a recipient
    // or interact with a message composer or another application.
    let session = control.session(context)?;
    for _ in 0..2 {
        if session.active_app_bundle().await.ok().as_deref()
            != Some("com.samsung.android.messaging")
        {
            break;
        }
        let source = session.hierarchy_source_snapshot().await?;
        let tree = riviu_core::ui_automation::tree::Tree::parse(source)?;
        if !(measured_empty_recipient_picker(&tree) || legacy_empty_recipient_picker(&tree))
            || session.active_app_bundle().await.ok().as_deref()
                != Some("com.samsung.android.messaging")
        {
            break;
        }
        authorize()?;
        session.back().await?;
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    }
    authorize()?;
    control.foreground_session_app(context, package).await?;
    authorize()?;
    let running = control
        .inspect_session_app_process(context, package)
        .await?;
    anyhow::ensure!(
        running.bundle_id == package && running.running,
        "TikTok chưa chạy lại; giữ bài đã gửi để kiểm tra sau"
    );
    Ok(Some(
        serde_json::json!({"state":"restarted","package":package,
        "startedAt":started_at,"finishedAt":chrono::Utc::now().to_rfc3339(),
        "oldPid":stopped.old_pid,"newPid":running.pid}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use riviu_core::{ui_automation::tree::Tree, HierarchySourceSnapshot};

    const WARM_VIEWER: &str = r#"<hierarchy><android.widget.FrameLayout package="com.ss.android.ugc.trill" displayed="true" bounds="[0,0][1080,2220]">
          <node package="com.ss.android.ugc.trill" displayed="true" enabled="true" clickable="true" resource-id="com.ss.android.ugc.trill:id/aur" content-desc="Back" bounds="[0,0][100,100]"/>
          <node package="com.ss.android.ugc.trill" displayed="true" enabled="true" class="android.widget.TextView" resource-id="com.ss.android.ugc.trill:id/dmk" text="Fixture caption" bounds="[0,200][100,250]"/>
          <node package="com.ss.android.ugc.trill" displayed="true" enabled="true" resource-id="com.ss.android.ugc.trill:id/qrp" text="27s ago" bounds="[0,300][100,350]"/>
          <node package="com.ss.android.ugc.trill" displayed="true" enabled="true" clickable="true" resource-id="com.ss.android.ugc.trill:id/ma9" text="Privacy settings" bounds="[0,400][100,450]"/>
          <node package="com.ss.android.ugc.trill" displayed="true" enabled="true" clickable="true" content-desc="Share video.  shares" bounds="[0,500][100,550]"/>
        </android.widget.FrameLayout></hierarchy>"#;

    #[test]
    fn verification_warm_surface_accepts_measured_viewer_and_refuses_upload_overlay() {
        // Minimal controls from mutual-guard-screen-170, Trill38.3.2/en.
        // Surface proof permits navigation only, never publication success.
        let xml = WARM_VIEWER;
        assert_eq!(
            warm_surface(&parsed(xml.into()), "fixture.account"),
            Some("ownPostViewer")
        );
        for overlay in [
            r#"<node package="com.ss.android.ugc.trill" displayed="true" class="android.widget.ProgressBar"/>"#,
            r#"<node package="com.ss.android.ugc.trill" displayed="true" class="android.widget.EditText"/>"#,
            r#"<node package="com.ss.android.ugc.trill" displayed="true" text="Uploading 50%"/>"#,
            r#"<node package="com.other.app" displayed="true"/>"#,
        ] {
            assert!(warm_surface(
                &parsed(xml.replace("</hierarchy>", &format!("{overlay}</hierarchy>"))),
                "fixture.account"
            )
            .is_none());
        }
        assert!(warm_surface(
            &parsed(xml.replace("Privacy settings", "Other settings")),
            "fixture.account"
        )
        .is_none());
    }

    struct WarmPhone {
        reads: std::sync::atomic::AtomicU64,
        fault: &'static str,
    }
    #[async_trait::async_trait]
    impl riviu_core::UiSession for WarmPhone {
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(if self.fault == "foreign" {
                "other.app"
            } else {
                "com.ss.android.ugc.trill"
            }
            .into())
        }
        async fn hierarchy_source_snapshot(&self) -> anyhow::Result<HierarchySourceSnapshot> {
            let generation = self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            Ok(HierarchySourceSnapshot {
                generation: if self.fault == "stale" { 1 } else { generation },
                xml: WARM_VIEWER.into(),
            })
        }
        fn gui_session_epoch(&self) -> String {
            if self.fault == "epoch" && self.reads.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                "replaced"
            } else {
                "original"
            }
            .into()
        }
        async fn tap(&self, _: riviu_core::TapPoint) -> anyhow::Result<()> {
            panic!("warm admission cannot tap")
        }
        async fn swipe(&self, _: riviu_core::SwipeGesture) -> anyhow::Result<()> {
            panic!("warm admission cannot swipe")
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("warm admission cannot type")
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("warm admission cannot leave app")
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("warm admission cannot tap")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            panic!("unexpected locator")
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
    }

    #[tokio::test]
    async fn verification_warm_admission_is_read_only_and_requires_fresh_same_epoch_app() {
        for fault in ["none", "foreign", "epoch", "stale"] {
            let phone = WarmPhone {
                reads: std::sync::atomic::AtomicU64::new(0),
                fault,
            };
            let proof = admit_warm(
                &phone,
                "com.ss.android.ugc.trill",
                "en",
                "38.3.2",
                "fixture.account",
            )
            .await;
            assert_eq!(proof.is_ok(), fault == "none", "{fault}");
            if let Ok(proof) = proof {
                assert_eq!(proof["state"], "keptRunning");
            }
        }
    }

    #[test]
    fn verification_observation_allows_two_submitted_debts_but_not_active_work_or_stop() {
        let path =
            std::env::temp_dir().join(format!("warm-verification-{}.db", uuid::Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let raw = rusqlite::Connection::open(&path).unwrap();
        let request = serde_json::json!({
            "requestId":"fixture", "sourceRoot":"fixture", "bundleIds":[], "udids":["phone"],
            "visibility":"public", "cleanupPolicy":"keepImportedAssets", "soundPolicy":{"kind":"default"},
            "sheetEnabled":false, "executionConfirmed":true
        }).to_string();
        for id in ["old", "new"] {
            raw.execute("INSERT INTO publish_campaigns(id,request_id,source_root,request_json,state,created_at,updated_at) VALUES(?1,?1,'fixture',?2,'verifying','now','now')", rusqlite::params![id, request]).unwrap();
            raw.execute("INSERT INTO publish_bundles(id,campaign_id,ordinal,name,source_path,caption,caption_sha256,manifest_json,created_at) VALUES(?1,?1,0,'fixture','fixture','caption',?2,'{}','now')", rusqlite::params![id, "a".repeat(64)]).unwrap();
            let intent = serde_json::json!({"effectIntent":"post", "package":"com.ss.android.ugc.trill", "expectedAccount":"fixture.account", "submittedAt":chrono::Utc::now().to_rfc3339()}).to_string();
            raw.execute("INSERT INTO publish_assignments(id,campaign_id,bundle_id,ordinal,udid,state,effect_intent,evidence_json,created_at,updated_at) VALUES(?1,?1,?1,0,'phone','verifying',?2,?3,'now','now')", rusqlite::params![id, intent, r#"{"post":{"state":"submitted","publicationVerified":false},"verificationStatus":{"state":"pending"}}"#]).unwrap();
        }
        let bundle = serde_json::json!({"id":"old","sourcePath":"fixture","name":"fixture","mediaKind":"image","images":[],"captionPath":"fixture","caption":"caption","captionSha256":"a".repeat(64),"totalBytes":0,"partners":[]}).to_string();
        raw.execute("UPDATE publish_bundles SET manifest_json=?1", [&bundle])
            .unwrap();
        let row = db
            .get_publish_assignment_detail("old", "old")
            .unwrap()
            .unwrap()
            .assignments
            .remove(0);
        assert_eq!(db.publish_device_guard("phone").unwrap().blocking.len(), 2);
        let _lease = db
            .try_publish_work("phone", "verify", "old")
            .unwrap()
            .unwrap();
        assert!(db
            .try_publish_work("phone", "compose", "new")
            .unwrap()
            .is_none());
        assert!(
            authorize(&db, &row, None).is_ok(),
            "settled submissions must allow observational verification under the exclusive lease"
        );
        assert!(
            authorize_restart(&db, &row, None).is_err(),
            "shared debt never grants destructive restart"
        );
        raw.execute("INSERT INTO publish_pipeline_runs(campaign_id,token,created_at) VALUES('new','active','now')", []).unwrap();
        assert!(
            authorize(&db, &row, None).is_err(),
            "a competing active pipeline must remain protected"
        );
        raw.execute(
            "DELETE FROM publish_pipeline_runs WHERE campaign_id='new'",
            [],
        )
        .unwrap();
        raw.execute(
            "UPDATE publish_assignments SET state='posting' WHERE id='new'",
            [],
        )
        .unwrap();
        assert!(
            authorize(&db, &row, None).is_err(),
            "unknown in-flight Post cannot become link debt"
        );
        raw.execute(
            "UPDATE publish_assignments SET state='verifying' WHERE id='new'",
            [],
        )
        .unwrap();
        db.begin_publish_operation_stop("old").unwrap();
        assert!(
            authorize(&db, &row, None).is_err(),
            "Stop must revoke the existing observer"
        );
    }

    fn empty_picker() -> String {
        r#"<hierarchy>
  <node package="com.samsung.android.messaging" displayed="true" bounds="[0,0][2220,1080]">
    <node package="com.samsung.android.messaging" displayed="true" resource-id="com.samsung.android.messaging:id/picker_activity_main" bounds="[126,0][2220,1080]">
      <node package="com.samsung.android.messaging" displayed="true" class="android.widget.EditText" resource-id="com.samsung.android.messaging:id/search_src_text" text="Search Contacts or enter number" hint="Search Contacts or enter number" showing-hint="true" focused="true" bounds="[179,16][2194,132]"/>
      <node package="com.samsung.android.messaging" displayed="true" resource-id="com.samsung.android.messaging:id/empty_layout" bounds="[126,169][2220,922]">
        <node package="com.samsung.android.messaging" displayed="true" resource-id="com.samsung.android.messaging:id/empty_title" text="No contacts" bounds="[179,508][2167,582]"/>
      </node>
      <node package="com.samsung.android.messaging" displayed="true" resource-id="com.samsung.android.messaging:id/title" text="Conversations" content-desc="Conversations, Tab 1 of 2" selected="false" bounds="[542,974][799,1028]"/>
      <node package="com.samsung.android.messaging" displayed="true" resource-id="com.samsung.android.messaging:id/title" text="Contacts" content-desc="Contacts, Tab 2 of 2" selected="true" bounds="[1595,974][1756,1028]"/>
    </node>
  </node>
</hierarchy>"#
            .into()
    }

    fn parsed(xml: String) -> Tree {
        Tree::parse(HierarchySourceSnapshot { generation: 1, xml }).unwrap()
    }

    #[test]
    fn uncertain_processing_receipt_can_request_one_measured_restart() {
        let assignment = PublishAssignmentRecord {
            publication_id: "publication".into(), attempt_id: None, dispatch: None,
            sheet_delivery: None, id: "assignment".into(), campaign_id: "campaign".into(),
            bundle_id: "bundle".into(), ordinal: 0, udid: "phone".into(),
            state: riviu_core::PublishCampaignState::Uncertain,
            effect_intent: Some(serde_json::json!({"effectIntent":"post",
                "package":"com.zhiliaoapp.musically", "expectedAccount":"fixture.account",
                "submittedAt":"2026-09-30T00:00:00Z"}).to_string()),
            evidence_json: Some(serde_json::json!({"effectIntent":"post_carousel",
                "accountDiagnostic":{"state":"proved","package":"com.zhiliaoapp.musically",
                    "expectedAccount":"fixture.account","observedAccount":"fixture.account"},
                "verificationStatus":{"reasonCode":"tiktokProcessing"},
                "verificationDiagnostic":{"package":"com.zhiliaoapp.musically",
                    "locale":"en","version":"45.7.3","copyAttempts":1,
                    "expandedPhotoError":"ProcessingNotice { text: Post is being processed }"}}
            ).to_string()), error_code: None,
        };
        assert!(!requested(&assignment, "com.zhiliaoapp.musically"));
        assert!(processing_restart_requested(&assignment,
            "com.zhiliaoapp.musically", "en", "45.7.3"));
        assert!(!processing_restart_requested(&assignment,
            "com.zhiliaoapp.musically", "en", "45.4.3"));
        let mut missing_copy = assignment.clone();
        let mut evidence: serde_json::Value = serde_json::from_str(
            missing_copy.evidence_json.as_deref().unwrap()).unwrap();
        evidence["verificationDiagnostic"]["copyAttempts"] = 0.into();
        missing_copy.evidence_json = Some(evidence.to_string());
        assert!(!processing_restart_requested(&missing_copy,
            "com.zhiliaoapp.musically", "en", "45.7.3"));
    }

    #[test]
    fn measured_samsung_empty_recipient_picker_allows_only_empty_contacts_view() {
        let xml = empty_picker();
        assert!(super::measured_empty_recipient_picker(&parsed(xml.clone())));
        for invalid in [
            xml.replace("picker_activity_main", "message_composer"),
            xml.replace("empty_layout", "contact_list"),
            xml.replace("No contacts", "One contact"),
            xml.replace("selected=\"true\"", "selected=\"false\""),
            xml.replace("showing-hint=\"true\"", "showing-hint=\"false\""),
            xml.replace("Search Contacts or enter number\" hint", "123456789\" hint"),
            xml.replace(
                "com.samsung.android.messaging",
                "com.other.messaging",
            ),
            xml.replace(
                "    </node>\n  </node>\n</hierarchy>",
                "      <node package=\"com.samsung.android.messaging\" displayed=\"true\" text=\"123456789\" bounds=\"[200,200][500,250]\"/>\n    </node>\n  </node>\n</hierarchy>",
            ),
        ] {
            assert!(!super::measured_empty_recipient_picker(&parsed(invalid)));
        }
    }

    #[test]
    fn previous_samsung_recipient_labels_still_identify_the_empty_picker() {
        let xml = r#"<hierarchy><node package="com.samsung.android.messaging" bounds="[0,0][1080,2220]"><node package="com.samsung.android.messaging" text="Select recipients" bounds="[20,20][400,80]"/><node package="com.samsung.android.messaging" text="No contacts" bounds="[20,300][400,360]"/></node></hierarchy>"#;
        assert!(super::legacy_empty_recipient_picker(&parsed(xml.into())));
        assert!(!super::legacy_empty_recipient_picker(&parsed(
            xml.replace("No contacts", "One contact")
        )));
    }
}
