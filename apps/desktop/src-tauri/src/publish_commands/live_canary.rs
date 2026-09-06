//! Explicit, ignored single-device acceptance using the production Publish functions.
use super::*;
use riviu_core::driver::{DeviceDriver, ElementQuery};

const SERIAL: &str = "9889db374744474635";
const HANDLE: &str = "@ghin.lt.sng.sng";

fn save(out: &Path, name: &str, value: &impl serde::Serialize) -> anyhow::Result<()> {
    fs::write(out.join(name), serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

#[tokio::test]
#[ignore = "Explicit Android canary; inspect first, then requires RIVIU_PUBLISH_CANARY=publish-one"]
async fn live_publish_canary() -> anyhow::Result<()> {
    let mode = std::env::var("RIVIU_PUBLISH_CANARY")?;
    anyhow::ensure!(
        matches!(
            mode.as_str(),
            "inspect" | "sound-scout" | "rehearse" | "publish-one" | "link-only" | "link-scout"
        ),
        "invalid canary mode"
    );
    let source = PathBuf::from(std::env::var("RIVIU_PUBLISH_SOURCE")?).canonicalize()?;
    let out = PathBuf::from(std::env::var("RIVIU_PUBLISH_REPORT")?);
    fs::create_dir_all(&out)?;
    anyhow::ensure!(
        matches!(mode.as_str(), "link-only" | "link-scout") || !out.join("canary.db").exists(),
        "use a new report directory; do not replay a canary"
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sidecars/android");
    let noarch = root.join("noarch");
    let driver = Arc::new(riviu_android_driver::AndroidDriver::new(
        &riviu_android_driver::AndroidDriverConfig {
            bundled_adb_path: Some(root.join("win-x86_64/adb.exe")),
            bundled_minicap_apk: Some(noarch.join("minicap.apk")),
            bundled_scrcpy_server: Some(noarch.join("scrcpy-server")),
            bundled_riviu_agent_apk: Some(noarch.join("riviu-agent.apk")),
            bundled_agent_server_apk: Some(noarch.join("appium-uiautomator2-server.apk")),
            bundled_agent_test_apk: Some(noarch.join("appium-uiautomator2-server-test.apk")),
            ..Default::default()
        },
    )?);
    let frames = Arc::new(riviu_ios_driver::StreamHub::new());
    driver.set_frame_sink(Arc::new(frames.as_ref().clone()));
    let control = Arc::new(DeviceControlPlane::new(
        driver.clone(),
        Arc::new(riviu_core::DeviceWorkCoordinator::new()),
        Arc::new(riviu_core::StreamBudgetManager::new(1)?),
    ));
    let events = riviu_core::events::EventBus::new(128);
    let registry = riviu_core::DeviceRegistry::new(events.clone());
    registry.upsert_many(
        driver
            .list_devices()
            .await?
            .into_iter()
            .filter(|device| device.udid == SERIAL)
            .collect(),
    );
    anyhow::ensure!(registry.get(SERIAL).is_some(), "canary phone absent");
    let db = Arc::new(Database::open(out.join("canary.db"))?);
    let result: anyhow::Result<()> = async {
        if mode == "link-only" || mode == "link-scout" {
            let campaign: PublishCampaignRecord = serde_json::from_slice(&fs::read(out.join("campaign.json"))?)?;
            let detail = db.get_publish_campaign(&campaign.id)?.context("missing canary")?;
            anyhow::ensure!(detail.assignments.len() == 1 && detail.assignments[0].udid == SERIAL && detail.assignments[0].state == riviu_core::PublishCampaignState::Succeeded, "link retry requires exactly the confirmed canary; never Post");
            if mode == "link-scout" {
                let context = open_publish_context(&control,SERIAL).await?;
                let session = control.streaming_session(&context)?;
                let observed: anyhow::Result<()> = async {
                    let (package,version,locale) = control.tiktok_build(SERIAL).await?;
                    let labels = riviu_core::tiktok_labels::controls_for(&package,&locale,&version).context("labels")?;
                    let profile = labels.label(riviu_core::tiktok_labels::TikTokControl::ProfileTab).context("profile")?;
                    let tab = session.locate(profile.to_query()).await?.context("profile tab")?;
                    session.tap(tab.centre()).await?;
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    let handles = session.locate_all_described(ElementQuery::Text { value: HANDLE, exact: true }).await?;
                    anyhow::ensure!(handles.len()==1,"account changed");
                    let tiles = session.locate_all(labels.post_tile_id().context("tile locator")?.to_query()).await?;
                    let caption = &detail.bundles[0].caption;
                    let prefix: String = caption.chars().take(24).collect();
                    let mut found=false;
                    for tile in tiles.into_iter().take(3) {
                        session.tap(tile.centre()).await?;
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        if session.locate(ElementQuery::Text { value:&prefix,exact:false }).await?.is_some() { found=true;break; }
                        session.back().await?;
                    }
                    anyhow::ensure!(found,"canary caption not found");
                    fs::write(out.join("owned-post.xml"),session.hierarchy_source_snapshot().await?.xml)?;
                    let mut stream=FrameSource::subscribe(frames.as_ref(),SERIAL);
                    if let Ok(Some(frame))=tokio::time::timeout(Duration::from_secs(5),stream.next()).await { fs::write(out.join("owned-post.jpg"),frame.as_ref())?; }
                    let mark=format!("riviu-link-scout-{}",Uuid::new_v4());
                    let primed=session.set_clipboard("plaintext",mark.as_bytes()).await;
                    save(&out,"clipboard-prime.json",&serde_json::json!({"error":primed.as_ref().err().map(|e|format!("{e:#}"))}))?;
                    primed?;
                    let share=session.locate(labels.label(riviu_core::tiktok_labels::TikTokControl::Share).context("share")?.to_query()).await?.context("share absent")?;
                    session.tap(share.centre()).await?;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    fs::write(out.join("share-sheet.xml"),session.hierarchy_source_snapshot().await?.xml)?;
                    let mut stream=FrameSource::subscribe(frames.as_ref(),SERIAL);
                    if let Ok(Some(frame))=tokio::time::timeout(Duration::from_secs(5),stream.next()).await { fs::write(out.join("share-sheet.jpg"),frame.as_ref())?; }
                    Ok(())
                }.await;
                let closed=control.close_ui_context(context).await;
                observed?;closed?;
                return Ok(());
            }
            let executed = execute_publish_campaign_inner(control.clone(), registry, db.clone(), events, "com.mrph.svc".into(), frames, campaign.id, true).await?;
            save(&out, "link-retry.json", &executed)?;
            anyhow::ensure!(db.pending_publish_sheet_rows(10)?.is_empty(), "Sheet disabled but outbox exists");
            anyhow::ensure!(executed.status == riviu_core::PublishExecutionStatus::Complete, "link still pending; no Post retried");
            return Ok(());
        }
        let manifest = scan_publish_folder(&source, PublishScanOptions::default())?;
        save(&out, "manifest.json", &manifest)?;
        let bundle = manifest.bundles.iter().find(|bundle| bundle.name == "set1 01 3n2d").context("requested first image bundle missing")?;
        let request = riviu_core::PublishPreflightRequest {
            source_root: source.to_string_lossy().to_string(), bundle_ids: vec![bundle.id.clone()],
            udids: vec![SERIAL.into()], target_ref: Some(riviu_core::TargetRef::Explicit { udids: vec![SERIAL.into()] }),
            run_at: None, caption_overrides: Default::default(),
            sound_policy: riviu_core::PublishSoundPolicy::TrendingAny { pool_size: 5, seed: 20260906 },
            sheet_enabled: false,
        };
        let prepared = build_publish_preflight(&control, &registry, &db, request.clone()).await?;
        save(&out, "preflight.json", &prepared.report)?;
        anyhow::ensure!(prepared.report.can_execute, "preflight failed; inspect report");
        let context = open_publish_context(&control, SERIAL).await?;
        let session = control.streaming_session(&context)?;
        let account_result: anyhow::Result<()> = async {
            let (package, version, locale) = control.tiktok_build(SERIAL).await?;
            let labels = riviu_core::tiktok_labels::controls_for(&package, &locale, &version).context("unmeasured build")?;
            let profile = labels.label(riviu_core::tiktok_labels::TikTokControl::ProfileTab).context("profile locator missing")?;
            let tab = session.locate(profile.to_query()).await?.context("profile tab absent")?;
            session.tap(tab.centre()).await?;
            tokio::time::sleep(Duration::from_secs(3)).await;
            let snapshot = session.hierarchy_source_snapshot().await?;
            fs::write(out.join("account.xml"), snapshot.xml)?;
            if let Some(frame) = frames.latest(SERIAL) { fs::write(out.join("account.jpg"), frame.as_ref())?; }
            let handles = session.locate_all_described(ElementQuery::Text { value: HANDLE, exact: true }).await?;
            anyhow::ensure!(handles.len() == 1 && handles[0].description.as_deref() == Some(HANDLE), "canary account not proven");
            save(&out, "account.json", &serde_json::json!({"serial":SERIAL,"handle":HANDLE,"package":package,"version":version,"locale":locale,"proved":true}))?;
            Ok(())
        }.await;
        let closed = control.close_ui_context(context).await;
        account_result?;
        closed?;
        if mode == "inspect" { return Ok(()); }
        let id = Uuid::new_v4().to_string();
        let mut managed = copy_bundle_to_managed(bundle, &out.join("managed").join(&id))?;
        managed.id = format!("{id}:{}", bundle.id);
        let campaign_request = PublishCampaignRequest {
            request_id: id, source_root: request.source_root, bundle_ids: vec![managed.id.clone()],
            udids: request.udids, run_at: None, visibility: PublishVisibility::Public,
            cleanup_policy: PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
            sound_policy: request.sound_policy, sheet_enabled: false, execution_confirmed: true,
            target_snapshot: Some(prepared.report.target_snapshot.clone()),
        };
        let campaign = db.create_publish_campaign_with_snapshot(&campaign_request, &[managed], &riviu_core::PublishExecutionSnapshotDraft {
            input_digest: prepared.report.input_digest, status: riviu_core::PublishExecutionStatus::Partial,
            retry_scope: riviu_core::PublishRetryScope::FullPipeline, report_json: serde_json::to_value(&campaign_request)?,
        })?;
        save(&out, "campaign.json", &campaign)?;
        if mode == "rehearse" || mode == "sound-scout" {
            let detail = transfer_publish_campaign_inner(control.clone(), db.clone(), events.clone(), "com.mrph.svc".into(), campaign.id.clone()).await?;
            let assignment = &detail.assignments[0];
            let bundle = &detail.bundles[0];
            let import = assignment.evidence_json.as_deref().and_then(import_id_from_evidence).context("import missing")?;
            let context = open_publish_context(&control, SERIAL).await?;
            let session = control.streaming_session(&context)?;
            if mode == "sound-scout" {
                let observed: anyhow::Result<()> = async {
                    use riviu_core::tiktok_composer::{Composer, ComposerPlan, Screen, CarouselRequest, reach_edit_step};
                    let (package, version, locale) = control.tiktok_build(SERIAL).await?;
                    let labels = riviu_core::tiktok_labels::controls_for(&package, &locale, &version).context("labels")?;
                    let plan = ComposerPlan::resolve(&labels)?;
                    let (width, height) = riviu_core::screen::measured_screen_size(session.as_ref()).await?;
                    let screen = Screen::new(width, height).context("screen")?;
                    let mut composer = Composer::new(session.as_ref(), plan, riviu_core::tiktok_composer::human_taps(screen));
                    let request = CarouselRequest { album: &import, images: bundle.images.len(), caption: &bundle.caption, screen };
                    let stop = std::sync::atomic::AtomicBool::new(false);
                    reach_edit_step(&mut composer, &request, &stop).await?;
                    let plan = riviu_core::tiktok_sound::SoundPickerPlan::resolve(&package, &locale, &version).context("sound plan")?;
                    let pool = riviu_core::tiktok_sound::open_and_observe_sounds(session.as_ref(), plan, 5).await;
                    let snapshot = session.hierarchy_source_snapshot().await?;
                    fs::write(out.join("sound-picker.xml"), snapshot.xml)?;
                    let mut stream = FrameSource::subscribe(frames.as_ref(), SERIAL);
                    if let Ok(Some(frame)) = tokio::time::timeout(Duration::from_secs(5), stream.next()).await { fs::write(out.join("sound-picker.jpg"), frame.as_ref())?; }
                    save(&out, "sound-pool.json", &serde_json::json!({"candidates":pool.as_ref().ok().map(|p| &p.candidates),"error":pool.as_ref().err().map(ToString::to_string)}))?;
                    let pool = pool?;
                    let selection = riviu_core::publish::select_sound_candidate(&campaign_request.sound_policy, &pool.candidates)?;
                    let selected = riviu_core::tiktok_sound::choose_and_confirm_sound(session.as_ref(), plan, &pool, selection.index).await;
                    fs::write(out.join("production-selection.xml"), session.hierarchy_source_snapshot().await?.xml)?;
                    let mut stream = FrameSource::subscribe(frames.as_ref(), SERIAL);
                    if let Ok(Some(frame)) = tokio::time::timeout(Duration::from_secs(5), stream.next()).await { fs::write(out.join("production-selection.jpg"), frame.as_ref())?; }
                    save(&out, "production-selection.json", &serde_json::json!({"selection":selection,"error":selected.as_ref().err().map(|e|format!("{e:#}"))}))?;
                    selected?;
                    Ok(())
                }.await;
                let cleaned = tidy_up_the_imported_media(&control, context, SERIAL, &import).await;
                save(&out, "media-cleanup.json", &serde_json::json!({"cleanup":cleaned.as_ref().ok(),"error":cleaned.as_ref().err().map(ToString::to_string)}))?;
                observed?;
                cleaned?;
                return Ok(());
            }
            let mut reached_post = false;
            let mut before_post = |selection: Option<&riviu_core::SoundSelectionEvidence>| {
                reached_post = true;
                save(&out, "sound.json", &selection)?;
                anyhow::bail!("canary rehearsal: stop before public Post")
            };
            let outcome = post_through_the_composer(&control, session.as_ref(), &campaign.id, SERIAL, bundle, &import, &campaign_request.sound_policy, &mut before_post).await;
            let description = match outcome { PostOutcome::NothingPublished(reason) => reason, PostOutcome::Unknown(reason) => format!("UNCERTAIN {reason}"), PostOutcome::Posted(_) => "UNEXPECTED POST".into() };
            if let Ok(snapshot) = session.hierarchy_source_snapshot().await { fs::write(out.join("composer.xml"), snapshot.xml)?; }
            if let Some(frame) = frames.latest(SERIAL) { fs::write(out.join("composer.jpg"), frame.as_ref())?; }
            let cleaned = tidy_up_the_imported_media(&control, context, SERIAL, &import).await;
            save(&out, "rehearsal.json", &serde_json::json!({"reachedPost":reached_post,"outcome":description,"cleanup":cleaned.as_ref().ok(),"cleanupError":cleaned.as_ref().err().map(ToString::to_string)}))?;
            cleaned?;
            anyhow::ensure!(reached_post, "rehearsal stopped before the Post boundary: {description}");
            return Ok(());
        }
        let executed = execute_publish_campaign_inner(control.clone(), registry, db.clone(), events, "com.mrph.svc".into(), frames, campaign.id.clone(), true).await;
        save(&out, "detail.json", &db.get_publish_campaign(&campaign.id)?)?;
        let executed = executed?;
        save(&out, "execution.json", &executed)?;
        anyhow::ensure!(db.pending_publish_sheet_rows(10)?.is_empty(), "Sheet disabled but outbox exists");
        anyhow::ensure!(executed.status == riviu_core::PublishExecutionStatus::Complete, "live publish not complete; do not retry Post");
        Ok(())
    }.await;
    let package = control.resolve_tiktok_package(SERIAL).await?;
    let owner = control
        .try_acquire_exclusive(SERIAL, DeviceWorkOwner::Script)
        .await?;
    let first_termination = control.terminate_app(&owner, &package).await;
    let termination = if first_termination.is_err() {
        tokio::time::sleep(Duration::from_secs(1)).await;
        control.terminate_app(&owner, &package).await
    } else {
        first_termination
    };
    let closed = control.close_exclusive_context(owner);
    let cleanup = control.shutdown_cleanup().await;
    save(
        &out,
        if mode == "link-only" {
            "link-cleanup.json"
        } else if mode == "link-scout" {
            "link-scout-cleanup.json"
        } else {
            "cleanup.json"
        },
        &serde_json::json!({"proof":termination.as_ref().ok(),"terminationError":termination.as_ref().err().map(ToString::to_string),"closeError":closed.as_ref().err().map(ToString::to_string),"cleanupError":cleanup.as_ref().err().map(ToString::to_string),"runError":result.as_ref().err().map(ToString::to_string)}),
    )?;
    result?;
    termination?;
    closed?;
    cleanup?;
    Ok(())
}
