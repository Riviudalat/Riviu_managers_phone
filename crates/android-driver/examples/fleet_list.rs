//! Print the fleet exactly as `DeviceDriver::list_devices` reports it.
//!
//! ```text
//! cargo run -p riviu-android-driver --example fleet_list
//! ```
//!
//! Read-only, and it exists for one question a unit test cannot answer: **does a phone adb
//! can see but not drive still get a row?** `AdbDeviceState::Offline` used to be discarded,
//! so a device whose cable or hub dropped simply vanished from the grid — no row, no reason,
//! indistinguishable from unplugged. Watching this while a phone reboots is how that is
//! checked on real hardware rather than on a fixture.

use riviu_core::driver::DeviceDriver;

#[path = "common/mod.rs"]
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let driver = riviu_android_driver::AndroidDriver::new(
        &common::repo_config(),
    )?;
    // Printed before the count, and that order is the point: `0 device(s)` on its own has been
    // read as "no phone", as "no TikTok", and as everything except "no adb" — see
    // `common::describe_adb`.
    println!("{}", common::describe_adb(&common::repo_config()));
    let devices = driver.list_devices().await?;
    println!("{} device(s)", devices.len());
    for device in devices {
        println!(
            "  {:<20} {:<13} {:<12} {}",
            device.udid,
            format!("{:?}", device.status),
            device.model,
            device.last_error.unwrap_or_default()
        );
    }
    Ok(())
}
