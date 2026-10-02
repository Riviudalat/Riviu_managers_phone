//! Root draft preparation only. No campaign, action ledger or Send-capable facade.
use anyhow::{ensure, Context};
use serde_json::json;
use sha2::{Digest, Sha256};
use tauri::Manager;

#[tauri::command]
pub(crate) async fn no_public_prepare_interaction(
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
    ensure!(device.helper_canary, "scoped helper SET/GET preparation approval required");
    let text = device.draft_text.as_deref().context("draft text missing")?;
    riviu_core::interaction_hierarchy::rehearsal::validate_rehearsal_text(text)?;
    let url = device.target_url.as_deref().context("target missing")?;
    let target = riviu_core::parse_tiktok_links(url)
        .into_iter()
        .next()
        .and_then(|row| row.target)
        .context("canonical target required")?;
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"udid":udid,"package":device.package,"account":device.expected_account,"target":target,"text":text})
        )?)
    );
    let state = app.state::<crate::state::AppState>();
    let admission = state.ensure_accepting_work().map_err(|error| anyhow::anyhow!("{}", error.message))?;
    let (run, inserted) = state.rehearsal_runs.begin(
        scope,
        udid,
        id,
        "interaction",
        &digest,
        std::time::Duration::from_secs(90),
    )?;
    if !inserted {
        return Ok(run.status());
    }
    let registry = state.rehearsal_runs.clone();
    let handle = app.clone();
    let owned = run.clone();
    registry.spawn(&run, async move {
        let _admission = admission;
        let state = handle.state::<crate::state::AppState>();
        let result = prepare(&state, &device, &target, &owned).await;
        if let Err(error) = result {
            let _ = owned.finish(
                "needsAttention",
                json!({"error":error.to_string(),"publicEffectsAllowed":false}),
            );
        }
    })?;
    Ok(run.status())
}
async fn prepare(
    state: &crate::state::AppState,
    device: &crate::no_public::DeviceScope,
    target: &riviu_core::ResolvedTikTokTarget,
    run: &crate::no_public_runs::RunContext,
) -> anyhow::Result<()> {
    use riviu_core::interaction_hierarchy::rehearsal::{
        prepare_root_before_send, prove_rehearsal_binding, DraftCleanup,
    };
    run.check()?;
    run.record_phase("acquiring", &json!({}))?;
    let exclusive = state
        .control
        .try_acquire_exclusive(&run.udid, riviu_core::DeviceWorkOwner::Interaction)
        .await?;
    let (exclusive, capacity) = state
        .control
        .reserve_ui_capacity_until(exclusive, || run.check().is_err(), || {})
        .await?;
    run.check()?;
    let package = state.control.resolve_tiktok_package(&run.udid).await?;
    ensure!(
        package == device.package,
        "package changed from approved scope"
    );
    run.record_phase("preparingScopedHelper", &json!({"clipboardSetGetRequired":true}))?;
    let helper = match state.android.as_ref().context("Android helper scope required")?
        .prepare_helper_canary(&run.udid, run.report_dir.clone()).await {
        Ok(helper) => helper,
        Err(error) => { state.control.quarantine_exclusive_context(exclusive)?; return Err(error); }
    };
    if let Err(error) = helper.qualify_clipboard_roundtrip().await {
        state.control.quarantine_exclusive_context(exclusive)?;
        return Err(error.context("scoped clipboard SET/GET qualification failed"));
    }
    if let Err(error) = helper.capture_clipboard_baseline().await {
        state.control.quarantine_exclusive_context(exclusive)?;
        return Err(error.context("target Copy-link clipboard baseline unavailable"));
    }
    if let Err(error) = run.record_phase("clipboardRoundTripVerified", &json!({"baselineRestored":true,"contentsLogged":false})) {
        state.control.quarantine_exclusive_context(exclusive)?;
        return Err(error);
    }
    let ctx = state
        .control
        .start_interaction_session_or_quarantine(
            exclusive,
            &package,
            riviu_core::InteractionSessionKind::Ordinary,
        )
        .await?;
    let context = state.control.start_reserved_stream_or_quarantine(ctx, capacity).await?;
    let owned_context = crate::no_public_owned::OwnedUi::new(state.control.clone(), context).with_helper(helper.clone());
    let session = state.control.streaming_session(owned_context.context())?;
    session.set_gui_scope(riviu_core::ui_automation::GuiScope {
        run_id: run.request_id.clone(),
        assignment_id: None,
        device_id: run.udid.clone(),
        deadline_ms: Some(chrono::Utc::now().timestamp_millis() + run.remaining_ms() as i64),
    });
    let work: Result<_, anyhow::Error> = async {
        run.check()?;
        run.record_phase("provingAccountAndTarget", &json!({}))?;
        let (actual, version, locale) = state.control.tiktok_build(&run.udid).await?;
        ensure!(actual == package, "app metadata changed");
        let labels = riviu_core::tiktok_labels::controls_for(&package, &locale, &version)
            .context("unmeasured app tuple")?;
        let text = device.draft_text.as_deref().context("draft missing")?;
        let binding = prove_rehearsal_binding(
            session.as_ref(),
            labels,
            &device.expected_account,
            target,
            text,
            &run.stop,
            run.deadline,
        )
        .await?;
        helper.restore_clipboard_baseline(binding.canonical_clipboard_value().as_bytes()).await?;
        run.record_phase("targetClipboardRestored", &json!({"metadataPreserved":true,"contentsLogged":false}))?;
        let screen = riviu_core::screen::measured_screen_size(session.as_ref()).await?;
        run.check()?;
        run.record_phase("preparingDraft", &json!({}))?;
        prepare_root_before_send(
            session.as_ref(),
            labels,
            screen,
            binding,
            text,
            &run.stop,
            run.deadline,
            String::new,
        )
        .await
        .map_err(Into::into)
    }
    .await;
    let cleanup = match &work {
        Ok(prepared) => prepared.cleanup,
        Err(error) => error
            .downcast_ref::<riviu_core::interaction_hierarchy::rehearsal::PrepareFailure>()
            .map(|failure| failure.cleanup)
            .unwrap_or(DraftCleanup::NotCreated),
    };
    let safe = matches!(
        cleanup,
        DraftCleanup::NotCreated | DraftCleanup::ClearedAndVerified
    );
    let outcome = json!({"prepared":work.as_ref().ok(),"error":work.as_ref().err().map(ToString::to_string),"draftCleanup":cleanup,"publicEffectIntentWritten":false,"publicEffectDispatched":false});
    if !safe {
        let persistence = run.finish("needsAttention", outcome);
        owned_context.quarantine()?;
        persistence?;
        return Ok(());
    }
    // Failure to persist the semantic outcome also keeps ownership, never releases on a guess.
    if let Err(error) = run.record_phase("draftCleanupSettled", &outcome) {
        owned_context.quarantine()?;
        return Err(error);
    }
    let closed = owned_context.close_safe().await;
    let state_name = if closed.is_err() {
        "needsAttention"
    } else if work.is_ok() {
        "preparedAndCleaned"
    } else {
        "blocked"
    };
    run.finish(state_name,json!({"outcome":outcome,"closed":closed.is_ok(),"closeError":closed.as_ref().err().map(ToString::to_string)}))?;
    closed?;
    Ok(())
}
