//! Ask a real phone the question Flow's preflight asks, and print the answer.
//!
//! Read-only: one `dumpsys` per package, one `sha256sum`, one `dumpsys display`, one
//! `getprop`, and whatever `ensure_agent` has to do to prove the agent can see. Nothing is
//! tapped, launched or installed beyond the agent this app already installs.
//!
//! ```text
//! cargo run -p riviu-android-driver --example flow_qualify -- <serial> [<target-package>]
//! ```
//!
//! It exists because the thing B1 had to fix cannot be proved by a unit test: the failure
//! was that `AndroidDriver` did not implement `inspect_device_for_target` at all, so Flow
//! refused at preflight on every Android phone. A test with a fixed set of facts proves the
//! snapshot is *assembled* right; only a phone proves the facts can be *read*.

use riviu_android_driver::{AndroidDriver, AndroidDriverConfig};
use riviu_core::driver::DeviceDriver;
use riviu_core::qualified_geometry_profile_id;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(serial) = args.first() else {
        println!("usage: flow_qualify <serial> [<target-package>]   (read-only)");
        return Ok(());
    };

    let driver = AndroidDriver::new(&AndroidDriverConfig::default())?;
    let target = match args.get(1) {
        Some(package) => package.clone(),
        None => driver.resolve_tiktok_package(serial).await?,
    };
    println!("serial  {serial}\ntarget  {target}");

    let started = std::time::Instant::now();
    let snapshot = driver.inspect_device_for_target(serial, &target).await?;
    let elapsed = started.elapsed();

    println!("\n--- snapshot ({} ms) ---", elapsed.as_millis());
    println!("{}", serde_json::to_string_pretty(&snapshot)?);

    // The gate `build_preflight` applies before it will run a node. Printed rather than
    // asserted so a phone that fails says which half it failed.
    println!("\n--- preflight gate ---");
    println!(
        "target matches request  {}",
        snapshot.target_app.bundle_id == target
    );
    println!("control surface live    {}", snapshot.protected_auth_ready);
    println!(
        "agent identity complete {}",
        !snapshot.installed_agent.bundle_id.trim().is_empty()
            && !snapshot.installed_agent.executable_name.trim().is_empty()
            && !snapshot.agent_version.trim().is_empty()
    );
    match qualified_geometry_profile_id(&snapshot) {
        Ok(id) => println!("geometry profile id     {id}"),
        Err(reason) => println!("geometry profile id     UNAVAILABLE ({reason})"),
    }

    let status = driver.cached_agent_status(serial);
    println!(
        "\nagent status            {:?} auth={} features={:?}",
        status.state, status.auth_ready, status.features
    );
    Ok(())
}
