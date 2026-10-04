//! Rehearse one isolated album. Sound selection is reversible; Post is blocked by callback.
use anyhow::Context;
use riviu_android_driver::{AdbProgram, AndroidDriver};
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::tiktok_composer::{
    reach_edit_step, reach_picker, CarouselRequest, Composer, ComposerPlan, ComposerVerdict,
    PhotoGrid, Screen,
};
use riviu_core::tiktok_labels::{controls_for, TikTokControl};
use std::{
    fs::{FileTimes, OpenOptions},
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, UNIX_EPOCH},
};

#[path = "common/mod.rs"]
mod common;

async fn capture(session: &dyn UiSession, out: &std::path::Path, name: &str) -> anyhow::Result<()> {
    std::fs::write(
        out.join(format!("{name}.xml")),
        session.hierarchy_source_snapshot().await?.xml,
    )?;
    match session.screenshot_png().await {
        Ok(png) => std::fs::write(out.join(format!("{name}.png")), png)?,
        Err(error) if error.is::<riviu_core::driver::ScreenshotReadUnavailable>() => {
            std::fs::write(
                out.join(format!("{name}.capture-unavailable.json")),
                serde_json::to_vec(&serde_json::json!({
                    "reason": "screenshot_unavailable",
                    "xmlAvailable": true,
                }))?,
            )?;
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

async fn capture_media_store(
    adb: &AdbProgram,
    serial: &str,
    out: &std::path::Path,
    imported: &serde_json::Value,
) -> anyhow::Result<()> {
    let album = imported["albumId"].as_str().context("import album ID")?;
    let resolved_album = album.replacen("/sdcard", "/storage/emulated/0", 1);
    let refs = imported["mediaRefs"].as_array().context("import media refs")?;
    let mut rows = Vec::with_capacity(refs.len());
    for media in refs {
        anyhow::ensure!(media["collection"] == "images", "expected image MediaStore row");
        let id = media["id"].as_str().context("image MediaStore ID")?;
        anyhow::ensure!(id.parse::<u64>().is_ok(), "non-numeric image MediaStore ID");
        let path = media["path"].as_str().context("image MediaStore path")?;
        anyhow::ensure!(
            path.starts_with(&format!("{album}/"))
                || path.starts_with(&format!("{resolved_album}/")),
            "image outside imported album"
        );
        let command = format!(
            "content query --uri content://media/external/images/media/{id} --projection _id:_data:date_added:date_modified:datetaken 2>&1"
        );
        let receipt = match adb.shell_output(serial, &command, Duration::from_secs(15)).await {
            Ok(response) => serde_json::json!({
                "exitCode": response.exit_code,
                "stdout": response.stdout,
                "stderr": response.stderr,
            }),
            Err(error) => serde_json::json!({"error": format!("{error:#}")}),
        };
        rows.push(serde_json::json!({
            "importId": imported["importId"],
            "albumId": album,
            "expectedId": id,
            "expectedPath": path,
            "command": command,
            "receipt": receipt,
        }));
    }
    std::fs::write(
        out.join("media-store-after-import.json"),
        serde_json::to_vec_pretty(&rows)?,
    )?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() == 4 && matches!(args[3].as_str(), "picker-only" | "legacy-cells" | "selection-controls" | "production" | "sound-scout" | "sound-measure" | "rehearse"), "usage: carousel_selection_scout <serial> <single-bundle-folder> <new-report-folder> <picker-only|legacy-cells|selection-controls|production|sound-scout|sound-measure|rehearse>");
    let serial = &args[0];
    let out = PathBuf::from(&args[2]);
    anyhow::ensure!(!out.exists(), "use a new report folder");
    std::fs::create_dir_all(&out)?;
    let manifest = riviu_core::scan_publish_folder(&args[1], Default::default())?;
    anyhow::ensure!(manifest.bundles.len() == 1, "one bundle required");
    let bundle = &manifest.bundles[0];
    anyhow::ensure!(
        bundle.video.is_none() && !bundle.images.is_empty(),
        "photo bundle required"
    );
    let staged = out.join("staged");
    riviu_core::copy_bundle_to_managed(bundle, &staged)?;
    if let Some(mode) = std::env::var_os("RIVIU_SCOUT_PRESERVE_SOURCE_MTIME") {
        anyhow::ensure!(mode == "1", "RIVIU_SCOUT_PRESERVE_SOURCE_MTIME must be 1 when set");
        let mut times = Vec::with_capacity(bundle.images.len());
        for image in &bundle.images {
            let source_modified = std::fs::metadata(&image.path)?.modified()?;
            let target = staged.join(&image.file_name);
            OpenOptions::new()
                .write(true)
                .open(&target)?
                .set_times(FileTimes::new().set_modified(source_modified))?;
            let staged_modified = std::fs::metadata(&target)?.modified()?;
            anyhow::ensure!(
                staged_modified == source_modified,
                "staged mtime differs from source for {}",
                image.file_name
            );
            times.push(serde_json::json!({
                "fileName": image.file_name,
                "sourceModifiedUnixNs": source_modified.duration_since(UNIX_EPOCH)?.as_nanos().to_string(),
                "stagedModifiedUnixNs": staged_modified.duration_since(UNIX_EPOCH)?.as_nanos().to_string(),
            }));
        }
        std::fs::write(out.join("source-mtime.json"), serde_json::to_vec_pretty(&times)?)?;
    }
    let helper_state_dir = PathBuf::from(
        std::env::var_os("RIVIU_SCOUT_HELPER_STATE_DIR")
            .context("RIVIU_SCOUT_HELPER_STATE_DIR is required")?,
    );
    anyhow::ensure!(
        helper_state_dir.is_absolute() && helper_state_dir.is_dir(),
        "RIVIU_SCOUT_HELPER_STATE_DIR must be an existing absolute directory"
    );
    let mut config = common::repo_config();
    config.helper_state_dir = Some(helper_state_dir);
    let adb = AdbProgram::resolve(config.adb_path.as_deref(), config.bundled_adb_path.as_deref())?;
    let driver = Arc::new(AndroidDriver::new(&config)?);
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
        anyhow::ensure!(imported["files"].as_u64()==Some(bundle.images.len() as u64),"import count mismatch");
        std::fs::write(out.join("import.json"),serde_json::to_vec_pretty(&imported)?)?;
        capture_media_store(&adb, serial, &out, &imported).await?;
        if std::env::var("RIVIU_SCOUT_REPROVE_ORDER").as_deref() == Ok("1") {
            let own_import = imported["importId"].as_str().context("import ID")?;
            let refs = imported["mediaRefs"].as_array().context("media refs")?;
            let mut reset = Vec::new();
            for (index, image) in bundle.images.iter().enumerate() {
                let name = riviu_android_driver::publish::sanitise_id(&image.file_name);
                let row = refs.iter().find(|row| row["path"].as_str().is_some_and(|path| path.ends_with(&format!("/{name}")))).context("exact scoped image")?;
                let id = row["id"].as_str().context("row ID")?;
                anyhow::ensure!(id.parse::<u64>().is_ok(), "numeric ID");
                let seconds = 1791052152u64 + u64::from(index >= 8);
                let command = format!("content update --uri content://media/external/images/media/{id} --bind datetaken:l:{} --bind date_modified:l:{seconds} 2>&1", seconds * 1000);
                let output = adb.shell_output(serial, &command, Duration::from_secs(15)).await?;
                anyhow::ensure!(output.exit_code == 0, "scoped metadata setup failed");
                reset.push(serde_json::json!({"command":command,"stdout":output.stdout,"stderr":output.stderr,"exitCode":output.exit_code}));
            }
            std::fs::write(out.join("resume-metadata-setup.json"),serde_json::to_vec_pretty(&reset)?)?;
            capture_media_store(&adb, serial, &out, &imported).await?;
            std::fs::rename(out.join("media-store-after-import.json"),out.join("media-store-before-reproof.json"))?;
            let mut wrong = bundle.images.clone();
            wrong[0].sha256 = "0".repeat(64);
            let rejected = control.ensure_publish_photo_order(&lease, own_import, &wrong).await;
            anyhow::ensure!(rejected.is_err(), "wrong frozen hash accepted");
            std::fs::write(out.join("wrong-hash-rejection.json"),serde_json::to_vec_pretty(&serde_json::json!({"error":rejected.err().map(|e|e.to_string()),"publicPost":false}))?)?;
            capture_media_store(&adb, serial, &out, &imported).await?;
            let before:serde_json::Value=serde_json::from_slice(&std::fs::read(out.join("media-store-before-reproof.json"))?)?;
            let after:serde_json::Value=serde_json::from_slice(&std::fs::read(out.join("media-store-after-import.json"))?)?;
            anyhow::ensure!(before==after,"wrong hash changed metadata");
            std::fs::rename(out.join("media-store-after-import.json"),out.join("media-store-after-rejection.json"))?;
            let started=std::time::Instant::now();
            let repaired=control.ensure_publish_photo_order(&lease, own_import, &bundle.images).await?;
            let repaired_ms=started.elapsed().as_millis();
            anyhow::ensure!(repaired["photoOrder"]["repaired"]==true,"drift not repaired");
            let started=std::time::Instant::now();
            let unchanged=control.ensure_publish_photo_order(&lease, own_import, &bundle.images).await?;
            let unchanged_ms=started.elapsed().as_millis();
            anyhow::ensure!(unchanged["photoOrder"]["repaired"]==false,"correct order rewritten");
            std::fs::write(out.join("resume-reproof.json"),serde_json::to_vec_pretty(&serde_json::json!({"repaired":repaired,"unchanged":unchanged,"repairMs":repaired_ms,"unchangedMs":unchanged_ms,"publicPost":false}))?)?;
            capture_media_store(&adb, serial, &out, &imported).await?;
        }
        driver.terminate_app(serial,&package).await?;
        driver.launch_app(serial,&package).await?;
        let session=driver.open_session(serial).await?;
        let version=session.app_version(&package).await.context("version")?;
        let locale=session.ui_language().await.context("locale")?;
        let labels=controls_for(&package,&locale,&version).context("unmeasured tuple")?;
        let (w,h)=riviu_core::screen::measured_screen_size(&session).await?;
        let screen=Screen::new(w,h).context("screen")?;
        let request=CarouselRequest {album:import_id.as_deref().context("import ID")?, images:bundle.images.len(),caption:&bundle.caption,screen};
        let mut composer=Composer::new(&session,ComposerPlan::resolve(&labels)?,|node:&riviu_core::ElementBox|node.centre());
        let stop=AtomicBool::new(false);
        let observed: anyhow::Result<()> = async {
            if args[3]=="rehearse" {
                let sound_plan=riviu_core::tiktok_sound::SoundPickerPlan::resolve(&package,&locale,&version).context("sound plan")?;
                let policy=riviu_core::PublishSoundPolicy::TrendingAny {pool_size:5,seed:2985670833};
                let mut reached_post=false;
                let outcome=riviu_core::tiktok_composer::publish_carousel_with_sound_effect_intent(
                    &session,ComposerPlan::resolve(&labels)?,sound_plan,&policy,
                    |node:&riviu_core::ElementBox|node.centre(),&request,&stop,
                    |selection| {
                        reached_post=true;
                        std::fs::write(out.join("sound.json"),serde_json::to_vec_pretty(selection)?)?;
                        anyhow::bail!("rehearsal stops before public Post")
                    },
                ).await;
                std::fs::write(out.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({"reachedPostBoundary":reached_post,"publicPost":false,"error":outcome.as_ref().err().map(|e|format!("{e:#}"))}))?)?;
                anyhow::ensure!(reached_post && outcome.is_err(),"rehearsal did not reach the blocked Post boundary: {outcome:?}");
            } else if matches!(args[3].as_str(),"production"|"sound-scout"|"sound-measure") {
                let verdict=reach_edit_step(&mut composer,&request,&stop).await?;
                if verdict != ComposerVerdict::Stopped {
                    capture(&session,&out,"before-cleanup-failure").await?;
                    std::fs::write(out.join("before-cleanup-state.json"),serde_json::to_vec_pretty(&serde_json::json!({
                        "verdict":format!("{verdict:?}"),
                        "foreground":session.active_app_bundle().await.ok(),
                        "publicPost":false
                    }))?)?;
                }
                capture(&session,&out,"production-editor").await?;
                std::fs::write(out.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({"verdict":format!("{verdict:?}"),"requestedImages":request.images,"publicPost":false}))?)?;
                anyhow::ensure!(verdict==ComposerVerdict::Stopped,"selection refused: {}",verdict.reason());
                if args[3]=="sound-measure" {
                    use riviu_core::ElementQuery;
                    anyhow::ensure!(package=="com.zhiliaoapp.musically"&&version=="46.2.42"&&locale.starts_with("en"),"measurement requires exact m13 tuple");
                    let entry=session.locate_all(ElementQuery::ResourceIdSuffix(":id/dv3")).await?;
                    anyhow::ensure!(entry.len()==1,"entry ambiguous");session.tap(entry[0].centre()).await?;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    capture(&session,&out,"measure-open").await?;
                    let hot=session.locate_all(ElementQuery::Text{value:"Hot",exact:true}).await?;
                    anyhow::ensure!(hot.len()==1,"Hot tab ambiguous");session.tap(hot[0].centre()).await?;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    capture(&session,&out,"measure-hot").await?;
                    let titles=session.locate_all_described(ElementQuery::ResourceIdSuffix(":id/title")).await?;
                    let artists=session.locate_all_described(ElementQuery::ResourceIdSuffix(":id/zdw")).await?;
                    anyhow::ensure!(titles.len()>=2&&artists.len()>=2,"no second complete sound candidate");
                    let expected=titles[1].description.clone().context("title")?;
                    session.tap(titles[1].centre()).await?;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    capture(&session,&out,"measure-selected").await?;
                    session.back().await?;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    capture(&session,&out,"measure-editor").await?;
                    std::fs::write(out.join("measure.json"),serde_json::to_vec_pretty(&serde_json::json!({"expected":expected,"publicPost":false}))?)?;
                    let mut boundary=false;
                    let measured=riviu_core::tiktok_composer::continue_from_edit_step_with_effect_intent(&mut composer,&bundle.caption,&stop,&mut || {boundary=true;anyhow::bail!("measurement stops before Post")}).await;
                    capture(&session,&out,"measure-caption").await?;
                    std::fs::write(out.join("caption-measure.json"),serde_json::to_vec_pretty(&serde_json::json!({"boundary":boundary,"error":measured.as_ref().err().map(|e|format!("{e:#}")),"publicPost":false}))?)?;
                    anyhow::ensure!(boundary&&measured.is_err(),"caption boundary not reached");
                }
                if args[3]=="sound-scout" {
                    let sound_plan=riviu_core::tiktok_sound::SoundPickerPlan::resolve(&package,&locale,&version).context("sound plan")?;
                    let pool=riviu_core::tiktok_sound::open_and_observe_sounds(&session,sound_plan,5).await;
                    capture(&session,&out,"sound-picker").await?;
                    let pool=pool?;
                    let policy=riviu_core::PublishSoundPolicy::TrendingAny {pool_size:5,seed:2985670833};
                    let mut selection=riviu_core::publish::select_sound_candidate(&policy,&pool.candidates)?;
                    std::fs::write(out.join("sound-pool.json"),serde_json::to_vec_pretty(&serde_json::json!({"candidates":pool.candidates,"selection":selection}))?)?;
                    let selected=riviu_core::tiktok_sound::choose_and_confirm_sound(&session,sound_plan,&pool,selection.index).await;
                    selection.confirmed=selected.is_ok();
                    capture(&session,&out,"sound-selected").await?;
                    std::fs::write(out.join("sound-result.json"),serde_json::to_vec_pretty(&serde_json::json!({"selection":selection,"error":selected.as_ref().err().map(|e|format!("{e:#}")),"publicPost":false}))?)?;
                    selected?;
                }
            } else {
                anyhow::ensure!(reach_picker(&mut composer,&request,&stop).await?==ComposerVerdict::Stopped,"picker refused");
                capture(&session,&out,"picker-before").await?;
                if session.locate(labels.label(TikTokControl::PickerNext).context("next")?.to_query()).await?.is_none() {
                    anyhow::ensure!(composer.tap_multi_select_once(&stop).await?,"multi-select absent");
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                capture(&session,&out,"picker-multi").await?;
                if args[3]=="picker-only" { return Ok(()); }
                let tabs=session.locate(labels.label(TikTokControl::PickerTabPhotos).context("tabs")?.to_query()).await?.context("tabs absent")?;
                let grid=PhotoGrid::below_tabs(screen,tabs.y+tabs.height).context("grid")?;
                for index in 0..request.images {
                    let cell=grid.cell(index).context("cell outside grid")?;
                    if args[3]=="selection-controls" {
                        anyhow::ensure!(package=="com.ss.android.ugc.trill" && version=="38.3.2","unmeasured selection control");
                        let controls=session.locate_all(riviu_core::ElementQuery::ResourceIdSuffix(":id/h4b")).await?;
                        let matched:Vec<_>=controls.iter().filter(|c|c.x>=cell.x && c.y>=cell.y && c.x+c.width<=cell.x+cell.width && c.y+c.height<=cell.y+cell.height).collect();
                        anyhow::ensure!(matched.len()==1,"ambiguous cell selector");
                        session.tap(matched[0].centre()).await?;
                    } else {
                        session.tap(cell.centre()).await?;
                    }
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    capture(&session,&out,&format!("cell-{}",index+1)).await?;
                }
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
