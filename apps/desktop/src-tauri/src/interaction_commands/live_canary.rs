//! One explicitly selected Android and post. Default acceptance is read-only.
use anyhow::Context;
use riviu_core::driver::{DeviceDriver, ElementQuery};
use riviu_core::{DeviceControlPlane, DeviceWorkCoordinator, FrameSource, StreamBudgetManager};
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;

const SERIAL: &str = "9889db374744474635";
const HANDLE: &str = "@ghin.lt.sng.sng";
const URL: &str = "https://www.tiktok.com/@ghin.lt.sng.sng/photo/7682384619777297672";
const COMMENT: &str = "Lịch trình chia theo ngày dễ theo dõi.";

fn save(out: &Path, name: &str, value: &impl serde::Serialize) -> anyhow::Result<()> {
    std::fs::write(out.join(name), serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn chosen_serial<'a>(mode: &str, requested: Option<&'a str>) -> anyhow::Result<&'a str> {
    if mode != "target-proof-only" {
        anyhow::ensure!(
            requested.is_none(),
            "RIVIU_TARGET_PROOF_SERIAL is only valid in target-proof-only mode"
        );
        return Ok(SERIAL);
    }
    let serial = requested.unwrap_or(SERIAL);
    anyhow::ensure!(
        !serial.is_empty()
            && !serial
                .chars()
                .any(|ch| ch.is_control() || ch.is_whitespace()),
        "target-proof serial must be nonempty and contain no whitespace or control characters"
    );
    Ok(serial)
}

async fn target_proof_only(
    control: &DeviceControlPlane,
    serial: &str,
    target: &riviu_core::ResolvedTikTokTarget,
    out: &Path,
) -> anyhow::Result<()> {
    let device =
        riviu_core::interaction_campaign::open_clean_interaction_context(control, serial).await?;
    let package = device.target_package;
    let context = device.context;
    let inspected: anyhow::Result<()> = async {
        let session = control.streaming_session(&context)?;
        let (observed_package, version, locale) = control.tiktok_build(serial).await?;
        anyhow::ensure!(
            observed_package == package,
            "TikTok package changed after clean start"
        );
        let labels = riviu_core::tiktok_labels::controls_for(&package, &locale, &version)
            .context("unmeasured target-proof tuple")?;
        save(
            out,
            "target-proof-device.json",
            &serde_json::json!({
                "serial": serial,
                "package": package,
                "version": version,
                "locale": locale,
                "target": target.normalized_url,
                "mode": "target-proof-only",
                "publicActionsEnabled": false,
            }),
        )?;
        let stop = AtomicBool::new(false);
        let mut errors = Vec::new();
        for attempt in 1..=2 {
            let started_at = chrono::Utc::now().to_rfc3339();
            let started = std::time::Instant::now();
            let arrival = riviu_core::interaction_hierarchy::open_exact_target_by_hierarchy(
                session.as_ref(),
                labels,
                &package,
                target,
                &stop,
            )
            .await
            .and_then(|arrival| {
                anyhow::ensure!(
                    matches!(
                        arrival,
                        riviu_core::interaction_hierarchy::TargetArrival::Identified { .. }
                    ),
                    "target-proof-only requires exact identification"
                );
                Ok(arrival)
            });
            let elapsed_ms = started.elapsed().as_millis();
            let hierarchy: anyhow::Result<String> = async {
                let xml = session.hierarchy_source_snapshot().await?.xml;
                let name = format!("target-proof-{attempt}.xml");
                std::fs::write(out.join(&name), xml)?;
                Ok(name)
            }
            .await;
            let screenshot: anyhow::Result<String> = async {
                let png = session.screenshot_png().await?;
                let name = format!("target-proof-{attempt}.png");
                std::fs::write(out.join(&name), png)?;
                Ok(name)
            }
            .await;
            save(
                out,
                &format!("target-proof-{attempt}.json"),
                &serde_json::json!({
                    "serial": serial,
                    "attempt": attempt,
                    "target": target.normalized_url,
                    "startedAt": started_at,
                    "elapsedMs": elapsed_ms,
                    "arrival": arrival.as_ref().ok().map(|value| format!("{value:?}")),
                    "error": arrival.as_ref().err().map(|error| format!("{error:#}")),
                    "hierarchy": hierarchy.as_ref().ok(),
                    "hierarchyError": hierarchy.as_ref().err().map(|error| format!("{error:#}")),
                    "screenshot": screenshot.as_ref().ok(),
                    "screenshotError": screenshot.as_ref().err().map(|error| format!("{error:#}")),
                    "publicActionsEnabled": false,
                }),
            )?;
            if let Err(error) = arrival {
                errors.push(format!("proof {attempt}: {error:#}"));
            }
            if let Err(error) = hierarchy {
                errors.push(format!("hierarchy {attempt}: {error:#}"));
            }
            if let Err(error) = screenshot {
                errors.push(format!("screenshot {attempt}: {error:#}"));
            }
        }
        anyhow::ensure!(errors.is_empty(), "{}", errors.join("; "));
        Ok(())
    }
    .await;
    let finished = control.finish_app_session(context, &package).await;
    save(
        out,
        "target-proof-finish.json",
        &serde_json::json!({
            "serial": serial,
            "proof": finished.as_ref().ok(),
            "cleanupError": finished.as_ref().err().map(|error| format!("{error:#}")),
            "error": inspected.as_ref().err().map(|error| format!("{error:#}")),
        }),
    )?;
    inspected?;
    finished?;
    Ok(())
}

#[tokio::test]
#[ignore = "Explicit canary; inspect is read-only, public-once requires separate confirmation"]
async fn live_interaction_canary() -> anyhow::Result<()> {
    let mode = std::env::var("RIVIU_INTERACTION_CANARY")?;
    let url = if mode == "target-proof-only" {
        std::env::var("RIVIU_INTERACTION_URL")
            .context("target-proof-only requires an explicit URL")?
    } else {
        std::env::var("RIVIU_INTERACTION_URL").unwrap_or_else(|_| URL.into())
    };
    let comment = std::env::var("RIVIU_INTERACTION_COMMENT").unwrap_or_else(|_| COMMENT.into());
    anyhow::ensure!(
        matches!(
            mode.as_str(),
            "inspect"
                | "account-only"
                | "inspection-apis"
                | "lifecycle-only"
                | "target-proof-only"
                | "public-once"
                | "comment-once"
        ),
        "unknown mode"
    );
    let requested_serial = match std::env::var("RIVIU_TARGET_PROOF_SERIAL") {
        Ok(serial) => Some(serial),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    let effective_serial = chosen_serial(&mode, requested_serial.as_deref())?;
    let out = PathBuf::from(std::env::var("RIVIU_INTERACTION_REPORT")?);
    std::fs::create_dir_all(&out)?;
    anyhow::ensure!(
        !out.join("canary.db").exists(),
        "do not replay a canary output directory"
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
    anyhow::ensure!(
        driver
            .list_devices()
            .await?
            .iter()
            .any(|d| d.udid == effective_serial),
        "canary absent: {effective_serial}"
    );
    let frames = Arc::new(riviu_ios_driver::StreamHub::new());
    driver.set_frame_sink(Arc::new(frames.as_ref().clone()));
    let control = Arc::new(DeviceControlPlane::new(
        driver,
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(StreamBudgetManager::new(1)?),
    ));
    let db = Arc::new(riviu_core::db::Database::open(out.join("canary.db"))?);
    let target = riviu_core::parse_tiktok_links(&url)
        .remove(0)
        .target
        .context("target")?;
    let result: anyhow::Result<()> = async {
        if mode=="target-proof-only" {
            return target_proof_only(&control,effective_serial,&target,&out).await;
        }
        if mode=="lifecycle-only" {
            let package=control.resolve_tiktok_package(SERIAL).await?;
            for workflow in ["interaction","publish"] {
                let lease=control.acquire_exclusive(SERIAL,riviu_core::DeviceWorkOwner::Script).await?;
                control.launch_app(&lease,&package).await?;
                let before=control.inspect_app_process(&lease,&package).await?;
                control.close_exclusive_context(lease)?;
                let context=if workflow=="interaction" {
                    riviu_core::interaction_campaign::open_clean_interaction_context(&control,SERIAL).await?.context
                } else { crate::publish_commands::open_publish_context(&control,SERIAL).await? };
                let active=control.inspect_streaming_app_process(&context,&package).await?;
                let finished=control.finish_app_session(context,&package).await;
                save(&out,&format!("{workflow}-lifecycle.json"),&serde_json::json!({"before":{"running":before.running,"pid":before.pid},"opened":{"running":active.running,"pid":active.pid},"finished":finished.as_ref().ok(),"error":finished.as_ref().err().map(ToString::to_string)}))?;
                finished?;
                anyhow::ensure!(before.running && active.running && before.pid!=active.pid,"clean start did not replace process");
            }
            let engine=riviu_core::NurtureEngine::new(db.clone(),control.clone(),frames.clone(),out.join("artifacts"));
            let settings=riviu_core::NurtureSettings {num_videos:1,num_rounds:1,watch_min:1.0,watch_max:1.0,like_prob:0,comment_prob:0,save_prob:0,follow_prob:0,frenzy_prob:0,..Default::default()};
            db.save_nurture_settings(&settings)?;
            let status=engine.run_session(SERIAL,settings,Arc::new(AtomicBool::new(false)),Some(Duration::from_secs(12)), |_|{}).await?;
            save(&out,"nurture-lifecycle.json",&status)?;
            anyhow::ensure!(status.cleanup_proof.is_some() && status.cleanup_error.is_none(),"nurture cleanup unproven");
            anyhow::ensure!(status.likes==0 && status.comments==0 && status.saves==0 && status.follows==0,"unexpected public action");
            return Ok(());
        }
        if mode=="inspection-apis" {
            db.set_device_handle(SERIAL,"",HANDLE)?;
            let account=super::inspection::read_account(&control,&db,SERIAL).await?;
            anyhow::ensure!(account.status=="matched","account API mismatch");
            save(&out,"account-api.json",&account)?;
            let source=PathBuf::from(std::env::var("RIVIU_READBACK_DB")?);
            let copy=out.join("readback.db"); std::fs::copy(source,&copy)?;
            let source=riviu_core::db::Database::open(copy)?;
            let report=super::inspection::readback(&control,&source,&std::env::var("RIVIU_READBACK_CAMPAIGN")?,&std::env::var("RIVIU_READBACK_ASSIGNMENT")?).await?;
            save(&out,"readback-api.json",&report)?;
            anyhow::ensure!(report.like=="present" && report.save==riviu_core::BookmarkState::Saved,"readback did not confirm existing effects");
            return Ok(());
        }
        let device = riviu_core::interaction_campaign::open_interaction_context(&control,SERIAL).await?;
        let context = device.context;
        let session = control.streaming_session(&context)?;
        let inspected: anyhow::Result<()> = async {
            let (package,version,locale) = control.tiktok_build(SERIAL).await?;
            let labels = riviu_core::tiktok_labels::controls_for(&package,&locale,&version).context("unmeasured build")?;
            let profile = labels.label(riviu_core::tiktok_labels::TikTokControl::ProfileTab).context("profile locator")?;
            let tab = session.locate(profile.to_query()).await?.context("profile absent")?;
            session.tap(tab.centre()).await?;
            tokio::time::sleep(Duration::from_secs(3)).await;
            anyhow::ensure!(session.locate_all_described(ElementQuery::Text {value:HANDLE,exact:true}).await?.len()==1,"wrong account");
            save(&out,"account.json",&serde_json::json!({"serial":SERIAL,"handle":HANDLE,"verified":true}))?;
            let observed = riviu_core::tiktok_account::observe_own_account(session.as_ref(),labels).await?;
            save(&out,"account-reader.json",&serde_json::json!({"observedHandle":observed,"expectedHandle":HANDLE}))?;
            anyhow::ensure!(observed.as_deref()==Some(HANDLE.trim_start_matches('@')), "production account reader mismatch");
            std::fs::write(out.join("profile.xml"),session.hierarchy_source_snapshot().await?.xml)?;
            if mode=="account-only" { return Ok(()); }
            let opened = riviu_core::interaction_hierarchy::open_target_by_hierarchy(session.as_ref(),labels,&package,&url,&target.author,&AtomicBool::new(false)).await;
            if let Err(ref error) = opened {
                save(&out,"arrival.json",&serde_json::json!({"publicAllowed":false,"error":error.code(),"url":url}))?;
                std::fs::write(out.join("arrival-error.xml"),session.hierarchy_source_snapshot().await?.xml)?;
                let mut stream = FrameSource::subscribe(frames.as_ref(),SERIAL);
                if let Ok(Some(frame)) = tokio::time::timeout(Duration::from_secs(5),stream.next()).await {std::fs::write(out.join("arrival-error.jpg"),frame.as_ref())?;}
            }
            let mut arrival = opened.map_err(|e|anyhow::anyhow!(e.code()))?;
            if matches!(arrival, riviu_core::interaction_hierarchy::TargetArrival::Structural) {
                let proof = riviu_core::interaction_hierarchy::confirm_target_from_share_link(session.as_ref(),labels,&target).await;
                save(&out,"link-proof.json",&serde_json::json!({"proved":proof.is_ok(),"error":proof.as_ref().err().map(|e|format!("{e:#}"))}))?;
                arrival = proof?;
            }
            save(&out,"arrival.json",&serde_json::json!({"arrival":format!("{arrival:?}"),"publicAllowed":matches!(arrival,riviu_core::interaction_hierarchy::TargetArrival::Identified {..})}))?;
            use riviu_core::tiktok_labels::TikTokControl;
            let like = session.locate(labels.label(TikTokControl::Like).context("like locator")?.to_query()).await?;
            let liked = session.locate(labels.label(TikTokControl::Liked).context("liked locator")?.to_query()).await?;
            let bookmark = session.locate_stateful(labels.label(TikTokControl::Bookmark).context("save locator")?.to_query()).await?;
            let saved = riviu_core::SaveAdapter::observe(&mut riviu_core::HierarchySaveAdapter::new(session.as_ref(),labels)).await?;
            let counters = riviu_core::interaction_hierarchy::read_post_counters(session.as_ref(),labels).await;
            save(&out,"observation.json",&serde_json::json!({"serial":SERIAL,"account":HANDLE,"url":url,"package":package,"version":version,"locale":locale,
                "likeControl":like.as_ref().map(|e|&e.description),"likedControl":liked.as_ref().map(|e|&e.description),
                "bookmark":bookmark.as_ref().map(|e|serde_json::json!({"checked":e.checked,"selected":e.selected,"description":e.element.description})),"saveState":saved.state,"counters":format!("{counters:?}")}))?;
            std::fs::write(out.join("target.xml"),session.hierarchy_source_snapshot().await?.xml)?;
            let mut stream = FrameSource::subscribe(frames.as_ref(),SERIAL);
            if let Ok(Some(frame)) = tokio::time::timeout(Duration::from_secs(5),stream.next()).await {std::fs::write(out.join("target.jpg"),frame.as_ref())?;}
            anyhow::ensure!(matches!(arrival,riviu_core::interaction_hierarchy::TargetArrival::Identified {..}),"target not proven");
            if mode == "comment-once" {
                anyhow::ensure!(liked.is_some() && saved.state==riviu_core::BookmarkState::Saved,"read back existing Like and Save before comment; never retry those toggles");
            }
            Ok(())
        }.await;
        let closed = control.close_ui_context(context).await;
        inspected?;closed?;
        if matches!(mode.as_str(), "inspect" | "account-only") {return Ok(());}
        let request = riviu_core::ThreadCampaignRequest {
            scripted_conversation: None,
            request_id:uuid::Uuid::new_v4().to_string(),targets:vec![target],actor_udids:vec![SERIAL.into()],message_count:1,
            instruction:String::new(),max_words:12,mode:riviu_core::ThreadMode::Standalone,shape:riviu_core::ThreadShape::Star,
            cohort_size:None,manual_comments:vec![comment],actions:riviu_core::InteractionActionSet {like:mode=="public-once",save:mode=="public-once",comment:true},mentions:vec![],mention_parent:false,
        };
        let plan = riviu_core::plan_threads(&request)?;
        let campaign = db.create_interaction_campaign(&request,&plan)?;
        db.update_interaction_campaign_state(&campaign,riviu_core::ThreadCampaignState::Running,None)?;
        save(&out,"request.json",&request)?;
        let engine = riviu_core::NurtureEngine::new(db.clone(),control.clone(),frames.clone(),out.join("artifacts"));
        let executed = riviu_core::interaction_campaign::execute_thread_campaign(db.clone(),control.clone(),engine,riviu_core::EventBus::new(128),campaign.clone(),request,plan,None,riviu_core::FlowArtifactStore::new(out.join("artifacts"))?,frames).await;
        let detail = db.get_interaction_campaign(&campaign)?.context("campaign missing")?;
        save(&out,"result.json",&detail)?;
        executed?;
        anyhow::ensure!(detail.assignments.len()==1 && detail.assignments[0].state==riviu_core::ThreadMessageState::Succeeded,"interaction not confirmed; no automatic retry");
        Ok(())
    }.await;
    let package = control.resolve_tiktok_package(effective_serial).await?;
    let owner = control
        .try_acquire_exclusive(effective_serial, riviu_core::DeviceWorkOwner::Script)
        .await?;
    let mut stopped = control.terminate_app(&owner, &package).await;
    if stopped.is_err() {
        tokio::time::sleep(Duration::from_secs(1)).await;
        stopped = control.terminate_app(&owner, &package).await;
    }
    let closed = control.close_exclusive_context(owner);
    let cleanup = control.shutdown_cleanup().await;
    save(
        &out,
        "cleanup.json",
        &serde_json::json!({"serial":effective_serial,"proof":stopped.as_ref().ok(),"stopError":stopped.as_ref().err().map(ToString::to_string),"closeError":closed.as_ref().err().map(ToString::to_string),"cleanupError":cleanup.as_ref().err().map(ToString::to_string),"error":result.as_ref().err().map(ToString::to_string)}),
    )?;
    result?;
    stopped?;
    closed?;
    cleanup?;
    Ok(())
}

#[test]
fn target_proof_serial_override_is_validated_and_never_changes_public_modes() {
    assert_eq!(chosen_serial("target-proof-only", None).unwrap(), SERIAL);
    for serial in [
        "98895a3355424e484f",
        "ce021822e3f548f40b",
        "192.168.1.8:5555",
    ] {
        assert_eq!(
            chosen_serial("target-proof-only", Some(serial)).unwrap(),
            serial
        );
    }
    for serial in ["", " ", "a b", "a\nb", "a\rb", "a\tb", "a\0b", "a\u{7f}b"] {
        assert!(chosen_serial("target-proof-only", Some(serial)).is_err());
    }
    for mode in [
        "public-once",
        "comment-once",
        "inspect",
        "account-only",
        "inspection-apis",
        "lifecycle-only",
    ] {
        assert_eq!(chosen_serial(mode, None).unwrap(), SERIAL);
        assert!(chosen_serial(mode, Some("ce021822e3f548f40b")).is_err());
        assert!(chosen_serial(mode, Some(SERIAL)).is_err());
    }
}
