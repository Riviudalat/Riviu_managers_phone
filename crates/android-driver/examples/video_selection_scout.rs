//! Rehearse one isolated album. Sound selection is reversible; Post is blocked by callback.
use anyhow::Context;
use riviu_android_driver::AndroidDriver;
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::tiktok_composer::{
    reach_picker, CarouselRequest, Composer, ComposerPlan, ComposerVerdict, Screen,
};
use riviu_core::tiktok_labels::controls_for;
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

#[path = "common/mod.rs"]
mod common;

async fn capture(session: &dyn UiSession, out: &std::path::Path, name: &str) -> anyhow::Result<()> {
    std::fs::write(
        out.join(format!("{name}.xml")),
        session.hierarchy_source_snapshot().await?.xml,
    )?;
    std::fs::write(
        out.join(format!("{name}.png")),
        session.screenshot_png().await?,
    )?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() == 4 && matches!(args[3].as_str(), "measure" | "rehearse"), "usage: video_selection_scout <serial> <single-video-bundle> <new-report-folder> <measure|rehearse>");
    let serial = &args[0];
    let out = PathBuf::from(&args[2]);
    anyhow::ensure!(!out.exists(), "use a new report folder");
    std::fs::create_dir_all(&out)?;
    let manifest = riviu_core::scan_publish_folder(&args[1], Default::default())?;
    anyhow::ensure!(manifest.bundles.len() == 1, "one bundle required");
    let bundle = &manifest.bundles[0];
    anyhow::ensure!(
        bundle.video.is_some() && bundle.images.is_empty(),
        "one video bundle required"
    );
    let staged = out.join("staged");
    riviu_core::copy_bundle_to_managed(bundle, &staged)?;
    let driver = Arc::new(AndroidDriver::new(&common::repo_config())?);
    let control = riviu_core::DeviceControlPlane::new(
        driver.clone(),
        Arc::new(riviu_core::DeviceWorkCoordinator::new()),
        Arc::new(riviu_core::StreamBudgetManager::new(1)?),
    );
    let lease = control
        .acquire_exclusive(serial, riviu_core::DeviceWorkOwner::Script)
        .await?;
    let package = driver.resolve_tiktok_package(serial).await?;
    let id = format!("carousel-scout-{}", uuid::Uuid::new_v4());
    let mut import_id = None;
    let result: anyhow::Result<()> = async {
        let staged_proof=driver.stage_publish_media(serial,"",&id,&staged).await?;
        let sha=staged_proof["manifestSha256"].as_str().context("manifest hash")?;
        let prepared=driver.prepare_publish_media(serial,&id,sha).await?;
        import_id=prepared["importId"].as_str().map(str::to_string);
        let imported=driver.import_publish_media(serial,&id,sha).await?;
        anyhow::ensure!(imported["files"].as_u64()==Some(1),"import count mismatch");
        std::fs::write(out.join("import.json"),serde_json::to_vec_pretty(&imported)?)?;
        driver.terminate_app(serial,&package).await?;
        driver.launch_app(serial,&package).await?;
        let session=driver.open_session(serial).await?;
        let version=session.app_version(&package).await.context("version")?;
        let locale=session.ui_language().await.context("locale")?;
        let labels=controls_for(&package,&locale,&version).context("unmeasured tuple")?;
        let (w,h)=riviu_core::screen::measured_screen_size(&session).await?;
        let screen=Screen::new(w,h).context("screen")?;
        let request=CarouselRequest {album:import_id.as_deref().context("import ID")?, images:1,caption:&bundle.caption,screen};
        let mut composer=Composer::new(&session,ComposerPlan::resolve(&labels)?,|node:&riviu_core::ElementBox|node.centre());
        let stop=AtomicBool::new(false);
        let observed: anyhow::Result<()> = async {
            let sound_plan=riviu_core::tiktok_sound::SoundPickerPlan::resolve(&package,&locale,&version).context("sound plan")?;
            if args[3]=="rehearse" {
                let video=riviu_core::tiktok_composer::VideoPickerPlan::resolve(&package,&locale,&version).context("video plan")?;
                let video_request=riviu_core::tiktok_composer::VideoRequest {album:request.album,caption:&bundle.caption,screen};
                let mut boundary=false;
                let result=riviu_core::tiktok_composer::publish_video_with_sound_effect_intent(&session,ComposerPlan::resolve(&labels)?,video,sound_plan,&riviu_core::PublishSoundPolicy::TrendingAny{pool_size:5,seed:2985670833},|n:&riviu_core::ElementBox|n.centre(),&video_request,&stop,|selection|{boundary=true;std::fs::write(out.join("sound.json"),serde_json::to_vec_pretty(selection)?)?;anyhow::bail!("video rehearsal stops before Post")}).await;
                std::fs::write(out.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({"reachedPostBoundary":boundary,"publicPost":false,"error":result.as_ref().err().map(|e|format!("{e:#}"))}))?)?;
                anyhow::ensure!(boundary&&result.is_err(),"video rehearsal did not reach Post: {result:?}");
            } else {
                anyhow::ensure!(reach_picker(&mut composer,&request,&stop).await?==ComposerVerdict::Stopped,"picker");
                capture(&session,&out,"video-picker").await?;
                let selector=match version.as_str(){"45.4.3"=>":id/k3j","45.7.3"=>":id/k_x","46.0.41"=>":id/kek","46.1.3"=>":id/kfk","46.2.1"=>":id/kir","46.4.3"=>":id/knq",_=>anyhow::bail!("unmeasured selector")};
                let rows=session.locate_all(riviu_core::ElementQuery::ResourceIdSuffix(selector)).await?;
                anyhow::ensure!(rows.len()==1,"one video selector required, got {}",rows.len());
                session.tap(rows[0].centre()).await?;
                tokio::time::sleep(Duration::from_secs(1)).await;
                capture(&session,&out,"video-selected").await?;
                let next_id=match version.as_str(){"45.4.3"=>":id/w86","45.7.3"=>":id/wjp","46.0.41"=>":id/wpw","46.1.3"=>":id/wrj","46.2.1"=>":id/wwo","46.4.3"=>":id/x4j",_=>anyhow::bail!("unmeasured Next")};
                let next=session.locate_all_described(riviu_core::ElementQuery::ResourceIdSuffix(next_id)).await?;
                anyhow::ensure!(next.len()==1&&next[0].enabled&&next[0].description.as_deref().is_some_and(|s|s=="Next (1)"),"video count readback");
                session.tap(next[0].centre()).await?;
                tokio::time::sleep(Duration::from_secs(3)).await;
                capture(&session,&out,"video-editor").await?;
                let pool=riviu_core::tiktok_sound::open_and_observe_sounds(&session,sound_plan,5).await?;
                let mut selection=riviu_core::publish::select_sound_candidate(&riviu_core::PublishSoundPolicy::TrendingAny{pool_size:5,seed:2985670833},&pool.candidates)?;
                riviu_core::tiktok_sound::choose_and_confirm_sound(&session,sound_plan,&pool,selection.index).await?;
                selection.confirmed=true;
                capture(&session,&out,"video-sound").await?;
                let mut boundary=false;
                let result=riviu_core::tiktok_composer::continue_from_edit_step_with_effect_intent(&mut composer,&bundle.caption,&stop,&mut || {boundary=true;anyhow::bail!("video measurement stops before Post")}).await;
                capture(&session,&out,"video-caption").await?;
                std::fs::write(out.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({"version":version,"reachedPostBoundary":boundary,"publicPost":false,"error":result.as_ref().err().map(|e|format!("{e:#}")),"sound":selection}))?)?;
                anyhow::ensure!(boundary&&result.is_err(),"video caption boundary: {result:?}");
            }
            Ok(())
        }.await;
        composer.leave().await;
        observed
    }.await;
    let terminated = driver.terminate_app(serial, &package).await;
    let media = if let Some(id) = import_id {
        Some(driver.cleanup_publish_media(serial, &id).await)
    } else {
        None
    };
    control.close_exclusive_context(lease)?;
    let cleanup = control.shutdown_cleanup().await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut final_process = driver.inspect_app_process(serial, &package).await;
    if final_process.as_ref().is_ok_and(|state| state.running) {
        driver.terminate_app(serial, &package).await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        final_process = driver.inspect_app_process(serial, &package).await;
    }
    std::fs::write(
        out.join("cleanup.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"error":result.as_ref().err().map(|e|format!("{e:#}")),"process":terminated.as_ref().ok(),"finalProcess":final_process.as_ref().ok().map(|p|serde_json::json!({"running":p.running,"pid":p.pid})),"media":media.as_ref().and_then(|r|r.as_ref().ok()),"cleanupError":cleanup.as_ref().err().map(ToString::to_string)}),
        )?,
    )?;
    result?;
    terminated?;
    if let Some(media) = media {
        media?;
    }
    cleanup?;
    anyhow::ensure!(!final_process?.running, "TikTok restarted after cleanup");
    println!("selection scout complete; no Post; temporary media cleaned");
    Ok(())
}
