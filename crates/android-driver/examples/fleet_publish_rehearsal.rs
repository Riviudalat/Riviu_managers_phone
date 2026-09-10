//! Fleet acceptance through production composer, stopping at its effect-intent callback.
//! One driver/port allocator, bounded parallel devices, isolated imports and durable reports.
use anyhow::Context;
use riviu_android_driver::AndroidDriver;
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::tiktok_composer::{
    self, CarouselRequest, ComposerPlan, PublishProgress, Screen, VideoPickerPlan, VideoRequest,
};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
#[path = "common/mod.rs"]
mod common;

fn save(out: &Path, name: &str, value: &impl serde::Serialize) -> anyhow::Result<()> {
    std::fs::write(out.join(name), serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
async fn capture(session: &dyn UiSession, out: &Path, name: &str) {
    if let Ok(source) = session.hierarchy_source_snapshot().await {
        let _ = std::fs::write(out.join(format!("{name}.xml")), source.xml);
    }
    if let Ok(png) = session.screenshot_png().await {
        let _ = std::fs::write(out.join(format!("{name}.png")), png);
    }
}
async fn one(
    driver: Arc<AndroidDriver>,
    control: Arc<riviu_core::DeviceControlPlane>,
    serial: String,
    bundle: riviu_core::PublishBundle,
    out: PathBuf,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(&out)?;
    let started = std::time::Instant::now();
    let context = control
        .acquire_exclusive(&serial, riviu_core::DeviceWorkOwner::Script)
        .await?;
    let id = format!("fleet-rehearsal-{}", uuid::Uuid::new_v4());
    let mut imported = None;
    let mut package = None;
    let result:anyhow::Result<()> = async {
        driver.verify_automation_transport(&serial).await?;
        let (pkg,version,locale)=driver.tiktok_build(&serial).await?;package=Some(pkg.clone());
        save(&out,"build.json",&serde_json::json!({"serial":serial,"package":pkg,"version":version,"locale":locale}))?;
        let labels=riviu_core::tiktok_labels::controls_for(&pkg,&locale,&version).context("unmeasured TikTok language/build")?;
        let sound=riviu_core::tiktok_sound::SoundPickerPlan::resolve(&pkg,&locale,&version).context("sound plan")?;
        let staged=out.join("staged");riviu_core::copy_bundle_to_managed(&bundle,&staged)?;
        let proof=driver.stage_publish_media(&serial,"",&id,&staged).await?;
        let hash=proof["manifestSha256"].as_str().context("manifest hash")?;
        let prep=driver.prepare_publish_media(&serial,&id,hash).await?;
        let import=prep["importId"].as_str().context("import ID")?.to_string();imported=Some(import.clone());
        let transfer=driver.import_publish_media(&serial,&id,hash).await?;
        save(&out,"import.json",&transfer)?;
        anyhow::ensure!(transfer["files"].as_u64()==Some(if bundle.video.is_some(){1}else{bundle.images.len() as u64}),"import count mismatch");
        driver.terminate_app(&serial,&pkg).await?;driver.launch_app(&serial,&pkg).await?;
        let session=driver.open_session(&serial).await?;
        let observed:anyhow::Result<()> = async {
            // Profile is an observed English accessibility description on this fleet.
            let profile=riviu_core::ElementQuery::Description{value:"Profile",exact:true};
            let deadline=std::time::Instant::now()+Duration::from_secs(40);
            let tab=loop {
                let tabs=session.locate_all(profile).await?;
                if tabs.len()==1 && tabs[0].enabled {break tabs[0].clone();}
                anyhow::ensure!(std::time::Instant::now()<deadline,"profile tab not ready; inspect screenshot/login");
                tokio::time::sleep(Duration::from_millis(400)).await;
            };
            session.tap(tab.centre()).await?;tokio::time::sleep(Duration::from_secs(2)).await;
            let notices=session.locate_all(riviu_core::ElementQuery::Text{value:"TikTok is more fun with friends. By syncing your phone contacts, you can find and get discovered by people you know.",exact:true}).await?;
            if notices.len()==1 {
                let deny=session.locate_all(riviu_core::ElementQuery::Text{value:"Don’t allow",exact:true}).await?;
                if let [button]=deny.as_slice(){session.tap(button.centre()).await?;tokio::time::sleep(Duration::from_millis(500)).await;}
            }
            let account=riviu_core::tiktok_account::observe_own_account(&session,labels).await?;
            capture(&session,&out,"account").await;
            save(&out,"account.json",&serde_json::json!({"handle":account}))?;
            anyhow::ensure!(account.is_some(),"TikTok account not signed in or own-profile readback missing");
            session.back().await?;
            let create=labels.label(riviu_core::tiktok_labels::TikTokControl::ComposerOpen).context("Create label")?;
            let deadline=std::time::Instant::now()+Duration::from_secs(40);
            loop {
                if session.locate(create.to_query()).await?.is_some(){break;}
                anyhow::ensure!(std::time::Instant::now()<deadline,"feed not ready after account readback");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let (w,h)=riviu_core::screen::measured_screen_size(&session).await?;
            let screen=Screen::new(w,h).context("screen")?;
            let stop=AtomicBool::new(false);let mut boundary=false;
            let progress=|p:PublishProgress|{
                use std::io::Write;
                let row=serde_json::json!({"at":chrono::Local::now().to_rfc3339(),"state":p.state(),"message":p.message(&serial)});
                if let Ok(mut f)=std::fs::OpenOptions::new().create(true).append(true).open(out.join("steps.jsonl")){let _=writeln!(f,"{row}");}
            };
            let intent=|selection:&riviu_core::SoundSelectionEvidence|{
                boundary=true;save(&out,"sound.json",selection)?;anyhow::bail!("rehearsal verified before public Post")
            };
            let policy=riviu_core::PublishSoundPolicy::TrendingAny{pool_size:5,seed:2985670833};
            let plan=ComposerPlan::resolve(&labels)?;
            let outcome=if bundle.video.is_some(){
                let req=VideoRequest{album:&import,caption:&bundle.caption,screen};
                tiktok_composer::publish_video_with_sound_effect_intent_and_progress(&session,plan,VideoPickerPlan::resolve(&pkg,&locale,&version).context("video plan")?,sound,&policy,|n:&riviu_core::ElementBox|n.centre(),&req,&stop,intent,&progress).await
            }else{
                let req=CarouselRequest{album:&import,images:bundle.images.len(),caption:&bundle.caption,screen};
                tiktok_composer::publish_carousel_with_sound_effect_intent_and_progress(&session,plan,sound,&policy,|n:&riviu_core::ElementBox|n.centre(),&req,&stop,intent,&progress).await
            };
            capture(&session,&out,"before-post").await;
            anyhow::ensure!(boundary && outcome.is_err(),"composer did not reach verified Post boundary: {outcome:?}");
            Ok(())
        }.await;
        if observed.is_err(){capture(&session,&out,"failure").await;}
        observed
    }.await;
    let terminated = if let Some(pkg) = package {
        driver.terminate_app(&serial, &pkg).await.map(|_| ())
    } else {
        Ok(())
    };
    let cleanup = if let Some(import) = imported {
        driver
            .cleanup_publish_media(&serial, &import)
            .await
            .map(Some)
    } else {
        Ok(None)
    };
    control.close_exclusive_context(context)?;
    save(
        &out,
        "result.json",
        &serde_json::json!({"serial":serial,"media":bundle.media_kind,"reachedPostBoundary":result.is_ok(),"publicPost":false,"error":result.as_ref().err().map(|e|format!("{e:#}")),"cleanup":cleanup.as_ref().ok(),"cleanupError":cleanup.as_ref().err().map(ToString::to_string),"terminated":terminated.is_ok(),"seconds":started.elapsed().as_secs_f64()}),
    )?;
    println!(
        "{} {} boundary={} cleanup={}",
        serial,
        bundle.name,
        result.is_ok(),
        cleanup.is_ok()
    );
    result?;
    terminated?;
    cleanup?;
    Ok(())
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 2 && args[0] == "cleanup" {
        let root = PathBuf::from(&args[1]);
        let driver = AndroidDriver::new(&common::repo_config())?;
        for phone in std::fs::read_dir(&root)? {
            let phone = phone?;
            if !phone.path().is_dir() {
                continue;
            }
            let serial = phone.file_name().to_string_lossy().to_string();
            for media in ["photo", "video"] {
                let folder = phone.path().join(media);
                if folder.join("result.json").exists() || !folder.join("import.json").exists() {
                    continue;
                }
                let import: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(folder.join("import.json"))?)?;
                let pkg = driver.resolve_tiktok_package(&serial).await?;
                driver.terminate_app(&serial, &pkg).await?;
                let result = driver
                    .cleanup_publish_media(&serial, import["importId"].as_str().context("import")?)
                    .await?;
                save(&folder, "interrupted-cleanup.json", &result)?;
                println!("cleaned {serial}/{media}");
            }
        }
        return Ok(());
    }
    anyhow::ensure!(
        args.len() == 4,
        "fleet_publish_rehearsal SERIALS.json PHOTO_BUNDLE VIDEO_BUNDLE NEW_REPORT"
    );
    let serials: Vec<String> = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    let out = PathBuf::from(&args[3]);
    anyhow::ensure!(!out.exists(), "new report path required");
    std::fs::create_dir_all(&out)?;
    let photo = riviu_core::scan_publish_folder(&args[1], Default::default())?
        .bundles
        .remove(0);
    let video = riviu_core::scan_publish_folder(&args[2], Default::default())?
        .bundles
        .remove(0);
    let driver = Arc::new(AndroidDriver::new(&common::repo_config())?);
    let control = Arc::new(riviu_core::DeviceControlPlane::new(
        driver.clone(),
        Arc::new(riviu_core::DeviceWorkCoordinator::new()),
        Arc::new(riviu_core::StreamBudgetManager::new(3)?),
    ));
    let permits = Arc::new(tokio::sync::Semaphore::new(3));
    let mut tasks = tokio::task::JoinSet::new();
    for serial in serials {
        let (driver, control, permits, photo, video, out) = (
            driver.clone(),
            control.clone(),
            permits.clone(),
            photo.clone(),
            video.clone(),
            out.clone(),
        );
        tasks.spawn(async move {
            let _permit = permits.acquire_owned().await?;
            let photos = one(
                driver.clone(),
                control.clone(),
                serial.clone(),
                photo,
                out.join(&serial).join("photo"),
            )
            .await;
            let videos = one(
                driver,
                control,
                serial.clone(),
                video,
                out.join(&serial).join("video"),
            )
            .await;
            Ok::<_, anyhow::Error>(
                serde_json::json!({"serial":serial,"photo":photos.is_ok(),"video":videos.is_ok()}),
            )
        });
    }
    let mut rows = Vec::new();
    while let Some(r) = tasks.join_next().await {
        rows.push(r??);
        save(&out, "summary.json", &rows)?;
    }
    control.shutdown_cleanup().await?;
    anyhow::ensure!(
        rows.iter()
            .all(|r| r["photo"] == true && r["video"] == true),
        "some phones need repair; inspect summary and per-device evidence"
    );
    Ok(())
}
