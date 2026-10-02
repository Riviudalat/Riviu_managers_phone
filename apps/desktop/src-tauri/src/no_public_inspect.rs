//! Account discovery activation: navigation only, no composer or data input.
use anyhow::{ensure, Context};
use serde_json::json;
use tauri::Manager;

#[tauri::command]
pub(crate) async fn no_public_inspect(
    app: tauri::AppHandle,
    udid: String,
    request_id: String,
) -> Result<serde_json::Value, crate::command_error::CommandError> {
    let policy = app.state::<crate::ui_smoke::StartupPolicy>();
    let scope = policy.rehearsal().ok_or_else(|| {
        crate::command_error::CommandError::code(
            "NoPublicModeRequired",
            "diagnostic activation required",
        )
    })?;
    let device = scope
        .device(&udid)
        .map_err(crate::command_error::CommandError::operation)?
        .clone();
    let state = app.state::<crate::state::AppState>();
    let admission = state.ensure_accepting_work()?;
    let (run, inserted) = state
        .rehearsal_runs
        .begin(
            scope,
            &udid,
            &request_id,
            "inspect",
            &device.package,
            std::time::Duration::from_secs(45),
        )
        .map_err(crate::command_error::CommandError::operation)?;
    if inserted {
        let handle = app.clone();
        let owned = run.clone();
        state
            .rehearsal_runs
            .spawn(&run, async move {
                let _admission = admission;
                let state = handle.state::<crate::state::AppState>();
                if let Err(error) = inspect(&state, &device, &owned).await {
                    let _ = owned.finish("needsAttention", json!({"error":error.to_string()}));
                }
            })
            .map_err(crate::command_error::CommandError::operation)?;
    }
    Ok(run.status())
}
async fn inspect(
    state: &crate::state::AppState,
    device: &crate::no_public::DeviceScope,
    run: &crate::no_public_runs::RunContext,
) -> anyhow::Result<()> {
    run.check()?;
    let exclusive = state
        .control
        .try_acquire_exclusive(&run.udid, riviu_core::DeviceWorkOwner::Interaction)
        .await?;
    let (exclusive, capacity) = state
        .control
        .reserve_ui_capacity_until(exclusive, || run.check().is_err(), || {})
        .await?;
    let package = state.control.resolve_tiktok_package(&run.udid).await?;
    ensure!(
        package == device.package,
        "package differs from discovery scope"
    );
    let (actual, version, locale) = state.control.tiktok_build(&run.udid).await?;
    ensure!(actual == package, "metadata package changed");
    run.record_phase("deviceMetadata", &json!({"package":package,"version":version,"locale":locale}))?;
    if device.allow_warm_launch {
        run.record_phase("warmForegroundIntent", &json!({"package":package,"forceStop":false,"unlock":false}))?;
        if let Err(error) = state.control.foreground_target_app(&exclusive, &package).await {
            state.control.quarantine_exclusive_context(exclusive)?;
            return Err(error.into());
        }
        run.check()?;
    }
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
    session.set_gui_scope(riviu_core::ui_automation::GuiScope { run_id: run.request_id.clone(), assignment_id: None, device_id: run.udid.clone(), deadline_ms: Some(chrono::Utc::now().timestamp_millis() + run.remaining_ms() as i64) });
    let labels = riviu_core::tiktok_labels::controls_for(&package, &locale, &version)
        .context("unmeasured labels")?;
    run.record_phase(
        "readingAccount",
        &json!({"package":package,"version":version,"locale":locale}),
    )?;
    let account = riviu_core::interaction_hierarchy::rehearsal::inspect_rehearsal_account_with_baseline(
        session.as_ref(),
        labels,
        &run.stop,
        run.deadline,
        |observation| {
            ensure!(observation.generation > 0 && observation.xml.len() <= 4 * 1024 * 1024,
                "discovery verifier baseline invalid");
            persist_evidence(&run.report_dir.join("discovery-verifier.xml"), observation.xml.as_bytes())?;
            run.record_phase("readingAccount", &json!({
                "verifierGeneration":observation.generation,
                "verifierSha256":riviu_core::frame_sha256(observation.xml.as_bytes()),
                "publicEffectsAllowed":false,
            }))
        },
    )
    .await?;
    let observation = match riviu_core::ui_automation::runtime::read_before_deadline(session.hierarchy_source_snapshot(), run.deadline, &run.stop).await? {
        riviu_core::ui_automation::runtime::ReadWaitResult::Ready(observation) => observation,
        _ => anyhow::bail!("account discovery interrupted before final readback"),
    };
    run.check()?;
    std::fs::write(run.report_dir.join("account.xml"), &observation.xml)?;
    run.record_phase("accountObserved",&json!({"account":account,"package":package,"version":version,"locale":locale,"snapshotSha256":riviu_core::frame_sha256(observation.xml.as_bytes())}))?;
    context.close_safe().await?;
    run.finish("preparedAndCleaned",json!({"account":account,"package":package,"version":version,"locale":locale,"deviceId":run.udid,"typed":false,"publicEffectDispatched":false}))?;
    Ok(())
}

fn persist_evidence(path: &std::path::Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn persist_baseline_xml(
    directory: &std::path::Path,
    observation: &riviu_core::HierarchySourceSnapshot,
) -> anyhow::Result<()> {
    ensure!(observation.generation > 0 && observation.xml.len() <= 4 * 1024 * 1024,
        "baseline generation or size invalid");
    persist_evidence(&directory.join("baseline.xml"), observation.xml.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_public_baseline_retains_unparseable_supporting_observation_without_overwrite() {
        let directory = std::env::temp_dir().join(format!("riviu-inspect-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let observation = riviu_core::HierarchySourceSnapshot { generation: 1, xml: "not an accessibility tree".into() };
        persist_baseline_xml(&directory, &observation).unwrap();
        let changed = riviu_core::HierarchySourceSnapshot { generation: 2, xml: "replacement".into() };
        assert!(persist_baseline_xml(&directory, &changed).is_err());
        assert_eq!(std::fs::read_to_string(directory.join("baseline.xml")).unwrap(), observation.xml);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn no_public_baseline_rejects_zero_generation_before_creating_evidence() {
        let directory = std::env::temp_dir().join(format!("riviu-inspect-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let observation = riviu_core::HierarchySourceSnapshot { generation: 0, xml: "<hierarchy/>".into() };
        assert!(persist_baseline_xml(&directory, &observation).is_err());
        assert!(!directory.join("baseline.xml").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
