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
    let elements = tree
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, n)| {
            if !n.visible(&package) || !tree.ancestors_visible(index) {
                return None;
            }
            let rect = n.rect()?;
            let val = |s: &str| (!s.trim().is_empty()).then(|| s.to_owned());
            let description = val(n.attr("content-desc"));
            let text = if description.is_none() {
                val(n.attr("text"))
            } else {
                None
            };
            let resource_id = if description.is_none() && text.is_none() {
                val(n.attr("resource-id"))
            } else {
                None
            };
            let mut selector = ElementSelector {
                package: package.clone(),
                description,
                text,
                resource_id,
                class_name: None,
            };
            if selector.matches(&tree).len() != 1 {
                selector.class_name = val(n.attr("class"));
                selector.resource_id = val(n.attr("resource-id"));
            }
            let selector = (selector.validate().is_ok() && selector.matches(&tree).len() == 1)
                .then_some(selector);
            Some(InspectorElement {
                index,
                parent: n.parent,
                text: n.attr("text").into(),
                description: n.attr("content-desc").into(),
                resource_id: n.attr("resource-id").into(),
                class_name: n.attr("class").into(),
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                enabled: rect.enabled,
                clickable: rect.clickable,
                selector,
            })
        })
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
    let expected = after
        .elements
        .iter()
        .filter(|e| e.selector.is_some() && (!e.text.is_empty() || !e.description.is_empty()))
        .filter(|e| !before.elements.iter().any(|old| old.selector == e.selector))
        // Prefer screen controls to account handles and changing counters so a
        // navigation recording can be replayed on another device/account.
        .min_by_key(|e| {
            if [
                "Edit profile",
                "Edit",
                "Sửa hồ sơ",
                "Profile menu",
                "Menu hồ sơ",
                "For You",
                "Dành cho bạn",
            ]
            .iter()
            .any(|label| e.text == *label || e.description == *label)
            {
                0
            } else if !e.description.is_empty() && e.clickable {
                1
            } else {
                2
            }
        })
        .and_then(|e| e.selector.clone());
    let verified = action_error.is_none() && expected.is_some();
    if let Some(record) = record.as_mut() {
        record.steps.push(RecordedStep {
            selector,
            before_id: before.id,
            after_id: after.id.clone(),
            verified,
            expected,
            error: action_error,
        });
        state
            .db
            .set_setting(&key(&udid), &serde_json::to_string(record).map_err(err)?)
            .map_err(err)?;
    }
    if !verified {
        return Err(err(
            "Đã thử bấm; chưa thấy thay đổi giao diện. Kiểm tra bằng chứng, không tự bấm lại.",
        ));
    }
    Ok(after)
}
