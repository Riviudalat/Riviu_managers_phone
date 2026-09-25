//! Isolated, explicitly opted-in scout/rehearsal. Not post-to-Sheet acceptance.
//! Production acceptance uses scripts/publish_acceptance.mjs and the existing AppState.
use super::*;
use riviu_core::driver::{DeviceDriver, ElementQuery};

fn save(out: &Path, name: &str, value: &impl serde::Serialize) -> anyhow::Result<()> {
    fs::write(out.join(name), serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn validate_canary_mode(mode: &str, isolated: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        matches!(mode, "inspect" | "sound-scout" | "rehearse" | "link-scout"),
        "public/execute canary disabled: use production IPC acceptance; scratch DB has no verifier/Sheet worker"
    );
    anyhow::ensure!(isolated, "set RIVIU_PUBLISH_CANARY_ISOLATED=confirmed-no-production-owner only for an isolated scout device");
    Ok(())
}

#[test]
fn isolated_canary_rejects_public_and_legacy_execute_modes() {
    for mode in ["publish-one", "link-only"] {
        assert!(
            validate_canary_mode(mode, true).is_err(),
            "{mode} must use the production IPC harness"
        );
    }
}

#[test]
fn isolated_canary_requires_explicit_scope_before_driver_creation() {
    for mode in ["inspect", "sound-scout", "rehearse", "link-scout"] {
        assert!(validate_canary_mode(mode, false).is_err());
        assert!(validate_canary_mode(mode, true).is_ok());
    }
}

#[tokio::test]
#[ignore = "Isolated scout only; requires explicit mode, device/account/bundle and no production owner"]
async fn live_publish_canary() -> anyhow::Result<()> {
    let mode = std::env::var("RIVIU_PUBLISH_CANARY")?;
    validate_canary_mode(
        &mode,
        std::env::var("RIVIU_PUBLISH_CANARY_ISOLATED").as_deref()
            == Ok("confirmed-no-production-owner"),
    )?;
    let serial = std::env::var("RIVIU_PUBLISH_UDID")?;
    let handle = std::env::var("RIVIU_PUBLISH_HANDLE")?;
    let bundle_name = std::env::var("RIVIU_PUBLISH_BUNDLE")?;
    anyhow::ensure!(
        !serial.trim().is_empty() && !handle.trim().is_empty() && !bundle_name.trim().is_empty(),
        "explicit device/account/bundle required"
    );
    let source = PathBuf::from(std::env::var("RIVIU_PUBLISH_SOURCE")?).canonicalize()?;
    let out = PathBuf::from(std::env::var("RIVIU_PUBLISH_REPORT")?);
    fs::create_dir_all(&out)?;
    anyhow::ensure!(
        matches!(mode.as_str(), "link-scout") || !out.join("canary.db").exists(),
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
    let result: anyhow::Result<()> = async {
    registry.upsert_many(
        driver
            .list_devices()
            .await?
            .into_iter()
            .filter(|device| device.udid == serial.as_str())
            .collect(),
    );
    anyhow::ensure!(registry.get(serial.as_str()).is_some(), "canary phone absent");
    let db = Arc::new(Database::open(out.join("canary.db"))?);
    save(&out, "scope.json", &serde_json::json!({"scope":"isolatedScoutOnly","sheetEnabled":false,"endToEndAcceptance":false,"mode":mode,"udid":serial}))?;
        if mode == "link-scout" {
            let campaign: PublishCampaignRecord = serde_json::from_slice(&fs::read(out.join("campaign.json"))?)?;
            let detail = db.get_publish_campaign(&campaign.id)?.context("missing canary")?;
            anyhow::ensure!(
                detail.assignments.len() == 1
                    && detail.assignments[0].udid == serial.as_str()
                    && matches!(
                        detail.assignments[0].state,
                        riviu_core::PublishCampaignState::Verifying
                            | riviu_core::PublishCampaignState::Succeeded
                    ),
                "link retry requires exactly the verifying or confirmed canary; never Post"
            );
            {
                // Warm observer only: open_publish_context would cold-start a pending upload.
                let context = control.open_manual_session(serial.as_str(), DeviceWorkOwner::Script).await?;
                let observed: anyhow::Result<()> = async {
                    let session = control.session(&context)?;
                    let package = control.resolve_tiktok_package(serial.as_str()).await?;
                    control.foreground_session_app(&context, &package).await?;
                    let (package,version,locale) = control.tiktok_build(serial.as_str()).await?;
                    let labels = riviu_core::tiktok_labels::controls_for(&package,&locale,&version).context("labels")?;
                    let profile = labels.label(riviu_core::tiktok_labels::TikTokControl::ProfileTab).context("profile")?;
                    let tab = session.locate(profile.to_query()).await?.context("profile tab")?;
                    session.tap(tab.centre()).await?;
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    let handles = session.locate_all_described(ElementQuery::Text { value: handle.as_str(), exact: true }).await?;
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
                    let mut stream=FrameSource::subscribe(frames.as_ref(),serial.as_str());
                    if let Ok(Some(frame))=tokio::time::timeout(Duration::from_secs(5),stream.next()).await { fs::write(out.join("owned-post.jpg"),frame.as_ref())?; }
                    let mark=format!("riviu-link-scout-{}",Uuid::new_v4());
                    let primed=session.set_clipboard("plaintext",mark.as_bytes()).await;
                    save(&out,"clipboard-prime.json",&serde_json::json!({"error":primed.as_ref().err().map(|e|format!("{e:#}"))}))?;
                    primed?;
                    let share=session.locate(labels.label(riviu_core::tiktok_labels::TikTokControl::Share).context("share")?.to_query()).await?.context("share absent")?;
                    session.tap(share.centre()).await?;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    fs::write(out.join("share-sheet.xml"),session.hierarchy_source_snapshot().await?.xml)?;
                    let mut stream=FrameSource::subscribe(frames.as_ref(),serial.as_str());
                    if let Ok(Some(frame))=tokio::time::timeout(Duration::from_secs(5),stream.next()).await { fs::write(out.join("share-sheet.jpg"),frame.as_ref())?; }
                    Ok(())
                }.await;
                let closed=control.close_manual_session(context);
                observed?;closed?;
                return Ok(());
            }
        }
        let manifest = scan_publish_folder(&source, PublishScanOptions::default())?;
        save(&out, "manifest.json", &manifest)?;
        let bundle = manifest.bundles.iter().find(|bundle| bundle.name == bundle_name).context("requested first image bundle missing")?;
        let request = riviu_core::PublishPreflightRequest {
        network: riviu_core::SocialNetwork::TikTok,
        delete_after_publish: true,
            source_root: source.to_string_lossy().to_string(), bundle_ids: vec![bundle.id.clone()],
            udids: vec![serial.as_str().into()], target_ref: Some(riviu_core::TargetRef::Explicit { udids: vec![serial.as_str().into()] }),
            run_at: None, caption_overrides: Default::default(),
            sound_policy: riviu_core::PublishSoundPolicy::TrendingAny { pool_size: 5, seed: 20260906 },
            sheet_enabled: false,
        };
        let prepared = build_publish_preflight(&control, &registry, &db, request.clone()).await?;
        save(&out, "preflight.json", &prepared.report)?;
        anyhow::ensure!(prepared.report.can_execute, "preflight failed; inspect report");
        let context = open_publish_context(&control, serial.as_str()).await?;
        let session = control.streaming_session(&context)?;
        let account_result: anyhow::Result<()> = async {
            let (package, version, locale) = control.tiktok_build(serial.as_str()).await?;
            let labels = riviu_core::tiktok_labels::controls_for(&package, &locale, &version).context("unmeasured build")?;
            let profile = labels.label(riviu_core::tiktok_labels::TikTokControl::ProfileTab).context("profile locator missing")?;
            let tab = session.locate(profile.to_query()).await?.context("profile tab absent")?;
            session.tap(tab.centre()).await?;
            tokio::time::sleep(Duration::from_secs(3)).await;
            let snapshot = session.hierarchy_source_snapshot().await?;
            fs::write(out.join("account.xml"), snapshot.xml)?;
            if let Some(frame) = frames.latest(serial.as_str()) { fs::write(out.join("account.jpg"), frame.as_ref())?; }
            let observed = riviu_core::tiktok_account::observe_own_account(session.as_ref(), labels).await?;
            anyhow::ensure!(observed.as_deref() == Some(handle.trim().trim_start_matches('@')), "canary account not proven");
            save(&out, "account.json", &serde_json::json!({"serial":serial.as_str(),"handle":handle.as_str(),"package":package,"version":version,"locale":locale,"proved":true}))?;
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
                        sheet_delivery: None,
            verification_contract_version: Some(1),
            verification_builds: riviu_core::publish_submission::builds_from_preflight(&prepared.report),
            request_id: id, source_root: request.source_root, bundle_ids: vec![managed.id.clone()],
            udids: request.udids, run_at: None, visibility: PublishVisibility::Public,
            cleanup_policy: PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
            network: riviu_core::SocialNetwork::TikTok,
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
            let context = open_publish_context(&control, serial.as_str()).await?;
            let session = control.streaming_session(&context)?;
            if mode == "sound-scout" {
                let observed: anyhow::Result<()> = async {
                    use riviu_core::tiktok_composer::{Composer, ComposerPlan, Screen, CarouselRequest, reach_edit_step};
                    let (package, version, locale) = control.tiktok_build(serial.as_str()).await?;
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
                    let mut stream = FrameSource::subscribe(frames.as_ref(), serial.as_str());
                    if let Ok(Some(frame)) = tokio::time::timeout(Duration::from_secs(5), stream.next()).await { fs::write(out.join("sound-picker.jpg"), frame.as_ref())?; }
                    save(&out, "sound-pool.json", &serde_json::json!({"candidates":pool.as_ref().ok().map(|p| &p.candidates),"error":pool.as_ref().err().map(ToString::to_string)}))?;
                    let pool = pool?;
                    let selection = riviu_core::publish::select_sound_candidate(&campaign_request.sound_policy, &pool.candidates)?;
                    let selected = riviu_core::tiktok_sound::choose_and_confirm_sound(session.as_ref(), plan, &pool, selection.index).await;
                    fs::write(out.join("production-selection.xml"), session.hierarchy_source_snapshot().await?.xml)?;
                    let mut stream = FrameSource::subscribe(frames.as_ref(), serial.as_str());
                    if let Ok(Some(frame)) = tokio::time::timeout(Duration::from_secs(5), stream.next()).await { fs::write(out.join("production-selection.jpg"), frame.as_ref())?; }
                    save(&out, "production-selection.json", &serde_json::json!({"selection":selection,"error":selected.as_ref().err().map(|e|format!("{e:#}"))}))?;
                    selected?;
                    Ok(())
                }.await;
                let cleaned = control.cleanup_publish_media_with_ui(&context, &import).await;
                let closed = control.close_ui_context(context).await;
                save(&out, "media-cleanup.json", &serde_json::json!({"cleanup":cleaned.as_ref().ok(),"error":cleaned.as_ref().err().map(ToString::to_string)}))?;
                observed?;
                cleaned?;
                closed?;
                return Ok(());
            }
            db.update_publish_campaign_state(&campaign.id, riviu_core::PublishCampaignState::Posting, None)?;
            let mut reached_post = false;
            let mut before_post = |selection: Option<&riviu_core::SoundSelectionEvidence>, _identity:Option<&riviu_core::publish_submission::PublishSubmissionProof>| {
                reached_post = true;
                save(&out, "sound.json", &selection)?;
                anyhow::bail!("canary rehearsal: stop before public Post")
            };
            let outcome = post_through_the_composer(&control, &db, &assignment.id, session.as_ref(), &campaign.id, serial.as_str(), bundle, &import, &campaign_request.sound_policy, false, &mut before_post, &|_| {}, &|_| {}).await;
            let definitely_not_posted = matches!(&outcome, PostOutcome::NothingPublished(_));
            let description = match outcome { PostOutcome::NothingPublished(reason) => reason, PostOutcome::Unknown(reason) => format!("UNCERTAIN {reason}"), PostOutcome::Posted(_) | PostOutcome::Submitted(_) => "UNEXPECTED POST".into() };
            let artifacts: anyhow::Result<()> = async {
                if let Ok(snapshot) = session.hierarchy_source_snapshot().await { fs::write(out.join("composer.xml"), snapshot.xml)?; }
                if let Some(frame) = frames.latest(serial.as_str()) { fs::write(out.join("composer.jpg"), frame.as_ref())?; }
                Ok(())
            }.await;
            let cleaned = if definitely_not_posted {
                control.cleanup_publish_media_with_ui(&context, &import).await.map_err(anyhow::Error::new)
            } else {
                Err(anyhow::anyhow!("unknown upload: keep media and TikTok"))
            };
            let closed = control.close_ui_context(context).await;
            save(&out, "rehearsal.json", &serde_json::json!({"reachedPost":reached_post,"outcome":description,"cleanup":cleaned.as_ref().ok(),"cleanupError":cleaned.as_ref().err().map(ToString::to_string)}))?;
            closed?;
            artifacts?;
            cleaned?;
            anyhow::ensure!(reached_post, "rehearsal stopped before the Post boundary: {description}");
            return Ok(());
        }
        anyhow::bail!("unsupported isolated scout mode; no dispatcher or publication worker started")
    }.await;
    // Scratch state cannot prove that TikTok has no upload. Never force-stop it,
    // even after a timeout or a failed DB/artifact write. Join only our own cleanup.
    let cleanup = control.shutdown_cleanup().await;
    let saved = save(
        &out,
        "cleanup.json",
        &serde_json::json!({"scope":"isolatedScoutOnly","sheetEnabled":false,"endToEndAcceptance":false,"tiktokTerminated":false,"cleanupError":cleanup.as_ref().err().map(ToString::to_string),"runError":result.as_ref().err().map(ToString::to_string)}),
    );
    result?;
    cleanup?;
    saved?;
    Ok(())
}
