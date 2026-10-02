//! Existing-controller image rehearsal helper. No IPC registration, campaign, Post
//! intent, submission timestamp, verifier or Sheet; caller owns run registry/lease.
use anyhow::{ensure, Context};
use riviu_core::{DeviceControlPlane, PublishBundle, UiSession, UiWithStreamContext};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::atomic::AtomicBool};
use tokio::time::Instant;

pub(crate) struct ApprovedPublishInput {
    pub bundle: PublishBundle,
    pub fingerprint: String,
    pub expected_account: String,
    pub package: String,
    pub sound_policy: riviu_core::PublishSoundPolicy,
}
pub(crate) struct PublishRunHooks<'a> {
    pub request_id: &'a str,
    pub udid: &'a str,
    pub deadline: Instant,
    pub stop: &'a AtomicBool,
    pub report_dir: &'a Path,
    pub phase: &'a (dyn Fn(&str, &Value) -> anyhow::Result<()> + Send + Sync),
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MediaCleanup {
    NotStarted,
    Verified,
    NeedsAttention,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublishRehearsalOutcome {
    pub preparation: Option<riviu_core::tiktok_composer::rehearsal::PublishPreparationOutcome>,
    pub media_cleanup: MediaCleanup,
    pub cleanup_safe: bool,
    pub blocker: Option<&'static str>,
    pub import_id: Option<String>,
    pub failure_phase: Option<&'static str>,
    pub failure_code: Option<String>,
    /// Controlled classification only; never raw device/error-chain content.
    pub preflight_detail: Option<String>,
}
fn classified_preflight_detail(error: &anyhow::Error) -> String {
    const KNOWN: &[(&str, &str)] = &[
        ("no-public publish tap target missing/overlapped", "tap_target_missing_or_overlapped"),
        ("no-public navigation actionable overlap", "navigation_actionable_overlap"),
        ("no-public publish layout changed before tap", "layout_changed_before_tap"),
        ("no-public publish unknown intended role refused", "unknown_intended_role"),
        ("no-public swipe requires owned picker", "swipe_requires_owned_picker"),
        ("no-public Back screen unproved", "back_screen_unproved"),
        ("account_session_changed", "account_session_changed"),
        ("account_unreadable", "account_unreadable"),
    ];
    // Inspect fixed known phrases internally; output only static labels. Prefer the
    // specific navigation failure over an outer account_unreadable wrapper.
    for (phrase, label) in KNOWN {
        if error.chain().take(16).any(|cause| {
            let message = cause.to_string();
            message.len() <= 16 * 1024 && message.contains(phrase)
        }) {
            return (*label).to_owned();
        }
    }
    "unclassified_redacted".to_owned()
}

fn record_failure(outcome: &mut PublishRehearsalOutcome, phase: &'static str,
    error: &anyhow::Error, run: &PublishRunHooks<'_>) {
    let recovery = riviu_core::publish_recovery::describe(error);
    let detail = classified_preflight_detail(error);
    outcome.failure_phase = Some(phase);
    outcome.failure_code = Some(recovery.code.clone());
    outcome.preflight_detail = Some(detail.clone());
    // Persist the controlled cause before the terminal receipt. Failure to record
    // diagnostics cannot authorize cleanup or suppress the terminal blocker.
    let _ = (run.phase)("boundedFailureDiagnostic", &json!({
        "stage": phase, "recoveryCode": recovery.code, "detail": detail,
        "rawCauseRedacted": true,
    }));
}

fn checked(run: &PublishRunHooks<'_>) -> anyhow::Result<()> {
    ensure!(
        !run.stop.load(std::sync::atomic::Ordering::Relaxed) && Instant::now() < run.deadline,
        "no-public publish request interrupted"
    );
    Ok(())
}
fn value(v: &Value) -> &Value {
    v.get("value").unwrap_or(v)
}
fn stage_readback(staged: &Value) -> bool {
    let staged = value(staged);
    staged["ok"].as_bool() == Some(true)
        && staged["readback"].as_str() == Some("size+sha256")
        && staged["hiddenFromMediaStore"].as_bool() == Some(true)
        && staged["reusedImport"].as_bool() != Some(true)
}
fn import_proof(
    prepared: &Value,
    imported: &Value,
    id: &str,
    hash: &str,
    images: usize,
) -> anyhow::Result<String> {
    let prepared = value(prepared);
    let imported = value(imported);
    let import = prepared["importId"]
        .as_str()
        .context("prepare import identity missing")?;
    ensure!(
        prepared["campaignId"].as_str() == Some(id) && prepared["state"].as_str() == Some("ready"),
        "prepare identity/state mismatch"
    );
    ensure!(
        import.starts_with(&format!("riviu-{id}-"))
            && import.ends_with(&hash[..hash.len().min(12)]),
        "prepare import identity mismatch"
    );
    ensure!(
        imported["campaignId"].as_str() == Some(id)
            && imported["importId"].as_str() == Some(import)
            && imported["state"].as_str() == Some("imported")
            && imported["files"].as_u64() == Some(images as u64)
            && imported["mediaCounts"]["images"].as_u64() == Some(images as u64)
            && imported["mediaCounts"]["videos"].as_u64() == Some(0),
        "import readback mismatch"
    );
    ensure!(
        imported["mediaRefs"]
            .as_array()
            .is_some_and(|refs| refs.len() == images
                && refs
                    .iter()
                    .all(|r| r["collection"].as_str() == Some("images")
                        && r["id"].as_str().is_some()
                        && r["path"].as_str().is_some_and(
                            |p| p.starts_with(&format!("/sdcard/Pictures/{import}/"))
                        ))),
        "import rows unproved"
    );
    ensure!(
        refs_set(&imported["mediaRefs"]).is_some_and(|refs| refs.len() == images),
        "import references duplicated"
    );
    Ok(import.to_owned())
}
fn refs_set(refs: &Value) -> Option<std::collections::BTreeSet<(String, String, String)>> {
    let refs = refs.as_array()?;
    let mut result = std::collections::BTreeSet::new();
    for r in refs {
        let key = (
            r["collection"].as_str()?.to_owned(),
            r["id"].as_str()?.to_owned(),
            r["path"].as_str()?.to_owned(),
        );
        if !result.insert(key) {
            return None;
        }
    }
    Some(result)
}
fn cleanup_proof(cleaned: &Value, import: &str, expected: &Value) -> bool {
    let v = value(cleaned);
    let Some(actual) = refs_set(&v["mediaRefs"]) else {
        return false;
    };
    let Some(expected) = refs_set(expected) else {
        return false;
    };
    v["state"].as_str() == Some("cleaned")
        && v["importId"].as_str() == Some(import)
        && (actual == expected || (actual.is_empty() && v["files"].as_u64() == Some(0)))
}
/// Only explicit successful readback can make cleanup safe. A partial/lost stage,
/// prepare/import ACK retains attention even if a later delete says cleaned.
pub(crate) async fn prepare_publish(
    control: &DeviceControlPlane,
    context: &UiWithStreamContext,
    session: &dyn UiSession,
    input: &ApprovedPublishInput,
    run: &PublishRunHooks<'_>,
) -> PublishRehearsalOutcome {
    let mut outcome = PublishRehearsalOutcome {
        preparation: None,
        media_cleanup: MediaCleanup::NotStarted,
        cleanup_safe: true,
        blocker: None,
        import_id: None,
        failure_phase: None,
        failure_code: None,
        preflight_detail: None,
    };
    let mut account_navigation_started = false;
    let before = async {
        checked(run)?;
        uuid::Uuid::parse_str(run.request_id).context("request UUID required")?;
        ensure!(
            control.supports_push_media(run.udid),
            "native import unavailable before transfer"
        );
        ensure!(
            input.bundle.video.is_none() && !input.bundle.images.is_empty(),
            "no-public image-only route"
        );
        ensure!(
            format!("{:x}", Sha256::digest(serde_json::to_vec(&input.bundle)?))
                == input.fingerprint,
            "approved source fingerprint changed"
        );
        let (package, version, locale) = control.tiktok_build(run.udid).await?;
        ensure!(package == input.package, "approved package changed");
        let labels = riviu_core::tiktok_labels::controls_for(&package, &locale, &version)
            .context("unmeasured publish tuple")?;
        let plan = riviu_core::tiktok_composer::ComposerPlan::resolve(&labels)?;
        let sound = riviu_core::tiktok_sound::SoundPickerPlan::resolve(&package, &locale, &version)
            .context("unmeasured sound tuple")?;
        let (width, height) = riviu_core::screen::measured_screen_size(session).await?;
        let screen =
            riviu_core::tiktok_composer::Screen::new(width, height).context("unmeasured screen")?;
        (run.phase)("provingSafeFeedAndAccount", &json!({}))?;
        account_navigation_started = true;
        let token = riviu_core::tiktok_composer::rehearsal::prove_safe_publish_screen(
            session,
            labels,
            plan,
            &input.expected_account,
            &input.bundle.caption,
            input.bundle.images.len(),
            run.stop,
            run.deadline,
        )
        .await?;
        let root = run.report_dir.join("managed");
        ensure!(!root.exists(), "managed rehearsal already exists");
        let destination = root.join("approved");
        let source = input.bundle.clone();
        let managed = tokio::task::spawn_blocking(move || {
            riviu_core::copy_bundle_to_managed(&source, &destination)
        })
        .await??;
        checked(run)?;
        Ok::<_, anyhow::Error>((labels, plan, sound, screen, token, managed, root))
    }
    .await;
    let (labels, plan, sound, screen, token, _managed, root) = match before {
        Ok(v) => v,
        Err(error) => {
            record_failure(&mut outcome, "preflight", &error, run);
            outcome.cleanup_safe = !account_navigation_started
                && !error.is::<riviu_core::tiktok_composer::rehearsal::SafeScreenNeedsAttention>();
            outcome.blocker = Some("preflightBlocked");
            return outcome;
        }
    };
    let id = format!("np-{}", run.request_id);
    let mut transfer_started = false;
    let transfer = async {
        checked(run)?;
        (run.phase)(
            "stagingMedia",
            &json!({"deviceImportScope":id,"sourceFingerprint":input.fingerprint}),
        )?;
        transfer_started = true;
        let staged = control
            .stage_publish_media_with_ui(context, "com.mrph.svc", &id, &root)
            .await?;
        checked(run)?;
        ensure!(
            control.supports_push_media(run.udid),
            "native import unavailable"
        );
        let hash = value(&staged)["manifestSha256"]
            .as_str()
            .context("stage manifest missing")?;
        ensure!(
            hash.len() == 64
                && hash.chars().all(|c| c.is_ascii_hexdigit())
                && value(&staged)["campaignId"].as_str() == Some(id.as_str())
                && value(&staged)["fileCount"]
                    .as_u64()
                    .is_some_and(|count| count >= input.bundle.images.len() as u64)
                && stage_readback(&staged),
            "stage identity/hash/readback invalid"
        );
        (run.phase)(
            "preparingImport",
            &json!({"deviceImportScope":id,"manifestSha256":hash}),
        )?;
        let prepared = control
            .prepare_publish_media_with_ui(context, &id, hash)
            .await?;
        let expected = value(&prepared)["importId"]
            .as_str()
            .context("prepared import missing")?
            .to_owned();
        outcome.import_id = Some(expected.clone());
        checked(run)?;
        (run.phase)(
            "importingMedia",
            &json!({"deviceImportScope":id,"importId":expected,"manifestSha256":hash}),
        )?;
        let imported = control
            .import_publish_media_with_ui(context, &id, hash)
            .await?;
        let import = import_proof(&prepared, &imported, &id, hash, input.bundle.images.len())?;
        (run.phase)(
            "mediaImportVerified",
            &json!({"importId":import,"mediaRefs":value(&imported)["mediaRefs"]}),
        )?;
        Ok::<_, anyhow::Error>((import, value(&imported)["mediaRefs"].clone()))
    }
    .await;
    let (import, import_refs) = match transfer {
        Ok(v) => v,
        Err(error) => {
            record_failure(&mut outcome, "mediaTransfer", &error, run);
            outcome.cleanup_safe = !transfer_started;
            outcome.media_cleanup = if transfer_started {
                MediaCleanup::NeedsAttention
            } else {
                MediaCleanup::NotStarted
            };
            outcome.blocker = Some("mediaTransferUnproved");
            return outcome;
        }
    };
    outcome.import_id = Some(import.clone());
    drop(token);
    if (run.phase)("reprovingAccountAfterImport", &json!({"importId":import})).is_err() {
        outcome.cleanup_safe = false;
        outcome.media_cleanup = MediaCleanup::NeedsAttention;
        outcome.blocker = Some("accountPhaseUnrecorded");
        return outcome;
    }
    let token = match riviu_core::tiktok_composer::rehearsal::prove_safe_publish_screen(
        session,
        labels,
        plan,
        &input.expected_account,
        &input.bundle.caption,
        input.bundle.images.len(),
        run.stop,
        run.deadline,
    )
    .await
    {
        Ok(token) => token,
        Err(error) => {
            record_failure(&mut outcome, "accountAfterImport", &error, run);
            outcome.cleanup_safe = false;
            outcome.media_cleanup = MediaCleanup::NeedsAttention;
            outcome.blocker = Some("accountChangedAfterImport");
            return outcome;
        }
    };
    let preparation = riviu_core::tiktok_composer::rehearsal::prepare_images_before_post(
        session,
        labels,
        plan,
        sound,
        &input.sound_policy,
        screen,
        token,
        &import,
        run.stop,
        run.deadline,
        run.phase,
    )
    .await;
    let can_delete = preparation.cleanup_safe;
    outcome.preparation = Some(preparation);
    if !can_delete {
        outcome.media_cleanup = MediaCleanup::NeedsAttention;
        outcome.cleanup_safe = false;
        outcome.blocker = Some("composerCleanupUnproved");
        return outcome;
    }
    // A separate cleanup budget/phase is independent of Stop, but never erases
    // anything unless the exact imported receipt and composer cleanup are proved.
    if (run.phase)("cleaningOwnedMedia", &json!({"importId":import})).is_err() {
        outcome.media_cleanup = MediaCleanup::NeedsAttention;
        outcome.cleanup_safe = false;
        outcome.blocker = Some("cleanupPhaseUnrecorded");
        return outcome;
    }
    let cleaned = control
        .cleanup_publish_media_with_ui(context, &import)
        .await;
    let verified = cleaned
        .as_ref()
        .is_ok_and(|v| cleanup_proof(v, &import, &import_refs));
    outcome.media_cleanup = if verified {
        MediaCleanup::Verified
    } else {
        MediaCleanup::NeedsAttention
    };
    outcome.cleanup_safe = verified;
    if !verified {
        outcome.blocker = Some("mediaCleanupUnproved");
    }
    if (run.phase)(
        "mediaCleanupSettled",
        &json!({"importId":import,"verified":verified}),
    )
    .is_err()
    {
        outcome.cleanup_safe = false;
        outcome.blocker = Some("cleanupReadbackUnrecorded");
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preflight_detail_preserves_specific_nested_navigation_cause_without_payload() {
        let cause = anyhow::anyhow!("no-public publish tap target missing/overlapped; private caption and token=secret")
            .context("account_unreadable");
        assert_eq!(classified_preflight_detail(&cause), "tap_target_missing_or_overlapped");
        for (message, expected) in [
            ("no-public navigation actionable overlap", "navigation_actionable_overlap"),
            ("no-public publish layout changed before tap", "layout_changed_before_tap"),
            ("no-public publish unknown intended role refused", "unknown_intended_role"),
            ("no-public swipe requires owned picker", "swipe_requires_owned_picker"),
            ("no-public Back screen unproved", "back_screen_unproved"),
            ("account_session_changed", "account_session_changed"),
            ("account_unreadable", "account_unreadable"),
        ] {
            assert_eq!(classified_preflight_detail(&anyhow::anyhow!(message).context("outer facade")), expected);
        }
    }
    #[test]
    fn preflight_detail_unknown_is_bounded_and_does_not_disclose_secret_data() {
        let private = "clipboard=private; token=secret; caption=user text; credential=hidden";
        let detail = classified_preflight_detail(&anyhow::anyhow!(private));
        assert_eq!(detail, "unclassified_redacted");
        for sensitive in ["clipboard", "private", "token", "secret", "caption", "credential", "hidden"] {
            assert!(!detail.contains(sensitive));
        }
        let excessive = anyhow::anyhow!(format!("{} account_unreadable", "x".repeat(16 * 1024)));
        assert_eq!(classified_preflight_detail(&excessive), "unclassified_redacted");
    }
    #[test]
    fn unknown_or_mismatched_cleanup_ack_cannot_release_context() {
        assert!(!cleanup_proof(
            &json!({"state":"cleaned","importId":"other","mediaRefs":[]}),
            "riviu-fixture",
            &json!([])
        ));
        assert!(!cleanup_proof(
            &json!({"state":"cleaned","importId":"riviu-fixture"}),
            "riviu-fixture",
            &json!([])
        ));
        assert!(cleanup_proof(
            &json!({"state":"cleaned","importId":"riviu-fixture","mediaRefs":[]}),
            "riviu-fixture",
            &json!([])
        ));
    }
    #[test]
    fn duplicate_import_refs_and_foreign_cleanup_refs_are_rejected() {
        let row =
            json!({"collection":"images","id":"1","path":"/sdcard/Pictures/riviu-fixture/x.png"});
        assert!(refs_set(&json!([row.clone(), row.clone()])).is_none());
        let expected = json!([row]);
        assert!(!cleanup_proof(
            &json!({"state":"cleaned","importId":"riviu-fixture","mediaRefs":[{"collection":"images","id":"999","path":"/sdcard/Pictures/riviu-fixture/x.png"}]}),
            "riviu-fixture",
            &expected
        ));
        assert!(!cleanup_proof(
            &json!({"state":"cleaned","importId":"riviu-fixture","mediaRefs":[]}),
            "riviu-fixture",
            &expected
        ));
        assert!(cleanup_proof(
            &json!({"state":"cleaned","importId":"riviu-fixture","files":0,"mediaRefs":[]}),
            "riviu-fixture",
            &expected
        ));
    }
    #[test]
    fn fresh_android_hidden_stage_receipt_is_supported_but_unproved_reuse_is_not() {
        assert!(stage_readback(
            &json!({"ok":true,"readback":"size+sha256","hiddenFromMediaStore":true})
        ));
        assert!(!stage_readback(
            &json!({"ok":true,"readback":"size+sha256"})
        ));
        assert!(!stage_readback(
            &json!({"ok":true,"readback":"size+sha256+MediaStore","hiddenFromMediaStore":false,"reusedImport":true})
        ));
    }
    #[test]
    fn import_lost_ack_or_wrong_rows_never_proves_transfer() {
        let prepare =
            json!({"campaignId":"fixture","state":"ready","importId":"riviu-fixture-aaaaaaaaaaaa"});
        assert!(import_proof(&prepare, &json!({}), "fixture", &"a".repeat(64), 1).is_err());
        assert!(import_proof(&prepare,&json!({"campaignId":"fixture","state":"imported","importId":"riviu-fixture-aaaaaaaaaaaa","files":1,"mediaCounts":{"images":1,"videos":0},"mediaRefs":[{"collection":"images","id":"1","path":"/Pictures/foreign/image.png"}]}),"fixture",&"a".repeat(64),1).is_err());
    }
}
