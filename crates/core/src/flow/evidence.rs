use std::time::Duration;

use base64::Engine;
use image::{DynamicImage, ImageFormat, RgbImage};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{EvidenceKind, EvidenceSpec, FlowCancellation};
use crate::{
    ui_error_kind, AppProcessState, GenerationFrame, GenerationFrameEvent, GenerationFrameSource,
    ProcessAbsenceProof, UiErrorKind, UiSession,
};

#[derive(Debug, Clone, PartialEq)]
pub enum EvidenceBaseline {
    None,
    Process {
        bundle_id: String,
        pid: Option<u64>,
    },
    Frame {
        generation: u64,
        jpeg_sha256: String,
        image: RgbImage,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum EvidenceBaselineWire {
    None,
    Process {
        bundle_id: String,
        pid: Option<u64>,
    },
    Frame {
        generation: u64,
        jpeg_sha256: String,
        image_width: u32,
        image_height: u32,
        rgb_base64: String,
    },
}

impl Serialize for EvidenceBaseline {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = match self {
            Self::None => EvidenceBaselineWire::None,
            Self::Process { bundle_id, pid } => EvidenceBaselineWire::Process {
                bundle_id: bundle_id.clone(),
                pid: *pid,
            },
            Self::Frame {
                generation,
                jpeg_sha256,
                image,
            } => EvidenceBaselineWire::Frame {
                generation: *generation,
                jpeg_sha256: jpeg_sha256.clone(),
                image_width: image.width(),
                image_height: image.height(),
                rgb_base64: base64::engine::general_purpose::STANDARD.encode(image.as_raw()),
            },
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EvidenceBaseline {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = EvidenceBaselineWire::deserialize(deserializer)?;
        match wire {
            EvidenceBaselineWire::None => Ok(Self::None),
            EvidenceBaselineWire::Process { bundle_id, pid } => {
                if bundle_id.trim().is_empty() || bundle_id.trim() != bundle_id {
                    return Err(serde::de::Error::custom("invalid process bundle id"));
                }
                if pid == Some(0) {
                    return Err(serde::de::Error::custom("invalid process PID"));
                }
                Ok(Self::Process { bundle_id, pid })
            }
            EvidenceBaselineWire::Frame {
                generation,
                jpeg_sha256,
                image_width,
                image_height,
                rgb_base64,
            } => {
                if !is_lower_sha256(&jpeg_sha256) || image_width == 0 || image_height == 0 {
                    return Err(serde::de::Error::custom("invalid frame identity"));
                }
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(rgb_base64)
                    .map_err(serde::de::Error::custom)?;
                let expected_length = u64::from(image_width)
                    .checked_mul(u64::from(image_height))
                    .and_then(|pixels| pixels.checked_mul(3))
                    .and_then(|length| usize::try_from(length).ok())
                    .ok_or_else(|| serde::de::Error::custom("frame dimensions overflow"))?;
                if bytes.len() != expected_length {
                    return Err(serde::de::Error::custom("invalid frame RGB length"));
                }
                let image = RgbImage::from_raw(image_width, image_height, bytes)
                    .ok_or_else(|| serde::de::Error::custom("invalid frame RGB length"))?;
                Ok(Self::Frame {
                    generation,
                    jpeg_sha256,
                    image,
                })
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceResult {
    pub kind: EvidenceKind,
    pub matched: bool,
    pub observed_sha256: String,
    pub measurement: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DecodedArtifactEvidence {
    pub sha256: String,
    pub format: String,
    pub size: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum EvidenceError {
    #[error("frame generation changed")]
    StaleGeneration,
    #[error("frame stream closed")]
    StreamClosed,
    #[error("evidence deadline expired")]
    Timeout,
    #[error("evidence verification was cancelled")]
    Cancelled,
    #[error("evidence did not match")]
    Mismatch,
    #[error("evidence capability is unavailable: {0}")]
    Unsupported(&'static str),
    #[error("evidence input is invalid: {0}")]
    Invalid(String),
}

impl EvidenceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::StaleGeneration => "StaleGeneration",
            Self::StreamClosed => "StreamClosed",
            Self::Timeout => "EvidenceTimeout",
            Self::Cancelled => "Cancelled",
            Self::Mismatch => "EvidenceMismatch",
            Self::Unsupported(_) => "EvidenceUnsupported",
            Self::Invalid(_) => "EvidenceInvalid",
        }
    }
}

pub(crate) async fn capture_baseline(
    source: &dyn GenerationFrameSource,
    udid: &str,
    generation: u64,
    specification: &EvidenceSpec,
    deadline: tokio::time::Instant,
    cancellation: &FlowCancellation,
) -> Result<EvidenceBaseline, EvidenceError> {
    check_boundary(deadline, cancellation)?;
    match specification {
        EvidenceSpec::ActiveAppEquals { bundle_id } => {
            validate_bundle(bundle_id)?;
            Ok(EvidenceBaseline::None)
        }
        EvidenceSpec::AccessibilityVisible { accessibility_id } => {
            validate_locator_value(accessibility_id, "accessibility id")?;
            Ok(EvidenceBaseline::None)
        }
        EvidenceSpec::ProcessAbsent { .. } => {
            Err(EvidenceError::Unsupported("processBaselineRequiresState"))
        }
        EvidenceSpec::FrameDigestChanged { .. }
        | EvidenceSpec::FrameRegionChanged { .. }
        | EvidenceSpec::QualifiedFramePredicate { .. }
        | EvidenceSpec::TextReadBackEquals { .. }
        | EvidenceSpec::ArtifactDecodedAndHashed => {
            let frame =
                next_qualified_frame(source, udid, generation, deadline, cancellation).await?;
            let image = decode_image(&frame.bytes)?;
            ensure_generation_current(source, udid, generation)?;
            if let EvidenceSpec::FrameRegionChanged {
                x,
                y,
                width,
                height,
                ..
            } = specification
            {
                validate_region(&image, *x, *y, *width, *height)?;
            }
            check_boundary(deadline, cancellation)?;
            Ok(EvidenceBaseline::Frame {
                generation,
                jpeg_sha256: sha256_hex(&frame.bytes),
                image,
            })
        }
    }
}

pub fn capture_process_baseline(
    specification: &EvidenceSpec,
    state: &AppProcessState,
) -> Result<EvidenceBaseline, EvidenceError> {
    let EvidenceSpec::ProcessAbsent { bundle_id } = specification else {
        return Err(EvidenceError::Invalid(
            "process state supplied for non-process evidence".into(),
        ));
    };
    validate_bundle(bundle_id)?;
    if state.bundle_id != *bundle_id || state.running != state.pid.is_some() || state.pid == Some(0)
    {
        return Err(EvidenceError::Mismatch);
    }
    Ok(EvidenceBaseline::Process {
        bundle_id: bundle_id.clone(),
        pid: state.pid,
    })
}

pub fn evaluate_process_absence(
    specification: &EvidenceSpec,
    baseline: &EvidenceBaseline,
    proof: &ProcessAbsenceProof,
) -> Result<EvidenceResult, EvidenceError> {
    let EvidenceSpec::ProcessAbsent { bundle_id } = specification else {
        return Err(EvidenceError::Invalid(
            "process proof supplied for non-process evidence".into(),
        ));
    };
    let EvidenceBaseline::Process {
        bundle_id: baseline_bundle,
        pid,
    } = baseline
    else {
        return Err(EvidenceError::Invalid(
            "process baseline is required".into(),
        ));
    };
    validate_bundle(bundle_id)?;
    if baseline_bundle != bundle_id || proof.bundle_id != *bundle_id || proof.old_pid != *pid {
        return Err(EvidenceError::Invalid(
            "process absence proof does not match the pre-effect identity".into(),
        ));
    }
    Ok(EvidenceResult {
        kind: EvidenceKind::ProcessAbsent,
        matched: true,
        observed_sha256: sha256_json(&json!({
            "bundleId": proof.bundle_id,
            "oldPid": proof.old_pid,
            "running": false,
        })),
        measurement: json!({
            "bundleId": bundle_id,
            "oldPid": pid,
            "running": false,
        }),
    })
}

pub fn verify_process_absence(
    specification: &EvidenceSpec,
    baseline: &EvidenceBaseline,
    proof: &ProcessAbsenceProof,
) -> Result<EvidenceResult, EvidenceError> {
    require_match(evaluate_process_absence(specification, baseline, proof)?)
}

pub fn evaluate_process_state(
    specification: &EvidenceSpec,
    baseline: &EvidenceBaseline,
    state: &AppProcessState,
) -> Result<EvidenceResult, EvidenceError> {
    let EvidenceSpec::ProcessAbsent { bundle_id } = specification else {
        return Err(EvidenceError::Invalid(
            "process state supplied for non-process evidence".into(),
        ));
    };
    let EvidenceBaseline::Process {
        bundle_id: baseline_bundle,
        pid: old_pid,
    } = baseline
    else {
        return Err(EvidenceError::Invalid(
            "process baseline is required".into(),
        ));
    };
    if baseline_bundle != bundle_id
        || state.bundle_id != *bundle_id
        || state.running != state.pid.is_some()
        || state.pid == Some(0)
    {
        return Err(EvidenceError::Mismatch);
    }
    if state.running && state.pid != *old_pid {
        return Err(EvidenceError::Invalid(
            "process PID changed after dispatch".into(),
        ));
    }
    let matched = !state.running;
    let measurement = if matched {
        json!({
            "bundleId": bundle_id,
            "oldPid": old_pid,
            "running": false,
        })
    } else {
        json!({
            "bundleId": bundle_id,
            "pid": state.pid,
            "preEffectPid": old_pid,
        })
    };
    Ok(EvidenceResult {
        kind: EvidenceKind::ProcessAbsent,
        matched,
        observed_sha256: sha256_json(&json!({
            "bundleId": state.bundle_id,
            "pid": state.pid,
            "running": state.running,
        })),
        measurement,
    })
}

pub fn decode_and_hash_artifact(bytes: &[u8]) -> Result<DecodedArtifactEvidence, EvidenceError> {
    if bytes.is_empty() {
        return Err(EvidenceError::Invalid("artifact is empty".into()));
    }
    let image_format = image::guess_format(bytes)
        .map_err(|error| EvidenceError::Invalid(format!("artifact format: {error}")))?;
    let format = match image_format {
        ImageFormat::Jpeg => "jpeg",
        ImageFormat::Png => "png",
        _ => {
            return Err(EvidenceError::Invalid(
                "artifact must be jpeg or png".into(),
            ));
        }
    };
    let image = image::load_from_memory_with_format(bytes, image_format)
        .map_err(|error| EvidenceError::Invalid(format!("artifact decode: {error}")))?;
    Ok(DecodedArtifactEvidence {
        sha256: sha256_hex(bytes),
        format: format.to_string(),
        size: u64::try_from(bytes.len())
            .map_err(|_| EvidenceError::Invalid("artifact size overflow".into()))?,
        width: image.width(),
        height: image.height(),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn evaluate_postcondition(
    source: &dyn GenerationFrameSource,
    session: Option<&dyn UiSession>,
    udid: &str,
    generation: u64,
    specification: &EvidenceSpec,
    baseline: &EvidenceBaseline,
    deadline: tokio::time::Instant,
    cancellation: &FlowCancellation,
) -> Result<EvidenceResult, EvidenceError> {
    check_boundary(deadline, cancellation)?;
    let result = match specification {
        EvidenceSpec::ActiveAppEquals { bundle_id } => {
            require_none_baseline(baseline)?;
            validate_bundle(bundle_id)?;
            let session = require_session(session)?;
            let observed = session.active_app_bundle().await;
            check_boundary(deadline, cancellation)?;
            let observed = observed.map_err(|error| map_device_read_error("active app", error))?;
            EvidenceResult {
                kind: EvidenceKind::ActiveAppEquals,
                matched: observed == *bundle_id,
                observed_sha256: sha256_hex(observed.as_bytes()),
                measurement: json!({"bundleId": observed}),
            }
        }
        EvidenceSpec::ProcessAbsent { .. } => {
            return Err(EvidenceError::Unsupported("processProofRequired"));
        }
        EvidenceSpec::FrameDigestChanged { minimum_distance } => {
            let (baseline_generation, baseline_sha, baseline_image) = frame_baseline(baseline)?;
            require_generation(generation, baseline_generation)?;
            let observed =
                next_post_frame(source, udid, generation, deadline, cancellation).await?;
            let observed_image = decode_image(&observed.bytes)?;
            ensure_generation_current(source, udid, generation)?;
            let distance = mean_luma_delta(baseline_image, &observed_image, None)?;
            let observed_sha256 = sha256_hex(&observed.bytes);
            EvidenceResult {
                kind: EvidenceKind::FrameDigestChanged,
                matched: distance >= *minimum_distance && observed_sha256 != baseline_sha,
                observed_sha256,
                measurement: json!({
                    "generation": generation,
                    "baselineSha256": baseline_sha,
                    "distance": distance,
                }),
            }
        }
        EvidenceSpec::FrameRegionChanged {
            x,
            y,
            width,
            height,
            minimum_distance,
        } => {
            let (baseline_generation, baseline_sha, baseline_image) = frame_baseline(baseline)?;
            require_generation(generation, baseline_generation)?;
            validate_region(baseline_image, *x, *y, *width, *height)?;
            let observed =
                next_post_frame(source, udid, generation, deadline, cancellation).await?;
            let observed_image = decode_image(&observed.bytes)?;
            ensure_generation_current(source, udid, generation)?;
            validate_region(&observed_image, *x, *y, *width, *height)?;
            let distance = mean_luma_delta(
                baseline_image,
                &observed_image,
                Some((*x, *y, *width, *height)),
            )?;
            let observed_sha256 = sha256_hex(&observed.bytes);
            EvidenceResult {
                kind: EvidenceKind::FrameRegionChanged,
                matched: distance >= *minimum_distance && observed_sha256 != baseline_sha,
                observed_sha256,
                measurement: json!({
                    "generation": generation,
                    "baselineSha256": baseline_sha,
                    "x": x,
                    "y": y,
                    "width": width,
                    "height": height,
                    "distance": distance,
                }),
            }
        }
        EvidenceSpec::QualifiedFramePredicate { .. } => {
            return Err(EvidenceError::Unsupported("qualifiedFramePredicate"));
        }
        EvidenceSpec::AccessibilityVisible { accessibility_id } => {
            validate_locator_value(accessibility_id, "accessibility id")?;
            let session = require_accessibility_session(session)?;
            let visibility = session.assert_visible(accessibility_id).await;
            check_boundary(deadline, cancellation)?;
            let visible = match visibility {
                Ok(()) => true,
                Err(error) if ui_error_kind(&error) == UiErrorKind::Http => false,
                Err(error) => {
                    return Err(map_device_read_error("accessibility visibility", error));
                }
            };
            let (generation_value, baseline_sha) = optional_frame_binding(baseline)?;
            let measurement = if let Some((generation_value, baseline_sha)) =
                generation_value.zip(baseline_sha)
            {
                json!({
                    "generation": generation_value,
                    "baselineSha256": baseline_sha,
                    "accessibilityId": accessibility_id,
                    "visible": visible,
                })
            } else {
                json!({"accessibilityId": accessibility_id, "visible": visible})
            };
            EvidenceResult {
                kind: EvidenceKind::AccessibilityVisible,
                matched: visible,
                observed_sha256: sha256_json(&measurement),
                measurement,
            }
        }
        EvidenceSpec::TextReadBackEquals { locator, value } => {
            validate_qualified_locator(locator)?;
            let session = require_accessibility_session(session)?;
            let (baseline_generation, baseline_sha, _) = frame_baseline(baseline)?;
            require_generation(generation, baseline_generation)?;
            let request_timeout = remaining(deadline, cancellation)?;
            let observed = session.read_text(locator, request_timeout).await;
            check_boundary(deadline, cancellation)?;
            let observed =
                observed.map_err(|error| map_device_read_error("text read-back", error))?;
            ensure_generation_current(source, udid, generation)?;
            EvidenceResult {
                kind: EvidenceKind::TextReadBackEquals,
                matched: observed == *value,
                observed_sha256: sha256_hex(observed.as_bytes()),
                measurement: json!({
                    "generation": generation,
                    "baselineSha256": baseline_sha,
                    "locator": locator,
                    "value": observed,
                }),
            }
        }
        EvidenceSpec::ArtifactDecodedAndHashed => {
            return Err(EvidenceError::Unsupported("artifactBytesRequired"));
        }
    };
    check_boundary(deadline, cancellation)?;
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn verify_postcondition(
    source: &dyn GenerationFrameSource,
    session: Option<&dyn UiSession>,
    udid: &str,
    generation: u64,
    specification: &EvidenceSpec,
    baseline: &EvidenceBaseline,
    deadline: tokio::time::Instant,
    cancellation: &FlowCancellation,
) -> Result<EvidenceResult, EvidenceError> {
    require_match(
        evaluate_postcondition(
            source,
            session,
            udid,
            generation,
            specification,
            baseline,
            deadline,
            cancellation,
        )
        .await?,
    )
}

fn require_match(result: EvidenceResult) -> Result<EvidenceResult, EvidenceError> {
    if result.matched {
        Ok(result)
    } else {
        Err(EvidenceError::Mismatch)
    }
}

fn require_session(session: Option<&dyn UiSession>) -> Result<&dyn UiSession, EvidenceError> {
    session.ok_or(EvidenceError::Unsupported("uiSession"))
}

fn require_accessibility_session(
    session: Option<&dyn UiSession>,
) -> Result<&dyn UiSession, EvidenceError> {
    let session = require_session(session)?;
    if session.supports_accessibility_readback() {
        Ok(session)
    } else {
        Err(EvidenceError::Unsupported("accessibilityReadback"))
    }
}

fn map_device_read_error(operation: &str, error: anyhow::Error) -> EvidenceError {
    if ui_error_kind(&error) == UiErrorKind::Timeout {
        EvidenceError::Timeout
    } else {
        EvidenceError::Invalid(format!("{operation} read: {error}"))
    }
}

fn check_boundary(
    deadline: tokio::time::Instant,
    cancellation: &FlowCancellation,
) -> Result<(), EvidenceError> {
    if cancellation.is_cancelled() {
        Err(EvidenceError::Cancelled)
    } else if tokio::time::Instant::now() >= deadline {
        Err(EvidenceError::Timeout)
    } else {
        Ok(())
    }
}

fn remaining(
    deadline: tokio::time::Instant,
    cancellation: &FlowCancellation,
) -> Result<Duration, EvidenceError> {
    check_boundary(deadline, cancellation)?;
    Ok(deadline.saturating_duration_since(tokio::time::Instant::now()))
}

async fn next_qualified_frame(
    source: &dyn GenerationFrameSource,
    udid: &str,
    generation: u64,
    deadline: tokio::time::Instant,
    cancellation: &FlowCancellation,
) -> Result<GenerationFrame, EvidenceError> {
    check_boundary(deadline, cancellation)?;
    let mut stream = source.subscribe_generation(udid, generation);
    if let Some(frame) = source.latest_in_generation(udid, generation) {
        return Ok(frame);
    }
    wait_frame_event(stream.as_mut(), generation, None, deadline, cancellation).await
}

async fn next_post_frame(
    source: &dyn GenerationFrameSource,
    udid: &str,
    generation: u64,
    deadline: tokio::time::Instant,
    cancellation: &FlowCancellation,
) -> Result<GenerationFrame, EvidenceError> {
    check_boundary(deadline, cancellation)?;
    let mut stream = source.subscribe_generation(udid, generation);
    let watermark = source
        .latest_in_generation(udid, generation)
        .ok_or(EvidenceError::StaleGeneration)?
        .sequence;
    wait_frame_event(
        stream.as_mut(),
        generation,
        Some(watermark),
        deadline,
        cancellation,
    )
    .await
}

async fn wait_frame_event(
    stream: &mut dyn crate::GenerationFrameStream,
    generation: u64,
    after_sequence: Option<u64>,
    deadline: tokio::time::Instant,
    cancellation: &FlowCancellation,
) -> Result<GenerationFrame, EvidenceError> {
    loop {
        let event = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(EvidenceError::Cancelled),
            _ = tokio::time::sleep_until(deadline) => return Err(EvidenceError::Timeout),
            event = stream.next() => event,
        };
        match event {
            GenerationFrameEvent::Frame(frame)
                if frame.generation == generation
                    && after_sequence.is_none_or(|watermark| frame.sequence > watermark) =>
            {
                return Ok(frame);
            }
            GenerationFrameEvent::Frame(frame) if frame.generation == generation => continue,
            GenerationFrameEvent::Frame(_) | GenerationFrameEvent::Advanced { .. } => {
                return Err(EvidenceError::StaleGeneration);
            }
            GenerationFrameEvent::Closed => return Err(EvidenceError::StreamClosed),
        }
    }
}

fn frame_baseline(baseline: &EvidenceBaseline) -> Result<(u64, &str, &RgbImage), EvidenceError> {
    let EvidenceBaseline::Frame {
        generation,
        jpeg_sha256,
        image,
    } = baseline
    else {
        return Err(EvidenceError::Invalid("frame baseline is required".into()));
    };
    if !is_lower_sha256(jpeg_sha256) || image.width() == 0 || image.height() == 0 {
        return Err(EvidenceError::Invalid("frame baseline is invalid".into()));
    }
    Ok((*generation, jpeg_sha256, image))
}

fn optional_frame_binding(
    baseline: &EvidenceBaseline,
) -> Result<(Option<u64>, Option<&str>), EvidenceError> {
    match baseline {
        EvidenceBaseline::None => Ok((None, None)),
        EvidenceBaseline::Frame {
            generation,
            jpeg_sha256,
            image,
        } if is_lower_sha256(jpeg_sha256) && image.width() > 0 && image.height() > 0 => {
            Ok((Some(*generation), Some(jpeg_sha256)))
        }
        _ => Err(EvidenceError::Invalid(
            "accessibility baseline is invalid".into(),
        )),
    }
}

fn require_generation(expected: u64, actual: u64) -> Result<(), EvidenceError> {
    if expected == actual {
        Ok(())
    } else {
        Err(EvidenceError::StaleGeneration)
    }
}

fn ensure_generation_current(
    source: &dyn GenerationFrameSource,
    udid: &str,
    generation: u64,
) -> Result<(), EvidenceError> {
    source
        .latest_in_generation(udid, generation)
        .map(|_| ())
        .ok_or(EvidenceError::StaleGeneration)
}

fn decode_image(bytes: &[u8]) -> Result<RgbImage, EvidenceError> {
    image::load_from_memory(bytes)
        .map(DynamicImage::into_rgb8)
        .map_err(|error| EvidenceError::Invalid(format!("frame decode: {error}")))
}

fn validate_region(
    image: &RgbImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<(), EvidenceError> {
    if width == 0
        || height == 0
        || x.checked_add(width)
            .is_none_or(|right| right > image.width())
        || y.checked_add(height)
            .is_none_or(|bottom| bottom > image.height())
    {
        return Err(EvidenceError::Invalid(
            "frame region is outside the decoded image".into(),
        ));
    }
    Ok(())
}

fn mean_luma_delta(
    before: &RgbImage,
    after: &RgbImage,
    region: Option<(u32, u32, u32, u32)>,
) -> Result<u32, EvidenceError> {
    if before.dimensions() != after.dimensions() {
        return Err(EvidenceError::Invalid(
            "frame dimensions changed during verification".into(),
        ));
    }
    let (x, y, width, height) = region.unwrap_or((0, 0, before.width(), before.height()));
    validate_region(before, x, y, width, height)?;
    let mut total = 0_u64;
    for row in y..y + height {
        for column in x..x + width {
            let a = luma(before.get_pixel(column, row).0);
            let b = luma(after.get_pixel(column, row).0);
            total += u64::from(a.abs_diff(b));
        }
    }
    let count = u64::from(width) * u64::from(height);
    u32::try_from(total / count)
        .map_err(|_| EvidenceError::Invalid("frame distance overflow".into()))
}

fn luma(pixel: [u8; 3]) -> u8 {
    let weighted = u32::from(pixel[0]) * 77 + u32::from(pixel[1]) * 150 + u32::from(pixel[2]) * 29;
    (weighted >> 8) as u8
}

fn validate_bundle(bundle_id: &str) -> Result<(), EvidenceError> {
    if bundle_id.trim().is_empty() || bundle_id.trim() != bundle_id {
        Err(EvidenceError::Invalid("bundle id is invalid".into()))
    } else {
        Ok(())
    }
}

fn require_none_baseline(baseline: &EvidenceBaseline) -> Result<(), EvidenceError> {
    if matches!(baseline, EvidenceBaseline::None) {
        Ok(())
    } else {
        Err(EvidenceError::Invalid("none baseline is required".into()))
    }
}

fn validate_locator_value(value: &str, label: &str) -> Result<(), EvidenceError> {
    if value.trim().is_empty() || value.trim() != value {
        Err(EvidenceError::Invalid(format!("{label} is invalid")))
    } else {
        Ok(())
    }
}

fn validate_qualified_locator(
    locator: &super::QualifiedElementLocator,
) -> Result<(), EvidenceError> {
    validate_locator_value(&locator.value, "qualified locator")
}

fn sha256_json(value: &serde_json::Value) -> String {
    sha256_hex(&serde_json::to_vec(value).expect("JSON value serialization cannot fail"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use parking_lot::Mutex;

    use super::super::{
        ElementLocatorStrategy, EvidenceKind, EvidenceSpec, QualifiedElementLocator,
    };
    use super::*;
    use crate::{
        AppProcessState, Frame, FrameSource, FrameStream, GenerationFrame, GenerationFrameEvent,
        GenerationFrameSource, GenerationFrameStream, ProcessAbsenceProof, SwipeGesture, TapPoint,
        UiSession,
    };

    const BASELINE_JPEG: &[u8] = include_bytes!("../../tests/fixtures/feed-iphone8.jpg");
    const CHANGED_JPEG: &[u8] = include_bytes!("../../tests/fixtures/feed-mid-swipe.jpg");

    #[derive(Clone)]
    struct TestGenerationFrames {
        state: Arc<Mutex<TestFrameState>>,
        notify: Arc<tokio::sync::Notify>,
    }

    struct TestFrameState {
        generation: u64,
        sequence: u64,
        latest: Option<Frame>,
        events: VecDeque<GenerationFrameEvent>,
        closed: bool,
        advance_after_latest: Option<u64>,
    }

    impl TestGenerationFrames {
        fn single_generation(generation: u64, bytes: &[u8]) -> Self {
            Self {
                state: Arc::new(Mutex::new(TestFrameState {
                    generation,
                    sequence: 1,
                    latest: Some(Arc::new(bytes.to_vec())),
                    events: VecDeque::new(),
                    closed: false,
                    advance_after_latest: None,
                })),
                notify: Arc::new(tokio::sync::Notify::new()),
            }
        }

        fn empty(generation: u64) -> Self {
            Self {
                state: Arc::new(Mutex::new(TestFrameState {
                    generation,
                    sequence: 0,
                    latest: None,
                    events: VecDeque::new(),
                    closed: false,
                    advance_after_latest: None,
                })),
                notify: Arc::new(tokio::sync::Notify::new()),
            }
        }

        fn publish(&self, generation: u64, bytes: &[u8]) {
            let frame = Arc::new(bytes.to_vec());
            let mut state = self.state.lock();
            state.sequence += 1;
            let sequence = state.sequence;
            state.latest = Some(frame.clone());
            state
                .events
                .push_back(GenerationFrameEvent::Frame(GenerationFrame {
                    generation,
                    sequence,
                    bytes: frame,
                }));
            drop(state);
            self.notify.notify_waiters();
        }

        fn publish_after_subscription(&self, generation: u64, bytes: &'static [u8]) {
            let frames = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(5)).await;
                frames.publish(generation, bytes);
            });
        }

        fn advance(&self, actual: u64) {
            let mut state = self.state.lock();
            let expected = state.generation;
            state.generation = actual;
            state.sequence = 0;
            state.latest = None;
            state
                .events
                .push_back(GenerationFrameEvent::Advanced { expected, actual });
            drop(state);
            self.notify.notify_waiters();
        }

        fn close(&self) {
            self.state.lock().closed = true;
            self.notify.notify_waiters();
        }

        fn advance_after_latest(&self, actual: u64) {
            self.state.lock().advance_after_latest = Some(actual);
        }
    }

    struct TestGenerationStream {
        expected: u64,
        after_sequence: u64,
        state: Arc<Mutex<TestFrameState>>,
        notify: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl GenerationFrameStream for TestGenerationStream {
        async fn next(&mut self) -> GenerationFrameEvent {
            loop {
                let notified = self.notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                {
                    let mut state = self.state.lock();
                    if state.generation != self.expected {
                        return GenerationFrameEvent::Advanced {
                            expected: self.expected,
                            actual: state.generation,
                        };
                    }
                    if let Some(event) = state.events.pop_front() {
                        if matches!(
                            &event,
                            GenerationFrameEvent::Frame(frame)
                                if frame.sequence <= self.after_sequence
                        ) {
                            continue;
                        }
                        return event;
                    }
                    if state.closed {
                        return GenerationFrameEvent::Closed;
                    }
                }
                notified.await;
            }
        }
    }

    struct EmptyFrameStream;

    #[async_trait]
    impl FrameStream for EmptyFrameStream {
        async fn next(&mut self) -> Option<Frame> {
            None
        }
    }

    impl FrameSource for TestGenerationFrames {
        fn subscribe(&self, _udid: &str) -> Box<dyn FrameStream> {
            Box::new(EmptyFrameStream)
        }

        fn latest(&self, _udid: &str) -> Option<Frame> {
            self.state.lock().latest.clone()
        }
    }

    impl GenerationFrameSource for TestGenerationFrames {
        fn subscribe_generation(
            &self,
            _udid: &str,
            generation: u64,
        ) -> Box<dyn GenerationFrameStream> {
            let after_sequence = self.state.lock().sequence;
            Box::new(TestGenerationStream {
                expected: generation,
                after_sequence,
                state: self.state.clone(),
                notify: self.notify.clone(),
            })
        }

        fn latest_in_generation(&self, _udid: &str, generation: u64) -> Option<GenerationFrame> {
            let mut state = self.state.lock();
            let frame = (state.generation == generation)
                .then(|| state.latest.clone())
                .flatten()
                .map(|bytes| GenerationFrame {
                    generation,
                    sequence: state.sequence,
                    bytes,
                });
            if frame.is_some() {
                if let Some(actual) = state.advance_after_latest.take() {
                    let expected = state.generation;
                    state.generation = actual;
                    state.sequence = 0;
                    state.latest = None;
                    state
                        .events
                        .push_back(GenerationFrameEvent::Advanced { expected, actual });
                }
            }
            frame
        }
    }

    struct ReadBackSession {
        active_bundle: String,
        visible: bool,
        text: String,
        supports_readback: bool,
    }

    #[async_trait]
    impl UiSession for ReadBackSession {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            Ok(())
        }

        async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            if self.visible {
                Ok(())
            } else {
                Err(crate::UiError::new(
                    crate::UiErrorKind::Http,
                    "element.find",
                    "fixture element is hidden",
                )
                .into())
            }
        }

        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(self.active_bundle.clone())
        }

        async fn read_text(
            &self,
            _locator: &crate::QualifiedElementLocator,
            _request_timeout: Duration,
        ) -> anyhow::Result<String> {
            Ok(self.text.clone())
        }

        fn supports_accessibility_readback(&self) -> bool {
            self.supports_readback
        }

        fn stream_url(&self) -> Option<String> {
            None
        }
    }

    enum RequestBoundaryBehavior {
        Cancel(FlowCancellation),
        Delay(Duration),
        Timeout,
    }

    struct FailingReadSession {
        behavior: RequestBoundaryBehavior,
    }

    #[async_trait]
    impl UiSession for FailingReadSession {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            Ok(())
        }

        async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            anyhow::bail!("fixture request failed")
        }

        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            match &self.behavior {
                RequestBoundaryBehavior::Cancel(cancellation) => cancellation.cancel(),
                RequestBoundaryBehavior::Delay(duration) => tokio::time::sleep(*duration).await,
                RequestBoundaryBehavior::Timeout => {
                    return Err(crate::UiError::new(
                        crate::UiErrorKind::Timeout,
                        "fixture.read",
                        "request deadline expired",
                    )
                    .into());
                }
            }
            anyhow::bail!("fixture request failed")
        }

        fn stream_url(&self) -> Option<String> {
            None
        }
    }

    fn deadline() -> tokio::time::Instant {
        tokio::time::Instant::now() + Duration::from_secs(1)
    }

    fn region_spec() -> EvidenceSpec {
        EvidenceSpec::FrameRegionChanged {
            x: 0,
            y: 0,
            width: 750,
            height: 1334,
            minimum_distance: 1,
        }
    }

    #[tokio::test]
    async fn repository_frames_produce_qualified_region_evidence() {
        let frames = TestGenerationFrames::single_generation(7, BASELINE_JPEG);
        let cancellation = FlowCancellation::default();
        let baseline = capture_baseline(
            &frames,
            "fixture",
            7,
            &region_spec(),
            deadline(),
            &cancellation,
        )
        .await
        .expect("baseline");
        frames.publish_after_subscription(7, CHANGED_JPEG);

        let result = verify_postcondition(
            &frames,
            None,
            "fixture",
            7,
            &region_spec(),
            &baseline,
            deadline(),
            &cancellation,
        )
        .await
        .expect("changed region");
        assert!(result.matched);
        assert_eq!(result.kind, EvidenceKind::FrameRegionChanged);
        assert_eq!(result.measurement["generation"], 7);
    }

    #[tokio::test]
    async fn full_frame_digest_uses_the_same_generation_bound_baseline() {
        let frames = TestGenerationFrames::single_generation(7, BASELINE_JPEG);
        let cancellation = FlowCancellation::default();
        let spec = EvidenceSpec::FrameDigestChanged {
            minimum_distance: 1,
        };
        let baseline = capture_baseline(&frames, "fixture", 7, &spec, deadline(), &cancellation)
            .await
            .expect("digest baseline");
        frames.publish_after_subscription(7, CHANGED_JPEG);

        let result = verify_postcondition(
            &frames,
            None,
            "fixture",
            7,
            &spec,
            &baseline,
            deadline(),
            &cancellation,
        )
        .await
        .expect("changed digest");

        assert_eq!(result.kind, EvidenceKind::FrameDigestChanged);
        assert!(result.measurement["distance"].as_u64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn gesture_ack_without_matching_frame_evidence_is_not_success() {
        let frames = TestGenerationFrames::single_generation(7, BASELINE_JPEG);
        let cancellation = FlowCancellation::default();
        let session = ReadBackSession {
            active_bundle: "com.fixture.app".into(),
            visible: true,
            text: String::new(),
            supports_readback: true,
        };
        let baseline = capture_baseline(
            &frames,
            "fixture",
            7,
            &region_spec(),
            deadline(),
            &cancellation,
        )
        .await
        .expect("baseline");
        session
            .tap(TapPoint { x: 10.0, y: 10.0 })
            .await
            .expect("injected WDA ACK");
        frames.publish_after_subscription(7, BASELINE_JPEG);

        let error = verify_postcondition(
            &frames,
            Some(&session),
            "fixture",
            7,
            &region_spec(),
            &baseline,
            deadline(),
            &cancellation,
        )
        .await
        .expect_err("unchanged frame must fail");
        assert_eq!(error.code(), "EvidenceMismatch");
    }

    #[tokio::test]
    async fn frame_published_before_verification_is_not_post_dispatch_evidence() {
        let frames = TestGenerationFrames::single_generation(7, BASELINE_JPEG);
        let cancellation = FlowCancellation::default();
        let baseline = capture_baseline(
            &frames,
            "fixture",
            7,
            &region_spec(),
            deadline(),
            &cancellation,
        )
        .await
        .expect("baseline");

        frames.publish(7, CHANGED_JPEG);
        let error = tokio::time::timeout(
            Duration::from_millis(250),
            verify_postcondition(
                &frames,
                None,
                "fixture",
                7,
                &region_spec(),
                &baseline,
                tokio::time::Instant::now() + Duration::from_millis(150),
                &cancellation,
            ),
        )
        .await
        .expect("verification remains bounded")
        .expect_err("a pre-verification frame is not causal evidence");
        assert_eq!(error.code(), "EvidenceTimeout");
    }

    #[tokio::test]
    async fn baseline_rejects_generation_advance_after_cached_frame_read() {
        let frames = TestGenerationFrames::single_generation(7, BASELINE_JPEG);
        frames.advance_after_latest(8);
        let error = capture_baseline(
            &frames,
            "fixture",
            7,
            &region_spec(),
            deadline(),
            &FlowCancellation::default(),
        )
        .await
        .expect_err("generation changed while decoding the baseline");
        assert_eq!(error.code(), "StaleGeneration");
    }

    #[tokio::test]
    async fn postcondition_waits_map_generation_close_deadline_and_cancel_exactly() {
        for case in ["advanced", "closed", "deadline", "cancelled"] {
            let frames = TestGenerationFrames::single_generation(7, BASELINE_JPEG);
            let cancellation = FlowCancellation::default();
            let baseline = capture_baseline(
                &frames,
                "fixture",
                7,
                &region_spec(),
                deadline(),
                &cancellation,
            )
            .await
            .expect("baseline");
            match case {
                "advanced" => frames.advance(8),
                "closed" => frames.close(),
                "cancelled" => cancellation.cancel(),
                _ => {}
            }
            let inner_deadline = if case == "deadline" {
                tokio::time::Instant::now() + Duration::from_millis(15)
            } else {
                deadline()
            };
            let error = tokio::time::timeout(
                Duration::from_millis(250),
                verify_postcondition(
                    &frames,
                    None,
                    "fixture",
                    7,
                    &region_spec(),
                    &baseline,
                    inner_deadline,
                    &cancellation,
                ),
            )
            .await
            .expect("postcondition wait must be bounded")
            .expect_err("fixture exit must fail");
            let expected = match case {
                "advanced" => "StaleGeneration",
                "closed" => "StreamClosed",
                "deadline" => "EvidenceTimeout",
                "cancelled" => "Cancelled",
                _ => unreachable!(),
            };
            assert_eq!(error.code(), expected, "case={case}");
        }
    }

    #[tokio::test]
    async fn frame_waits_fail_deterministically_on_generation_close_deadline_and_cancel() {
        let cases = ["advanced", "closed", "deadline", "cancelled"];
        for case in cases {
            let frames = TestGenerationFrames::empty(7);
            let cancellation = FlowCancellation::default();
            match case {
                "advanced" => frames.advance(8),
                "closed" => frames.close(),
                "cancelled" => cancellation.cancel(),
                _ => {}
            }
            let inner_deadline = if case == "deadline" {
                tokio::time::Instant::now() + Duration::from_millis(15)
            } else {
                deadline()
            };
            let error = tokio::time::timeout(
                Duration::from_millis(250),
                capture_baseline(
                    &frames,
                    "fixture",
                    7,
                    &region_spec(),
                    inner_deadline,
                    &cancellation,
                ),
            )
            .await
            .expect("evidence wait must be bounded")
            .expect_err("fixture exit must fail");
            let expected = match case {
                "advanced" => "StaleGeneration",
                "closed" => "StreamClosed",
                "deadline" => "EvidenceTimeout",
                "cancelled" => "Cancelled",
                _ => unreachable!(),
            };
            assert_eq!(error.code(), expected, "case={case}");
        }
    }

    #[tokio::test]
    async fn active_app_accessibility_and_unicode_readback_are_exact() {
        let frames = TestGenerationFrames::single_generation(7, BASELINE_JPEG);
        let cancellation = FlowCancellation::default();
        let session = ReadBackSession {
            active_bundle: "com.fixture.app".into(),
            visible: true,
            text: "Tiếng Việt chính xác".into(),
            supports_readback: true,
        };

        let active = verify_postcondition(
            &frames,
            Some(&session),
            "fixture",
            7,
            &EvidenceSpec::ActiveAppEquals {
                bundle_id: "com.fixture.app".into(),
            },
            &EvidenceBaseline::None,
            deadline(),
            &cancellation,
        )
        .await
        .expect("active app");
        assert!(active.matched);

        let visible = verify_postcondition(
            &frames,
            Some(&session),
            "fixture",
            7,
            &EvidenceSpec::AccessibilityVisible {
                accessibility_id: "SearchField".into(),
            },
            &EvidenceBaseline::None,
            deadline(),
            &cancellation,
        )
        .await
        .expect("visible");
        assert!(visible.matched);

        let spec = EvidenceSpec::TextReadBackEquals {
            locator: QualifiedElementLocator {
                strategy: ElementLocatorStrategy::AccessibilityId,
                value: "SearchField".into(),
            },
            value: "Tiếng Việt chính xác".into(),
        };
        let baseline = capture_baseline(&frames, "fixture", 7, &spec, deadline(), &cancellation)
            .await
            .expect("text baseline");
        let text = verify_postcondition(
            &frames,
            Some(&session),
            "fixture",
            7,
            &spec,
            &baseline,
            deadline(),
            &cancellation,
        )
        .await
        .expect("text read-back");
        assert_eq!(text.measurement["value"], "Tiếng Việt chính xác");
    }

    #[tokio::test]
    async fn readback_is_rejected_without_the_live_session_capability() {
        let frames = TestGenerationFrames::single_generation(7, BASELINE_JPEG);
        let cancellation = FlowCancellation::default();
        let session = ReadBackSession {
            active_bundle: "com.fixture.app".into(),
            visible: true,
            text: "fixture".into(),
            supports_readback: false,
        };
        let error = verify_postcondition(
            &frames,
            Some(&session),
            "fixture",
            7,
            &EvidenceSpec::AccessibilityVisible {
                accessibility_id: "SearchField".into(),
            },
            &EvidenceBaseline::None,
            deadline(),
            &cancellation,
        )
        .await
        .expect_err("unsupported readback");
        assert_eq!(error.code(), "EvidenceUnsupported");
    }

    #[tokio::test]
    async fn request_errors_do_not_hide_cancellation_or_deadline() {
        let frames = TestGenerationFrames::single_generation(7, BASELINE_JPEG);

        let cancellation = FlowCancellation::default();
        let cancelled = FailingReadSession {
            behavior: RequestBoundaryBehavior::Cancel(cancellation.clone()),
        };
        let error = evaluate_postcondition(
            &frames,
            Some(&cancelled),
            "fixture",
            7,
            &EvidenceSpec::ActiveAppEquals {
                bundle_id: "com.fixture.app".into(),
            },
            &EvidenceBaseline::None,
            deadline(),
            &cancellation,
        )
        .await
        .expect_err("request-side cancellation");
        assert_eq!(error.code(), "Cancelled");

        let delayed = FailingReadSession {
            behavior: RequestBoundaryBehavior::Delay(Duration::from_millis(20)),
        };
        let error = tokio::time::timeout(
            Duration::from_millis(250),
            evaluate_postcondition(
                &frames,
                Some(&delayed),
                "fixture",
                7,
                &EvidenceSpec::ActiveAppEquals {
                    bundle_id: "com.fixture.app".into(),
                },
                &EvidenceBaseline::None,
                tokio::time::Instant::now() + Duration::from_millis(5),
                &FlowCancellation::default(),
            ),
        )
        .await
        .expect("deadline check remains bounded")
        .expect_err("request completed after verifier deadline");
        assert_eq!(error.code(), "EvidenceTimeout");

        let request_timeout = FailingReadSession {
            behavior: RequestBoundaryBehavior::Timeout,
        };
        let error = evaluate_postcondition(
            &frames,
            Some(&request_timeout),
            "fixture",
            7,
            &EvidenceSpec::ActiveAppEquals {
                bundle_id: "com.fixture.app".into(),
            },
            &EvidenceBaseline::None,
            deadline(),
            &FlowCancellation::default(),
        )
        .await
        .expect_err("request-local timeout");
        assert_eq!(error.code(), "EvidenceTimeout");
    }

    #[tokio::test]
    async fn successful_reads_with_wrong_values_are_evidence_mismatches() {
        let frames = TestGenerationFrames::single_generation(7, BASELINE_JPEG);
        let cancellation = FlowCancellation::default();
        let session = ReadBackSession {
            active_bundle: "com.other.app".into(),
            visible: false,
            text: "khác".into(),
            supports_readback: true,
        };
        let active_error = verify_postcondition(
            &frames,
            Some(&session),
            "fixture",
            7,
            &EvidenceSpec::ActiveAppEquals {
                bundle_id: "com.fixture.app".into(),
            },
            &EvidenceBaseline::None,
            deadline(),
            &cancellation,
        )
        .await
        .expect_err("wrong active app");
        assert_eq!(active_error.code(), "EvidenceMismatch");

        let visible_error = verify_postcondition(
            &frames,
            Some(&session),
            "fixture",
            7,
            &EvidenceSpec::AccessibilityVisible {
                accessibility_id: "SearchField".into(),
            },
            &EvidenceBaseline::None,
            deadline(),
            &cancellation,
        )
        .await
        .expect_err("hidden element");
        assert_eq!(visible_error.code(), "EvidenceMismatch");

        let text_spec = EvidenceSpec::TextReadBackEquals {
            locator: QualifiedElementLocator {
                strategy: ElementLocatorStrategy::AccessibilityId,
                value: "SearchField".into(),
            },
            value: "Tiếng Việt chính xác".into(),
        };
        let baseline =
            capture_baseline(&frames, "fixture", 7, &text_spec, deadline(), &cancellation)
                .await
                .expect("text baseline");
        let text_error = verify_postcondition(
            &frames,
            Some(&session),
            "fixture",
            7,
            &text_spec,
            &baseline,
            deadline(),
            &cancellation,
        )
        .await
        .expect_err("wrong text");
        assert_eq!(text_error.code(), "EvidenceMismatch");
    }

    #[test]
    fn process_absence_is_bound_to_the_exact_pre_effect_identity() {
        let spec = EvidenceSpec::ProcessAbsent {
            bundle_id: "com.fixture.app".into(),
        };
        let baseline = capture_process_baseline(
            &spec,
            &AppProcessState {
                bundle_id: "com.fixture.app".into(),
                pid: Some(42),
                running: true,
            },
        )
        .expect("process baseline");
        let result = verify_process_absence(
            &spec,
            &baseline,
            &ProcessAbsenceProof {
                bundle_id: "com.fixture.app".into(),
                old_pid: Some(42),
            },
        )
        .expect("process absence");
        assert!(result.matched);
        assert_eq!(result.measurement["oldPid"], 42);

        let error = evaluate_process_absence(
            &spec,
            &baseline,
            &ProcessAbsenceProof {
                bundle_id: "com.other.app".into(),
                old_pid: Some(42),
            },
        )
        .expect_err("a mismatched proof must not produce a false success envelope");
        assert_eq!(error.code(), "EvidenceInvalid");
    }

    #[test]
    fn process_reconciliation_distinguishes_absence_non_delivery_and_pid_replacement() {
        let spec = EvidenceSpec::ProcessAbsent {
            bundle_id: "com.fixture.app".into(),
        };
        let baseline = EvidenceBaseline::Process {
            bundle_id: "com.fixture.app".into(),
            pid: Some(42),
        };
        let absent = evaluate_process_state(
            &spec,
            &baseline,
            &AppProcessState {
                bundle_id: "com.fixture.app".into(),
                pid: None,
                running: false,
            },
        )
        .expect("absent process");
        assert!(absent.matched);

        let same = evaluate_process_state(
            &spec,
            &baseline,
            &AppProcessState {
                bundle_id: "com.fixture.app".into(),
                pid: Some(42),
                running: true,
            },
        )
        .expect("same process proves non-delivery");
        assert!(!same.matched);
        assert_eq!(
            same.measurement,
            json!({
                "bundleId": "com.fixture.app",
                "pid": 42,
                "preEffectPid": 42,
            })
        );

        let replaced = evaluate_process_state(
            &spec,
            &baseline,
            &AppProcessState {
                bundle_id: "com.fixture.app".into(),
                pid: Some(43),
                running: true,
            },
        )
        .expect_err("replacement PID is uncertain");
        assert_eq!(replaced.code(), "EvidenceInvalid");
    }

    #[test]
    fn evidence_baselines_round_trip_through_the_exact_persisted_schema() {
        let frame = decode_image(BASELINE_JPEG).expect("fixture image");
        let baseline = EvidenceBaseline::Frame {
            generation: 7,
            jpeg_sha256: sha256_hex(BASELINE_JPEG),
            image: frame,
        };
        let value = serde_json::to_value(&baseline).expect("serialize baseline");
        assert_eq!(value.as_object().expect("object").len(), 6);
        assert_eq!(value["kind"], "frame");
        assert_eq!(value["imageWidth"], 750);
        assert_eq!(value["imageHeight"], 1334);
        assert_eq!(
            serde_json::from_value::<EvidenceBaseline>(value).expect("round trip"),
            baseline
        );

        let mut extra = serde_json::to_value(&baseline).expect("serialize extra fixture");
        extra["extra"] = json!(true);
        assert!(serde_json::from_value::<EvidenceBaseline>(extra).is_err());

        let malformed = json!({
            "kind": "frame",
            "generation": 7,
            "jpegSha256": "a".repeat(64),
            "imageWidth": 1,
            "imageHeight": 1,
            "rgbBase64": base64::engine::general_purpose::STANDARD.encode([0_u8; 4]),
        });
        assert!(serde_json::from_value::<EvidenceBaseline>(malformed).is_err());
    }

    #[test]
    fn artifact_must_decode_and_hash_the_exact_bytes() {
        let artifact = decode_and_hash_artifact(BASELINE_JPEG).expect("valid jpeg artifact");
        assert_eq!(artifact.format, "jpeg");
        assert_eq!((artifact.width, artifact.height), (750, 1334));
        assert_eq!(artifact.size, BASELINE_JPEG.len() as u64);
        assert_eq!(artifact.sha256.len(), 64);
        assert!(decode_and_hash_artifact(b"not-an-image").is_err());
    }
}
