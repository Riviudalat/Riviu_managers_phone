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

pub(super) fn authorize(
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
            !db.publish_operation_stopped(&assignment.campaign_id)?
                && current
                    .as_ref()
                    .is_some_and(|row| row.state == assignment.state
                        && row.effect_intent == assignment.effect_intent
                        && row.evidence_json == assignment.evidence_json),
            "Lượt xác minh đã thay đổi hoặc đã dừng"
        );
    }
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
    use riviu_core::{ui_automation::tree::Tree, HierarchySourceSnapshot};

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
