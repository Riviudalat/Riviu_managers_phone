#![allow(clippy::result_large_err)]

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use riviu_core::db::FlowStateConflict;
use riviu_core::{
    decode_and_hash_artifact, qualified_geometry_profile_id, release_one_catalog, ActionDefinition,
    AutomationScript, FlowArtifactRecord, FlowDocumentV2, FlowNodeAttemptRecord, FlowNotFound,
    FlowRetryError, FlowRevisionRecord, FlowRunDetail, FlowRunRecord, FlowRuntimeError,
    FlowSelectionError, FlowSummary, FlowTargetSelection, RevisionConflict, ScreenOrientation,
};
use riviu_script_engine::{compile_flow, import_legacy_v1, CompiledRevision, LegacyImportResult};
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::command_error::CommandError;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FlowCoordinateFrame {
    pub jpeg_base64: String,
    pub image_width: u32,
    pub image_height: u32,
    pub orientation: String,
    pub profile_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FlowArtifactPayload {
    pub artifact_id: Uuid,
    pub label: String,
    pub kind: String,
    pub size: u64,
    pub sha256: String,
    pub base64: String,
}

#[tauri::command]
pub fn flow_action_catalog() -> Vec<ActionDefinition> {
    release_one_catalog()
}

#[tauri::command]
pub fn flow_list(
    state: State<'_, AppState>,
    include_archived: bool,
) -> Result<Vec<FlowSummary>, CommandError> {
    state
        .db
        .list_flows(include_archived)
        .map_err(map_service_error)
}

#[tauri::command]
pub fn flow_get(
    state: State<'_, AppState>,
    id: String,
    revision: Option<u64>,
) -> Result<Option<FlowRevisionRecord>, CommandError> {
    state
        .db
        .get_flow_revision(parse_uuid(&id, "flow ID")?, revision)
        .map_err(map_service_error)
}

#[tauri::command]
pub fn flow_validate(document: FlowDocumentV2) -> Result<CompiledRevision, Vec<CommandError>> {
    compile_flow(&document, &release_one_catalog())
        .map_err(|errors| errors.into_iter().map(CommandError::from_compile).collect())
}

#[tauri::command]
pub fn flow_save_revision(
    state: State<'_, AppState>,
    document: FlowDocumentV2,
    expected_revision: Option<u64>,
) -> Result<FlowRevisionRecord, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let (document, compiled) = compile_for_save(document, expected_revision)?;
    state.flow_mutations.commit(&state.events, || {
        let saved = state
            .db
            .save_flow_revision(
                expected_revision,
                &document,
                &compiled.plan,
                &compiled.sha256,
            )
            .map_err(map_service_error)?;
        let flow_id = saved.document.id;
        Ok((saved, flow_id))
    })
}

#[tauri::command]
pub fn flow_archive(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let id = parse_uuid(&id, "flow ID")?;
    state.flow_mutations.commit(&state.events, || {
        let archived = state
            .db
            .archive_flow_atomic(id)
            .map_err(map_service_error)?;
        Ok(((), archived.flow_id))
    })
}

/// The same ceiling the V2 JSON dialog enforces (`MAX_FLOW_JSON_BYTES` in
/// `FlowJsonDialog.tsx`). The two import doors must refuse at the same size, or the one
/// without the cap becomes the way around it.
const MAX_LEGACY_SCRIPT_BYTES: usize = 1_048_576;

/// Whether a legacy script is too large to accept at all.
///
/// Asked before `serde_json::from_str`, because by the time serde answers, the whole string
/// has already been held by React, copied across IPC and walked by the parser — a pasted
/// hundred-megabyte `steps` array froze the desktop before any validation could refuse it.
fn legacy_script_too_large(script_json: &str) -> bool {
    script_json.len() > MAX_LEGACY_SCRIPT_BYTES
}

#[tauri::command]
pub fn flow_import_legacy(script_json: String) -> Result<LegacyImportResult, CommandError> {
    if legacy_script_too_large(&script_json) {
        return Err(CommandError::code(
            "FlowImportTooLarge",
            "legacy script exceeds 1 MiB; export it in smaller parts",
        ));
    }
    let script: AutomationScript = serde_json::from_str(&script_json)
        .map_err(|_| CommandError::invalid_argument("legacy script JSON is invalid"))?;
    Ok(import_legacy_v1(&script))
}

#[tauri::command]
pub fn flow_export(
    state: State<'_, AppState>,
    id: String,
    revision: Option<u64>,
) -> Result<String, CommandError> {
    let id = parse_uuid(&id, "flow ID")?;
    let record = state
        .db
        .get_flow_revision(id, revision)
        .map_err(map_service_error)?
        .ok_or_else(|| CommandError::code("FlowNotFound", "Flow revision does not exist"))?;
    serde_json::to_string_pretty(&record.document)
        .map_err(|_| CommandError::code("SerializationFailed", "Flow export failed"))
}

#[tauri::command]
pub async fn flow_run(
    state: State<'_, AppState>,
    id: String,
    revision: Option<u64>,
    selection: FlowTargetSelection,
) -> Result<FlowRunRecord, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let id = parse_uuid(&id, "flow ID")?;
    let record = state
        .db
        .get_flow_revision(id, revision)
        .map_err(map_service_error)?
        .ok_or_else(|| CommandError::code("FlowNotFound", "Flow revision does not exist"))?;
    state
        .flows
        .enqueue(record, selection)
        .await
        .map_err(map_service_error)
}

#[tauri::command]
pub fn flow_cancel_run(state: State<'_, AppState>, run_id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .flows
        .cancel_run(parse_uuid(&run_id, "Flow run ID")?)
        .map_err(map_service_error)
}

#[tauri::command]
pub async fn flow_retry_attempt(
    state: State<'_, AppState>,
    attempt_id: String,
) -> Result<FlowNodeAttemptRecord, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let parsed = parse_uuid(&attempt_id, "Flow attempt ID")?;
    state.flows.retry_attempt(parsed).await.map_err(|error| {
        let mut mapped = map_service_error(error);
        mapped.attempt_id = Some(parsed.to_string().into_boxed_str());
        mapped
    })
}

#[tauri::command]
pub fn flow_list_runs(
    state: State<'_, AppState>,
    limit: usize,
) -> Result<Vec<FlowRunRecord>, CommandError> {
    if !(1..=200).contains(&limit) {
        return Err(CommandError::invalid_argument(
            "Flow run list limit must be 1..=200",
        ));
    }
    state.db.list_flow_runs(limit).map_err(map_service_error)
}

#[tauri::command]
pub fn flow_get_run(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Option<FlowRunDetail>, CommandError> {
    state
        .db
        .get_flow_run(parse_uuid(&run_id, "Flow run ID")?)
        .map_err(map_service_error)
}

#[tauri::command]
pub async fn flow_coordinate_frame(
    state: State<'_, AppState>,
    udid: String,
    bundle_id: String,
) -> Result<FlowCoordinateFrame, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    validate_exact(&udid, "device UDID")?;
    validate_exact(&bundle_id, "bundle ID")?;

    // Retain and decode before acquiring the device. The command never asks WDA for a
    // screenshot, and on a device whose frames are already in the hub it starts nothing.
    let frame = match state.streams.latest(&udid) {
        Some(frame) => frame,
        None => borrow_one_frame(&state, &udid).await?,
    };
    let decoded = decode_and_hash_artifact(frame.as_slice()).map_err(|_| {
        device_error(
            "FrameInvalid",
            "The current stream frame is not a valid image",
            &udid,
        )
    })?;
    if decoded.format != "jpeg" || decoded.width == 0 || decoded.height == 0 {
        return Err(device_error(
            "FrameInvalid",
            "The coordinate frame must be a non-empty JPEG",
            &udid,
        ));
    }

    let context = state
        .control
        .try_acquire_exclusive(&udid, riviu_core::DeviceWorkOwner::ManualControl)
        .await
        .map_err(CommandError::from)?;
    let inspected = state
        .control
        .inspect_flow_device(&context, &bundle_id)
        .await;
    let released = state.control.close_exclusive_context(context);
    let snapshot = inspected.map_err(CommandError::from)?;
    released.map_err(CommandError::from)?;

    if snapshot.target_app.bundle_id != bundle_id || !snapshot.protected_auth_ready {
        return Err(device_error(
            "ProtectedAuthUnavailable",
            "The device has no target-qualified protected authentication snapshot",
            &udid,
        ));
    }
    let geometry = snapshot.geometry.as_ref().ok_or_else(|| {
        device_error(
            "QualifiedGeometryUnavailable",
            "The device has no qualified screen geometry",
            &udid,
        )
    })?;
    if (decoded.width, decoded.height) != (geometry.pixel_width, geometry.pixel_height) {
        return Err(device_error(
            "FrameGeometryMismatch",
            "The retained frame does not match the qualified device geometry",
            &udid,
        ));
    }
    let profile_id = qualified_geometry_profile_id(&snapshot).map_err(|_| {
        device_error(
            "QualifiedGeometryUnavailable",
            "The device geometry profile is invalid",
            &udid,
        )
    })?;

    Ok(FlowCoordinateFrame {
        jpeg_base64: BASE64.encode(frame.as_slice()),
        image_width: decoded.width,
        image_height: decoded.height,
        orientation: orientation_name(geometry.orientation).to_string(),
        profile_id,
    })
}

#[tauri::command]
pub fn flow_read_artifact(
    state: State<'_, AppState>,
    artifact_id: String,
) -> Result<FlowArtifactPayload, CommandError> {
    let artifact_id = parse_uuid(&artifact_id, "Flow artifact ID")?;
    let record = state
        .db
        .get_flow_artifact(artifact_id)
        .map_err(map_service_error)?
        .ok_or_else(|| CommandError::code("ArtifactNotFound", "Flow artifact does not exist"))?;
    let bytes = state
        .flow_artifacts
        .read_committed_image(&record)
        .map_err(|error| {
            log::error!("Flow artifact validation failed: {error:#}");
            artifact_integrity_error()
        })?;
    artifact_payload(record, bytes)
}

fn compile_one(document: &FlowDocumentV2) -> Result<CompiledRevision, CommandError> {
    compile_flow(document, &release_one_catalog()).map_err(|errors| {
        errors
            .into_iter()
            .next()
            .map(CommandError::from_compile)
            .unwrap_or_else(|| CommandError::code("CompileFailed", "Flow compilation failed"))
    })
}

fn compile_for_save(
    mut document: FlowDocumentV2,
    expected_revision: Option<u64>,
) -> Result<(FlowDocumentV2, CompiledRevision), CommandError> {
    document.revision = expected_revision
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| CommandError::invalid_argument("Flow revision exceeds u64"))?;
    let compiled = compile_one(&document)?;
    Ok((document, compiled))
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, CommandError> {
    validate_exact(value, field)?;
    let parsed = Uuid::parse_str(value)
        .map_err(|_| CommandError::invalid_argument(format!("{field} must be a canonical UUID")))?;
    if parsed.to_string() != value {
        return Err(CommandError::invalid_argument(format!(
            "{field} must be a canonical UUID"
        )));
    }
    Ok(parsed)
}

fn validate_exact(value: &str, field: &str) -> Result<(), CommandError> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(CommandError::invalid_argument(format!(
            "{field} must be non-empty and exact"
        )));
    }
    Ok(())
}

fn map_service_error(error: anyhow::Error) -> CommandError {
    let typed = error.downcast_ref::<RevisionConflict>().is_some()
        || error.downcast_ref::<FlowNotFound>().is_some()
        || error.downcast_ref::<FlowSelectionError>().is_some()
        || error.downcast_ref::<FlowRetryError>().is_some()
        || error.downcast_ref::<FlowRuntimeError>().is_some();
    if typed {
        return CommandError::from_service(error);
    }
    if let Some(conflict) = error.downcast_ref::<FlowStateConflict>() {
        return CommandError::code("StateConflict", conflict.to_string());
    }
    log::error!("Flow command failed: {error:#}");
    CommandError::code("OperationFailed", "The Flow operation failed")
}

fn device_error(code: &'static str, message: &'static str, udid: &str) -> CommandError {
    let mut error = CommandError::code(code, message);
    error.udid = Some(udid.to_string().into_boxed_str());
    error
}

/// How long a borrowed producer is given to publish its first frame.
///
/// The same 12 s the Android interaction handoff allows for one decoded frame. It is a
/// failure deadline, not a poll interval: a phone that has not produced anything in twelve
/// seconds is not slow, it is not producing.
const BORROWED_FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(12);

/// Start a producer just long enough to take one frame, then give it back.
///
/// Android devices publish **nothing** into the host's JPEG hub during ordinary use: their
/// tiles are on the H.264 view path, and `background_sample_candidate` skips Android
/// outright. So "no current stream frame is available" was not a device that had gone
/// quiet — it was the permanent, unavoidable state of every phone on this fleet, and it is
/// what made the Flow inspector's "Chọn toạ độ" and "Chụp mẫu từ thiết bị" buttons
/// unanswerable no matter how long an operator waited.
///
/// Borrowed rather than left running, and stopped rather than parked: the caller has the
/// bytes by then, and a producer left behind by a picker would sit against the device's
/// stream budget until something else happened to stop it.
async fn borrow_one_frame(state: &AppState, udid: &str) -> Result<riviu_core::Frame, CommandError> {
    let lease = state
        .control
        .reserve_background_stream(udid)
        .map_err(CommandError::from)?;
    let mut stream = riviu_core::FrameSource::subscribe(&state.streams, udid);
    let started = state.control.start_background_stream(&lease).await;
    let frame = match started {
        Ok(_) => {
            // The hub is checked again first: a producer can publish between the start
            // returning and this subscription being read.
            match state.streams.latest(udid) {
                Some(frame) => Ok(frame),
                None => tokio::time::timeout(BORROWED_FRAME_TIMEOUT, stream.next())
                    .await
                    .ok()
                    .flatten()
                    .ok_or_else(|| {
                        device_error(
                            "FrameUnavailable",
                            "The device produced no frame to pick coordinates on",
                            udid,
                        )
                    }),
            }
        }
        Err(error) => Err(CommandError::from(error)),
    };
    // Stopped whether or not a frame arrived. A borrowed producer that outlives its
    // borrower is exactly the leak the budget exists to prevent.
    if let Err(error) = state.control.stop_background_stream(&lease).await {
        log::warn!("could not stop the borrowed coordinate producer on {udid}: {error}");
    }
    frame
}

fn orientation_name(orientation: ScreenOrientation) -> &'static str {
    match orientation {
        ScreenOrientation::Portrait => "portrait",
        ScreenOrientation::PortraitUpsideDown => "portraitUpsideDown",
        ScreenOrientation::LandscapeLeft => "landscapeLeft",
        ScreenOrientation::LandscapeRight => "landscapeRight",
    }
}

fn artifact_payload(
    record: FlowArtifactRecord,
    bytes: Vec<u8>,
) -> Result<FlowArtifactPayload, CommandError> {
    let decoded = decode_and_hash_artifact(&bytes).map_err(|_| artifact_integrity_error())?;
    if decoded.format != record.kind
        || decoded.size != record.size
        || decoded.sha256 != record.sha256
        || decoded.width == 0
        || decoded.height == 0
    {
        return Err(artifact_integrity_error());
    }

    Ok(FlowArtifactPayload {
        artifact_id: record.id,
        label: record.label.clone(),
        kind: record.kind.clone(),
        size: record.size,
        sha256: record.sha256.clone(),
        base64: BASE64.encode(bytes),
    })
}

fn artifact_integrity_error() -> CommandError {
    CommandError::code(
        "ArtifactIntegrityFailed",
        "Flow artifact integrity validation failed",
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    const FIXTURE_JPEG: &[u8] =
        include_bytes!("../../../../crates/core/tests/fixtures/feed-iphone8.jpg");

    /// **The refusal sits at the byte boundary, and exactly there.** One byte under the V2
    /// dialog's ceiling parses; one byte over is refused before serde ever runs. A cap that
    /// triggered early would refuse real scripts; one that triggered late is no cap.
    #[test]
    fn legacy_import_refuses_oversized_scripts_at_the_shared_ceiling() {
        assert!(!legacy_script_too_large(
            &"a".repeat(MAX_LEGACY_SCRIPT_BYTES)
        ));
        assert!(legacy_script_too_large(
            &"a".repeat(MAX_LEGACY_SCRIPT_BYTES + 1)
        ));
        // And the command itself answers with the code the dialogs already speak.
        let oversized = "x".repeat(MAX_LEGACY_SCRIPT_BYTES + 1);
        let error = flow_import_legacy(oversized).expect_err("oversized script must be refused");
        assert!(
            format!("{error:?}").contains("FlowImportTooLarge"),
            "refusal must carry the shared code: {error:?}"
        );
    }

    #[test]
    fn flow_commands_catalog_exposes_only_release_one_actions() {
        let json = serde_json::to_value(flow_action_catalog()).expect("catalog JSON");
        let actions = json.as_array().expect("catalog array");
        assert!(actions
            .iter()
            .all(|action| action.get("disabledReason").is_some()));
        let encoded = serde_json::to_string(actions).expect("catalog encoding");
        assert!(encoded.contains("terminateApp"));
        assert!(encoded.contains("processAbsent"));
        assert!(!encoded.contains("rawHttp"));
        assert!(!encoded.contains("rawWda"));
        assert!(!encoded.contains("shell"));
        assert!(!encoded.to_ascii_lowercase().contains("token"));
    }

    #[test]
    fn flow_commands_validation_preserves_node_and_field_scope() {
        let mut document = FlowDocumentV2::empty("Fixture");
        let node_id = document.nodes[0].id;
        let node_id = node_id.to_string();
        document.nodes[0].position.x = f64::NAN;
        let errors = flow_validate(document).expect_err("non-finite position");
        assert!(errors.iter().any(|error| {
            error.code == "NonFiniteCoordinate"
                && error.node_id.as_deref() == Some(node_id.as_str())
                && error.field.as_deref() == Some("position")
        }));
    }

    #[test]
    fn flow_commands_assign_revision_before_compilation_and_hashing() {
        let mut document = FlowDocumentV2::empty("Save fixture");
        document.revision = 99;
        let (document, compiled) =
            compile_for_save(document, Some(4)).expect("compile revision five");
        assert_eq!(document.revision, 5);
        assert_eq!(compiled.plan.revision, 5);
        assert_eq!(
            compiled.sha256,
            riviu_core::compiled_plan_sha256(&compiled.plan).expect("compiled plan hash")
        );

        let overflow = compile_for_save(FlowDocumentV2::empty("Overflow"), Some(u64::MAX))
            .expect_err("revision overflow");
        assert_eq!(overflow.code, "InvalidArgument");
    }

    #[test]
    fn flow_commands_service_errors_keep_stable_codes_without_message_parsing() {
        let revision = map_service_error(
            RevisionConflict {
                expected: 3,
                actual: 4,
            }
            .into(),
        );
        assert_eq!(revision.code, "RevisionConflict");

        let not_found = map_service_error(
            FlowNotFound {
                flow_id: Uuid::nil(),
            }
            .into(),
        );
        assert_eq!(not_found.code, "FlowNotFound");

        let selection = map_service_error(FlowSelectionError::NoEligibleDevice.into());
        assert_eq!(selection.code, "NoEligibleDevice");

        let retry = map_service_error(FlowRetryError::NotAllowed { reason: "fixture" }.into());
        assert_eq!(retry.code, "RetryNotAllowed");

        let run_missing = map_service_error(
            FlowRuntimeError::RunNotFound {
                run_id: Uuid::nil(),
            }
            .into(),
        );
        assert_eq!(run_missing.code, "FlowRunNotFound");

        let attempt_missing = map_service_error(
            FlowRuntimeError::AttemptNotFound {
                attempt_id: Uuid::nil(),
            }
            .into(),
        );
        assert_eq!(attempt_missing.code, "FlowAttemptNotFound");

        let opaque = map_service_error(anyhow::anyhow!("C:\\private\\database.sqlite"));
        assert_eq!(opaque.code, "OperationFailed");
        assert!(!opaque.message.contains("database.sqlite"));
    }

    #[test]
    fn flow_commands_legacy_import_returns_typed_diagnostics() {
        let imported = flow_import_legacy(
            r#"{
                "version": 1,
                "name": "Legacy fixture",
                "steps": [{"action":"tap","point":{"x":10.0,"y":20.0}}]
            }"#
            .to_string(),
        )
        .expect("parse legacy script");
        assert!(imported.document.is_none());
        assert_eq!(imported.diagnostics.len(), 1);
        assert_eq!(imported.diagnostics[0].code, "GeometryRequired");
        assert_eq!(imported.diagnostics[0].field.as_deref(), Some("point"));
    }

    #[test]
    fn flow_commands_artifact_payload_contains_only_verified_public_fields() {
        let run_id = Uuid::new_v4();
        let device_run_id = Uuid::new_v4();
        let attempt_id = Uuid::new_v4();
        let artifact_id = Uuid::new_v4();
        let relative = Path::new(&run_id.to_string())
            .join(device_run_id.to_string())
            .join(attempt_id.to_string())
            .join(format!("{artifact_id}.jpeg"));
        let decoded = decode_and_hash_artifact(FIXTURE_JPEG).expect("decode fixture");
        let record = FlowArtifactRecord {
            id: artifact_id,
            attempt_id,
            relative_path: relative.to_string_lossy().to_string(),
            label: "fixture".to_string(),
            kind: "jpeg".to_string(),
            size: decoded.size,
            sha256: decoded.sha256,
            created_at: chrono::Utc::now(),
        };

        let payload = artifact_payload(record.clone(), FIXTURE_JPEG.to_vec())
            .expect("verified artifact payload");
        assert_eq!(payload.artifact_id, artifact_id);
        assert_eq!(
            BASE64
                .decode(payload.base64.as_bytes())
                .expect("payload bytes"),
            FIXTURE_JPEG
        );
        let json = serde_json::to_value(&payload).expect("payload JSON");
        assert_eq!(
            json.as_object()
                .expect("payload object")
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([
                "artifactId",
                "base64",
                "kind",
                "label",
                "sha256",
                "size",
            ])
        );

        assert_eq!(
            artifact_payload(record, b"tampered".to_vec())
                .expect_err("unverified payload")
                .code,
            "ArtifactIntegrityFailed"
        );
    }
}
