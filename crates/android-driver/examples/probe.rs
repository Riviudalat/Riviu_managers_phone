//! Gate G1: drive the real `AndroidDriver` against a real phone.
//!
//! Compiling is not evidence. This exercises the actual trait implementations
//! — `DeviceDriver` and `UiSession`, no test doubles — and times each call, so
//! the numbers in `docs/ANDROID_PROBE_REPORT_2026-08-09.md` can be checked
//! against the shipped code rather than against a shell transcript.
//!
//! ```text
//! cargo run -p riviu-android-driver --example probe -- <serial>
//! ```
//!
//! Read-only by default. It launches TikTok, reads labels, and captures a
//! screenshot. Pass `--terminate` to also exercise the force-stop path, which
//! closes the app on the device.

use std::time::{Duration, Instant};

use riviu_android_driver::{AndroidDriver, AndroidDriverConfig, Locator};
use riviu_core::driver::{DeviceDriver, UiSession};

const TIKTOK: &str = "com.zhiliaoapp.musically";

macro_rules! timed {
    ($label:expr, $body:expr) => {{
        let started = Instant::now();
        let outcome = $body;
        let elapsed = started.elapsed().as_millis();
        match &outcome {
            Ok(_) => println!("  {:<34} {:>6} ms  ok", $label, elapsed),
            Err(error) => println!("  {:<34} {:>6} ms  FAILED: {error}", $label, elapsed),
        }
        outcome
    }};
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let terminate = args.iter().any(|arg| arg == "--terminate");
    let wanted = args.iter().find(|arg| !arg.starts_with("--")).cloned();

    let driver = AndroidDriver::new(&AndroidDriverConfig::default())?;

    println!("== list_devices ==");
    let devices = timed!("list_devices", driver.list_devices().await)?;
    for device in &devices {
        println!(
            "  {:<20} {:<12} os={:<5} status={:?} battery={:?} agent={}",
            device.udid,
            device.model,
            device.ios_version,
            device.status,
            device.battery,
            device.wda_ready
        );
        if let Some(error) = &device.last_error {
            println!("    ^ {error}");
        }
    }

    let serial = match wanted {
        Some(serial) => serial,
        None => devices
            .iter()
            .find(|device| device.wda_ready)
            .or_else(|| devices.first())
            .map(|device| device.udid.clone())
            .ok_or_else(|| anyhow::anyhow!("no Android device attached"))?,
    };
    println!("\n== device {serial} ==");

    timed!(
        "launch_app(tiktok)",
        driver.launch_app(&serial, TIKTOK).await
    )?;
    tokio::time::sleep(Duration::from_secs(8)).await;

    println!("\n== open_session (starts the agent if needed) ==");
    let session = timed!("open_session", driver.open_session(&serial).await)?;

    println!("\n== UiSession ==");
    let size = timed!("window_size", session.window_size().await)?;
    println!("    screen = {:?} (must be the wm Override size)", size);

    let bundle = timed!("active_app_bundle", session.active_app_bundle().await)?;
    println!("    foreground = {bundle}");
    if bundle != TIKTOK {
        println!("    ! not TikTok; the label probes below will not find anything");
    }

    let _ = timed!(
        "assert_visible(\"For You\")",
        session.assert_visible("For You").await
    );

    // The like control carries its own state in the label, which is the whole
    // reason the CV layer is not needed here.
    println!("\n== label state, the evidence that replaces pixel matching ==");
    for label in ["Like", "Video liked"] {
        let locator = Locator::Description(label.to_string());
        let started = Instant::now();
        match session.agent().find(&locator).await {
            Ok(Some(_)) => println!(
                "  {:<34} {:>6} ms  PRESENT",
                format!("find(\"{label}\")"),
                started.elapsed().as_millis()
            ),
            Ok(None) => println!(
                "  {:<34} {:>6} ms  absent",
                format!("find(\"{label}\")"),
                started.elapsed().as_millis()
            ),
            Err(error) => println!("  find(\"{label}\") FAILED: {error}"),
        }
    }

    // A LIVE post has no action rail at all, which a feed loop must handle.
    let live = session
        .agent()
        .find(&Locator::DescriptionContains("Tap to watch LIVE".into()))
        .await?;
    if live.is_some() {
        println!("  current post is a LIVE stream: no action rail, a feed loop must swipe past");
    }

    let comments = session
        .agent()
        .find(&Locator::DescriptionContains("comments".into()))
        .await?;
    match comments {
        Some(element) => {
            let desc = session.agent().attribute(&element, "content-desc").await?;
            println!("  comment button label = {desc:?}   <- the count is readable here");
            let rect = timed!("rect(comment button)", session.agent().rect(&element).await)?;
            println!("    bounds = {rect:?}");
        }
        None => println!("  no comment button on this post"),
    }

    println!("\n== screenshot ==");
    let png = timed!("screenshot_png", session.screenshot_png().await)?;
    let looks_like_png = png.starts_with(&[0x89, b'P', b'N', b'G']);
    println!(
        "    {} bytes, PNG magic {}",
        png.len(),
        if looks_like_png { "ok" } else { "MISSING" }
    );
    anyhow::ensure!(looks_like_png, "screenshot_png did not return a PNG");

    println!("\n== capabilities this backend advertises ==");
    println!(
        "    supports_text_input           = {}",
        session.supports_text_input()
    );
    println!(
        "    supports_accessibility_readback = {}",
        session.supports_accessibility_readback()
    );
    println!(
        "    supports_verified_app_termination = {}",
        driver.supports_verified_app_termination(&serial)
    );
    println!(
        "    supports_text_comments        = {}",
        driver.supports_text_comments(&serial)
    );

    println!("\n== inspect_app_process ==");
    let state = timed!(
        "inspect_app_process",
        driver.inspect_app_process(&serial, TIKTOK).await
    )?;
    println!("    {state:?}");

    if terminate {
        println!("\n== terminate_app (proved by pidof, not by the command's exit code) ==");
        let proof = timed!("terminate_app", driver.terminate_app(&serial, TIKTOK).await)?;
        println!("    proof = {proof:?}");
        let after = driver.inspect_app_process(&serial, TIKTOK).await?;
        anyhow::ensure!(!after.running, "TikTok still running after terminate_app");
        println!("    confirmed gone");
    } else {
        println!("\n(skipping terminate_app; pass --terminate to exercise it)");
    }

    println!("\nG1 probe finished.");
    Ok(())
}
