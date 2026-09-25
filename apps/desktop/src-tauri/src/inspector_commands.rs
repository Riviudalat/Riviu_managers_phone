//! One inspection/recording path for UI and MCP. All device work shares Riviu leases.
use crate::{command_error::CommandError, commands::with_manual_session, state::AppState};
use base64::{engine::general_purpose::STANDARD, Engine};
use riviu_core::{
    ui_automation::{
        inspector::{resolve_unique, ElementSelector},
        tree::Tree,
    },
    DeviceWorkOwner,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;

static RECORD_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorElement {
    pub index: usize,
    pub parent: Option<usize>,
    #[serde(default)]
    pub package: String,
    pub text: String,
    pub description: String,
    pub resource_id: String,
    pub class_name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub enabled: bool,
    pub clickable: bool,
    #[serde(default)]
    pub checkable: Option<bool>,
    #[serde(default)]
    pub checked: Option<bool>,
    #[serde(default)]
    pub selected: Option<bool>,
    #[serde(default)]
    pub focusable: Option<bool>,
    #[serde(default)]
    pub focused: Option<bool>,
    #[serde(default)]
    pub scrollable: Option<bool>,
    #[serde(default)]
    pub long_clickable: Option<bool>,
    #[serde(default)]
    pub password: Option<bool>,
    pub selector: Option<ElementSelector>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorSnapshot {
    pub id: String,
    pub udid: String,
    pub package: String,
    pub version: String,
    pub locale: String,
    pub width: f64,
    pub height: f64,
    pub png_base64: String,
    pub tree_sha256: String,
    /// Exact source for local adapter fixtures; hash refers to these bytes.
    #[serde(default)]
    pub hierarchy_xml: String,
    pub elements: Vec<InspectorElement>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedStep {
    pub selector: ElementSelector,
    pub before_id: String,
    pub after_id: String,
    pub verified: bool,
    #[serde(default)]
    pub expected: Option<ElementSelector>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorRecording {
    pub id: String,
    pub udid: String,
    pub name: String,
    pub steps: Vec<RecordedStep>,
    pub active: bool,
}
fn err(e: impl std::fmt::Display) -> CommandError {
    CommandError::operation(e)
}
fn key(udid: &str) -> String {
    format!("inspector.recording.{udid}")
}
async fn capture(
    session: &dyn riviu_core::UiSession,
    udid: &str,
) -> anyhow::Result<InspectorSnapshot> {
    let package = session.active_app_bundle().await?;
    let source = session.hierarchy_source_snapshot().await?;
    let digest = format!("{:x}", Sha256::digest(source.xml.as_bytes()));
    let hierarchy_xml = source.xml.clone();
    let tree = Tree::parse(source)?;
    let (width, height) = session.window_size().await?;
    let png = session.screenshot_png().await?;
    anyhow::ensure!(
        package == session.active_app_bundle().await?,
        "inspector_app_changed"
    );
    let elements = (0..tree.nodes.len())
        .filter_map(|index| element_from_tree(&tree, index, &package))
        .collect();
    Ok(InspectorSnapshot {
        id: uuid::Uuid::new_v4().to_string(),
        udid: udid.into(),
        version: session.app_version(&package).await.unwrap_or_default(),
        locale: session.ui_language().await.unwrap_or_default(),
        package,
        width,
        height,
        png_base64: STANDARD.encode(png),
        tree_sha256: digest,
        hierarchy_xml,
        elements,
    })
}
fn xml_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}
fn element_from_tree(tree: &Tree, index: usize, package: &str) -> Option<InspectorElement> {
    let node = tree.nodes.get(index)?;
    if !node.visible(package) || !tree.ancestors_visible(index) {
        return None;
    }
    let rect = node.rect()?;
    let selector = riviu_core::ui_automation::inspector::selector_for_node(tree, index, package);
    Some(InspectorElement {
        index,
        parent: node.parent,
        package: node.attr("package").into(),
        text: node.attr("text").into(),
        description: node.attr("content-desc").into(),
        resource_id: node.attr("resource-id").into(),
        class_name: node.attr("class").into(),
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        enabled: rect.enabled,
        clickable: rect.clickable,
        checkable: xml_bool(node.attr("checkable")),
        checked: xml_bool(node.attr("checked")),
        selected: xml_bool(node.attr("selected")),
        focusable: xml_bool(node.attr("focusable")),
        focused: xml_bool(node.attr("focused")),
        scrollable: xml_bool(node.attr("scrollable")),
        long_clickable: xml_bool(node.attr("long-clickable")),
        password: xml_bool(node.attr("password")),
        selector,
    })
}
fn save_observation(state: &AppState, snapshot: &InspectorSnapshot) -> Result<(), CommandError> {
    let dir = state.artifacts_dir.join("inspector");
    std::fs::create_dir_all(&dir).map_err(err)?;
    std::fs::write(
        dir.join(format!("{}.json", snapshot.id)),
        serde_json::to_vec(snapshot).map_err(err)?,
    )
    .map_err(err)
}
pub async fn observe(state: &AppState, udid: String) -> Result<InspectorSnapshot, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let id = udid.clone();
    let snapshot = with_manual_session(
        state,
        &udid,
        DeviceWorkOwner::ManualControl,
        move |session| async move { capture(session.as_ref(), &id).await },
    )
    .await?;
    save_observation(state, &snapshot)?;
    Ok(snapshot)
}
pub fn recording(state: &AppState, udid: &str) -> Result<Option<InspectorRecording>, CommandError> {
    state
        .db
        .get_setting(&key(udid))
        .map_err(err)?
        .map(|v| serde_json::from_str(&v).map_err(err))
        .transpose()
}
#[tauri::command]
pub async fn inspector_observe(
    state: State<'_, AppState>,
    udid: String,
) -> Result<InspectorSnapshot, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    observe(&state, udid).await
}
#[tauri::command]
pub fn inspector_recording(
    state: State<'_, AppState>,
    udid: String,
) -> Result<Option<InspectorRecording>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    recording(&state, &udid)
}
#[tauri::command]
pub async fn inspector_record(
    state: State<'_, AppState>,
    udid: String,
    name: String,
    active: bool,
) -> Result<InspectorRecording, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    record(&state, udid, name, active).await
}
pub async fn record(
    state: &AppState,
    udid: String,
    name: String,
    active: bool,
) -> Result<InspectorRecording, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let _lock = RECORD_LOCK.lock().await;
    let existing = recording(state, &udid)?;
    let value = if active {
        if existing.as_ref().is_some_and(|r| r.active) {
            return Err(err("Đang ghi; dừng phiên hiện tại trước"));
        }
        InspectorRecording {
            id: uuid::Uuid::new_v4().to_string(),
            udid: udid.clone(),
            name: name.chars().take(120).collect(),
            steps: vec![],
            active: true,
        }
    } else {
        let mut r = existing.ok_or_else(|| err("Chưa có phiên ghi"))?;
        r.active = false;
        r
    };
    state
        .db
        .set_setting(&key(&udid), &serde_json::to_string(&value).map_err(err)?)
        .map_err(err)?;
    Ok(value)
}
#[tauri::command]
pub async fn inspector_tap(
    state: State<'_, AppState>,
    udid: String,
    selector: ElementSelector,
) -> Result<InspectorSnapshot, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    tap(&state, udid, selector).await
}

#[tauri::command]
pub fn inspector_confirm_postcondition(
    state: State<'_, AppState>,
    udid: String,
    snapshot_id: String,
    expected: ElementSelector,
) -> Result<InspectorRecording, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    confirm_postcondition(&state, &udid, &snapshot_id, expected)
}

fn read_observation(state: &AppState, id: &str) -> Result<InspectorSnapshot, CommandError> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(CommandError::invalid_argument(
            "Mã bằng chứng Inspector không hợp lệ",
        ));
    }
    let root = state.artifacts_dir.join("inspector");
    let path = root.join(format!("{id}.json"));
    let bytes = std::fs::read(&path).map_err(err)?;
    serde_json::from_slice(&bytes).map_err(err)
}

fn confirm_postcondition(
    state: &AppState,
    udid: &str,
    snapshot_id: &str,
    expected: ElementSelector,
) -> Result<InspectorRecording, CommandError> {
    expected.validate().map_err(err)?;
    let mut record = recording(state, udid)?.ok_or_else(|| err("Chưa có phiên ghi"))?;
    let step = record
        .steps
        .last_mut()
        .ok_or_else(|| err("Chưa có bước cần xác minh"))?;
    if step.verified
        || step.after_id != snapshot_id
        || step
            .error
            .as_deref()
            .is_none_or(|v| v != "Chờ chọn phần tử kết quả")
    {
        return Err(err(
            "Bước ghi đã thay đổi; đọc lại Inspector trước khi xác minh",
        ));
    }
    let before = read_observation(state, &step.before_id)?;
    let after = read_observation(state, &step.after_id)?;
    if before.udid != udid || after.udid != udid || after.package != expected.package {
        return Err(err("Bằng chứng không thuộc đúng máy hoặc ứng dụng"));
    }
    if !postcondition_appeared(&before.elements, &after.elements, &expected) {
        return Err(err(
            "Phần tử kết quả phải xuất hiện sau thao tác và chưa có trong màn hình trước đó",
        ));
    }
    step.expected = Some(expected);
    step.verified = true;
    step.error = None;
    state
        .db
        .set_setting(&key(udid), &serde_json::to_string(&record).map_err(err)?)
        .map_err(err)?;
    Ok(record)
}

fn postcondition_appeared(
    before: &[InspectorElement],
    after: &[InspectorElement],
    expected: &ElementSelector,
) -> bool {
    after
        .iter()
        .any(|element| element.selector.as_ref() == Some(expected))
        && !before
            .iter()
            .any(|element| element.selector.as_ref() == Some(expected))
}
pub async fn tap(
    state: &AppState,
    udid: String,
    selector: ElementSelector,
) -> Result<InspectorSnapshot, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let _lock = RECORD_LOCK.lock().await;
    let id = udid.clone();
    let selected = selector.clone();
    let mut record = recording(state, &udid)?.filter(|r| r.active);
    if record.as_ref().is_some_and(|r| r.steps.len() >= 100) {
        return Err(err("Phiên ghi đã đủ 100 bước; dừng và lưu trước"));
    }
    let artifact_dir = state.artifacts_dir.join("inspector");
    let record_key = key(&udid);
    let db = state.db.clone();
    let intent_selector = selector.clone();
    let mut pending_record = record.clone();
    let (before, after, action_error) = with_manual_session(
        state,
        &udid,
        DeviceWorkOwner::ManualControl,
        move |session| async move {
            let before = capture(session.as_ref(), &id).await?;
            let target = resolve_unique(session.as_ref(), &selected).await?;
            anyhow::ensure!(target.clickable, "inspector_element_not_clickable");
            // Persist intent before dispatch: a failed screenshot or process exit after
            // the tap must leave an unverified step instead of silently dropping it.
            std::fs::create_dir_all(&artifact_dir)?;
            std::fs::write(
                artifact_dir.join(format!("{}.json", before.id)),
                serde_json::to_vec(&before)?,
            )?;
            if let Some(pending) = pending_record.as_mut() {
                pending.steps.push(RecordedStep {
                    selector: intent_selector,
                    before_id: before.id.clone(),
                    after_id: String::new(),
                    verified: false,
                    expected: None,
                    error: Some("Chờ xác minh thao tác".into()),
                });
                db.set_setting(&record_key, &serde_json::to_string(pending)?)?;
            }
            let error = session
                .tap(target.centre())
                .await
                .err()
                .map(|e| e.to_string());
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
            let after = capture(session.as_ref(), &id).await?;
            Ok((before, after, error))
        },
    )
    .await?;
    save_observation(state, &before)?;
    save_observation(state, &after)?;
    let verified = record.is_none() && action_error.is_none();
    if let Some(record) = record.as_mut() {
        record.steps.push(RecordedStep {
            selector,
            before_id: before.id,
            after_id: after.id.clone(),
            verified: false,
            expected: None,
            error: action_error.or_else(|| Some("Chờ chọn phần tử kết quả".into())),
        });
        state
            .db
            .set_setting(&key(&udid), &serde_json::to_string(record).map_err(err)?)
            .map_err(err)?;
    }
    if record.is_none() && !verified {
        return Err(err(
            "Đã thử bấm nhưng driver chưa xác nhận thao tác; không tự bấm lại.",
        ));
    }
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selector(text: &str) -> ElementSelector {
        ElementSelector {
            package: "app.fixture".into(),
            text: Some(text.into()),
            description: None,
            resource_id: None,
            class_name: None,
            schema_version: None,
            text_prefix: None,
            description_prefix: None,
            scope: None,
            action_target: None,
        }
    }

    fn element(index: usize, selector: ElementSelector) -> InspectorElement {
        InspectorElement {
            index,
            parent: None,
            package: "app.fixture".into(),
            text: selector.text.clone().unwrap_or_default(),
            description: String::new(),
            resource_id: String::new(),
            class_name: "android.widget.Button".into(),
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            enabled: true,
            clickable: true,
            checkable: None,
            checked: None,
            selected: None,
            focusable: None,
            focused: None,
            scrollable: None,
            long_clickable: None,
            password: None,
            selector: Some(selector),
        }
    }

    #[test]
    fn element_properties_preserve_true_false_and_missing_xml_attributes() {
        let tree = Tree::parse(riviu_core::HierarchySourceSnapshot {
            generation: 1,
            xml: concat!(
                "<hierarchy>",
                "<node package=\"app.fixture\" bounds=\"[10,20][30,40]\" enabled=\"true\" clickable=\"false\" ",
                "checkable=\"true\" checked=\"false\" selected=\"true\" focusable=\"false\" focused=\"true\" ",
                "scrollable=\"false\" long-clickable=\"true\" password=\"false\" />",
                "<node package=\"app.fixture\" bounds=\"[30,40][50,60]\" />",
                "</hierarchy>"
            )
            .into(),
        })
        .unwrap();

        let full = element_from_tree(&tree, 1, "app.fixture").unwrap();
        assert_eq!(full.package, "app.fixture");
        assert_eq!(full.checkable, Some(true));
        assert_eq!(full.checked, Some(false));
        assert_eq!(full.selected, Some(true));
        assert_eq!(full.focusable, Some(false));
        assert_eq!(full.focused, Some(true));
        assert_eq!(full.scrollable, Some(false));
        assert_eq!(full.long_clickable, Some(true));
        assert_eq!(full.password, Some(false));
        let json = serde_json::to_value(&full).unwrap();
        assert_eq!(json["longClickable"], true);

        let missing = element_from_tree(&tree, 2, "app.fixture").unwrap();
        assert_eq!(missing.checkable, None);
        assert_eq!(missing.long_clickable, None);

        let mut old_json = serde_json::to_value(element(1, selector("Home"))).unwrap();
        for key in [
            "package",
            "checkable",
            "checked",
            "selected",
            "focusable",
            "focused",
            "scrollable",
            "longClickable",
            "password",
        ] {
            old_json.as_object_mut().unwrap().remove(key);
        }
        let old: InspectorElement = serde_json::from_value(old_json).unwrap();
        assert_eq!(old.package, "");
        assert_eq!(old.checkable, None);
    }

    #[test]
    fn recorder_requires_an_explicit_new_postcondition() {
        let expected = selector("Profile");
        assert!(postcondition_appeared(
            &[element(1, selector("Home"))],
            &[element(2, expected.clone())],
            &expected,
        ));
        assert!(!postcondition_appeared(
            &[element(1, expected.clone())],
            &[element(2, expected.clone())],
            &expected,
        ));
        assert!(!postcondition_appeared(
            &[element(1, selector("Home"))],
            &[element(2, selector("Inbox"))],
            &expected,
        ));
    }
}
