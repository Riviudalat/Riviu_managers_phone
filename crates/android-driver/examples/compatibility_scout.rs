//! Interactive, single-device calibration under an exclusive control-plane lease.
//! Commands are local JSON files; Post/Send are deliberately excluded from this scout.
use anyhow::Context;
use riviu_android_driver::{AdbProgram, AndroidDriver, HelperClient};
use riviu_core::{driver::DeviceDriver, ElementQuery, UiSession};
use std::{path::PathBuf, sync::Arc, time::Duration};
#[path = "common/mod.rs"]
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() == 3, "compatibility_scout SERIAL BUNDLE REPORT");
    let serial = &args[0];
    let out = PathBuf::from(&args[2]);
    anyhow::ensure!(!out.exists(), "new report directory required");
    std::fs::create_dir_all(&out)?;
    let config = common::repo_config();
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
    let id = format!("compat-scout-{}", uuid::Uuid::new_v4());
    let mut import = None;
    let result: anyhow::Result<()> = async {
        let adb=AdbProgram::resolve(config.adb_path.as_deref(),config.bundled_adb_path.as_deref())?;
        let helper=HelperClient::ensure(adb,serial,config.bundled_riviu_agent_apk.as_deref()).await?;
        anyhow::ensure!(helper.is_alive().await,"helper status");helper.shutdown().await?;
        let manifest=riviu_core::scan_publish_folder(&args[1],Default::default())?;
        anyhow::ensure!(manifest.bundles.len()==1,"one test bundle");
        let staged=out.join("staged");riviu_core::copy_bundle_to_managed(&manifest.bundles[0],&staged)?;
        let proof=driver.stage_publish_media(serial,"",&id,&staged).await?;
        let sha=proof["manifestSha256"].as_str().context("hash")?;
        let prepared=driver.prepare_publish_media(serial,&id,sha).await?;
        import=prepared["importId"].as_str().map(str::to_owned);
        let imported=driver.import_publish_media(serial,&id,sha).await?;
        std::fs::write(out.join("import.json"),serde_json::to_vec_pretty(&imported)?)?;
        driver.terminate_app(serial,&package).await?;driver.launch_app(serial,&package).await?;
        let session=driver.open_session(serial).await?;
        std::fs::write(out.join("ready.json"),serde_json::to_vec_pretty(&serde_json::json!({"serial":serial,"package":package,"version":session.app_version(&package).await,"locale":session.ui_language().await,"import":import}))?)?;
        for _ in 0..7200 {
            let command=out.join("command.json");
            if !command.exists(){tokio::time::sleep(Duration::from_millis(250)).await;continue;}
            let raw=std::fs::read(&command)?;std::fs::remove_file(command)?;
            let v:serde_json::Value=serde_json::from_slice(&raw)?;
            let name=v["name"].as_str().context("name")?;
            anyhow::ensure!(name.chars().all(|c|c.is_ascii_alphanumeric()||c=='-'),"name");
            let action=v["action"].as_str().unwrap_or("capture");
            if action=="stop"{break;}
            let action_result:anyhow::Result<()> = async {
                match action {
                    "capture"=>{},
                    "back"=>session.back().await?,
                    "text"=>session.type_text(v["text"].as_str().context("text")?).await?,
                    "tap"=>{
                        let value=v["value"].as_str().context("value")?;
                        anyhow::ensure!(!["Post","Send","Your Story","Đăng"].contains(&value),"public action excluded");
                        let query=match v["by"].as_str().context("by")? {
                            "id"=>ElementQuery::ResourceIdSuffix(value),
                            "text"=>ElementQuery::Text{value,exact:true},
                            "desc"=>ElementQuery::Description{value,exact:true},
                            _=>anyhow::bail!("query"),
                        };
                        let rows=session.locate_all(query).await?;
                        let index=v["index"].as_u64().map(|n|n as usize);
                        anyhow::ensure!(index.is_some()||rows.len()==1,"expected unique target; got {}",rows.len());
                        let row=rows.get(index.unwrap_or(0)).context("target index")?;
                        anyhow::ensure!(row.enabled,"disabled target");session.tap(row.centre()).await?;
                    },
                    _=>anyhow::bail!("action"),
                }
                if action!="capture" {tokio::time::sleep(Duration::from_millis(1500)).await;}
                std::fs::write(out.join(format!("{name}.xml")),session.hierarchy_source_snapshot().await?.xml)?;
                std::fs::write(out.join(format!("{name}.png")),session.screenshot_png().await?)?;
                Ok(())
            }.await;
            std::fs::write(out.join(format!("{name}.result.json")),serde_json::to_vec_pretty(&serde_json::json!({"ok":action_result.is_ok(),"error":action_result.err().map(|e|format!("{e:#}"))}))?)?;
        }
        Ok(())
    }.await;
    let stopped = driver.terminate_app(serial, &package).await;
    let cleaned = if let Some(id) = import {
        Some(driver.cleanup_publish_media(serial, &id).await)
    } else {
        None
    };
    control.close_exclusive_context(lease)?;
    let shutdown = control.shutdown_cleanup().await;
    let process = driver.inspect_app_process(serial, &package).await;
    std::fs::write(
        out.join("cleanup.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"error":result.as_ref().err().map(|e|format!("{e:#}")),"terminated":stopped.is_ok(),"media":cleaned.as_ref().and_then(|v|v.as_ref().ok()),"shutdownError":shutdown.as_ref().err().map(ToString::to_string),"running":process.as_ref().ok().map(|p|p.running)}),
        )?,
    )?;
    result?;
    stopped?;
    if let Some(c) = cleaned {
        c?;
    }
    shutdown?;
    anyhow::ensure!(!process?.running, "process remains");
    Ok(())
}
