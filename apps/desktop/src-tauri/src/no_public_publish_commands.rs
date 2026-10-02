//! Typed receipt facade; all media/composer behavior is in the reviewed helper.
use anyhow::{ensure, Context};
use serde_json::json;
use tauri::Manager;

#[tauri::command]
pub(crate) async fn no_public_prepare_publish(
    app: tauri::AppHandle,
    udid: String,
    request_id: String,
) -> Result<serde_json::Value, crate::command_error::CommandError> {
    start(&app, &udid, &request_id)
        .await
        .map_err(crate::command_error::CommandError::operation)
}
async fn start(app: &tauri::AppHandle, udid: &str, id: &str) -> anyhow::Result<serde_json::Value> {
    let policy = app.state::<crate::ui_smoke::StartupPolicy>();
    let scope = policy.rehearsal().context("no-public mode required")?;
    let device = scope.device(udid)?.clone();
    let source = device
        .source_root
        .as_ref()
        .context("source missing")?
        .clone();
    let bundle_id = device.bundle_id.as_ref().context("bundle missing")?.clone();
    let manifest = tokio::task::spawn_blocking(move || {
        riviu_core::scan_publish_folder(&source, Default::default())
    })
    .await??;
    let bundle = manifest
        .bundles
        .into_iter()
        .find(|b| b.id == bundle_id)
        .context("approved bundle not found")?;
    ensure!(
        bundle.video.is_none(),
        "only image rehearsal is qualified; video refused before device access"
    );
    let fingerprint = riviu_core::frame_sha256(&serde_json::to_vec(&bundle)?);
    ensure!(
        device.publish_fingerprint.as_deref() == Some(fingerprint.as_str()),
        "approved source fingerprint changed"
    );
    let input = crate::no_public_publish::ApprovedPublishInput {
        bundle,
        fingerprint: fingerprint.clone(),
        expected_account: device.expected_account.clone(),
        package: device.package.clone(),
        sound_policy: device
            .sound_policy
            .clone()
            .context("approved sound policy missing")?,
    };
    let digest = riviu_core::frame_sha256(&serde_json::to_vec(
        &json!({"fingerprint":fingerprint,"account":device.expected_account,"package":device.package,"sound":device.sound_policy}),
    )?);
    let state = app.state::<crate::state::AppState>();
    let admission = state
        .ensure_accepting_work()
        .map_err(|error| anyhow::anyhow!("{}", error.message))?;
    let (run, inserted) = state.rehearsal_runs.begin(
        scope,
        udid,
        id,
        "publish",
        &digest,
        std::time::Duration::from_secs(300),
    )?;
    if inserted {
        let handle = app.clone();
        let owned = run.clone();
        state.rehearsal_runs.spawn(&run, async move {
            let _admission = admission;
            let state = handle.state::<crate::state::AppState>();
            if let Err(error) = prepare(&state, &input, &owned).await {
                let _ = owned.finish("needsAttention", json!({"error":error.to_string()}));
            }
        })?;
    }
    Ok(run.status())
}
async fn prepare(
    state: &crate::state::AppState,
    input: &crate::no_public_publish::ApprovedPublishInput,
    run: &crate::no_public_runs::RunContext,
) -> anyhow::Result<()> {
    run.check()?;
    run.record_phase("acquiring", &json!({}))?;
    let exclusive = state
        .control
        .try_acquire_exclusive(&run.udid, riviu_core::DeviceWorkOwner::Script)
        .await?;
    let (exclusive, capacity) = state
        .control
        .reserve_ui_capacity_until(exclusive, || run.check().is_err(), || {})
        .await?;
    let package = state.control.resolve_tiktok_package(&run.udid).await?;
    ensure!(package == input.package, "package changed");
    let ctx = state
        .control
        .start_interaction_session_or_quarantine(
            exclusive,
            &package,
            riviu_core::InteractionSessionKind::Ordinary,
        )
        .await?;
    let ctx = state.control.start_reserved_stream_or_quarantine(ctx, capacity).await?;
    let context = crate::no_public_owned::OwnedUi::new(state.control.clone(), ctx);
    let session = state.control.streaming_session(context.context())?;
    session.set_gui_scope(riviu_core::ui_automation::GuiScope {
        run_id: run.request_id.clone(),
        assignment_id: None,
        device_id: run.udid.clone(),
        deadline_ms: Some(chrono::Utc::now().timestamp_millis() + run.remaining_ms() as i64),
    });
    let phase = |name: &str, value: &serde_json::Value| run.record_phase(name, value);
    let hooks = crate::no_public_publish::PublishRunHooks {
        request_id: &run.request_id,
        udid: &run.udid,
        deadline: run.deadline,
        stop: &run.stop,
        report_dir: &run.report_dir,
        phase: &phase,
    };
    let outcome = crate::no_public_publish::prepare_publish(
        &state.control,
        context.context(),
        session.as_ref(),
        input,
        &hooks,
    )
    .await;
    let safe = outcome.cleanup_safe;
    let prepared = outcome
        .preparation
        .as_ref()
        .is_some_and(|p| p.prepared.is_some() && p.blocker.is_none() && p.cleanup_safe)
        && outcome.blocker.is_none();
    let receipt = serde_json::to_value(&outcome)?;
    if !safe {
        let saved = run.finish("needsAttention", receipt);
        context.quarantine()?;
        saved?;
        return Ok(());
    }
    if let Err(error) = run.record_phase("ownedCleanupVerified", &receipt) {
        context.quarantine()?;
        return Err(error);
    }
    context.close_safe().await?;
    run.finish(
        if prepared {
            "preparedAndCleaned"
        } else {
            "blocked"
        },
        receipt,
    )?;
    Ok(())
}
