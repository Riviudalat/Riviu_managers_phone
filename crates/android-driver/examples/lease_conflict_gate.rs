//! Prove that a phone held by one piece of work cannot be taken by another, on real hardware.
//!
//! ```text
//! cargo run -p riviu-android-driver --example lease_conflict_gate -- <serial>
//! ```
//!
//! **What was wrong, and why the fix needs a hardware run at all.** Twenty-four commands in
//! `commands/android_ops.rs` reached the Android driver with **no lease** — `factory_reset`,
//! `root_shell`, `power_off_device`, `set_input_method`, `open_system_settings` among them.
//! The consequence was not that those actions failed. It was that they *succeeded* while a
//! nurture session or an interaction campaign still believed it owned the phone: the session
//! kept its stream reservation, kept tapping coordinates, and **reported the work done**.
//! `open_system_settings` is the clearest case — it changes the foreground app, so a session
//! driving TikTok goes on tapping into Settings and calls it a success.
//!
//! The 47 lease tests in `device_control/mod.rs` all run against a mock driver, and they prove
//! the state machine. What they cannot prove is that the guard engages on a **real serial**
//! whose route table was built from a real `list_devices()` — which is the exact shape of bug
//! that let Gate G2 pass for weeks while the app could not open an Android session at all
//! (see `examples/control_plane.rs`).
//!
//! Four claims:
//!
//! 1. A free phone can be held.
//! 2. A second holder is **refused**, and the refusal **names the work that has it** — the
//!    part that makes the refusal actionable rather than just a wall. A refusal that says
//!    "busy" and stops there leaves the operator with nothing to stop.
//! 3. The same owner asking twice is refused too, so "it is only other people" is not the
//!    rule being relied on.
//! 4. After release the phone is takeable again, and no cleanup ticket was quarantined.
//!
//! **Refusal, not preemption, and that is the shipped decision.** `DeviceControlPlane` has no
//! revocation: nothing can take a lease from its holder in a way the holder learns about. So
//! an override would mean either inventing a preemption protocol — a worse risk to a running
//! campaign than the bug being fixed — or dropping the lease silently, which *is* the bug.
//! This gate therefore asserts a refusal. If preemption is ever designed, this file is where
//! the new contract gets its hardware proof, and claim 2 becomes "the holder observes the
//! interruption" instead.
//!
//! Read-only on the phone: it takes and releases leases and starts no stream. Nothing on the
//! device changes, which is why it is safe to run on a fleet phone mid-shift.

use std::sync::Arc;

use riviu_android_driver::AndroidDriver;
use riviu_core::{DeviceControlPlane, DeviceWorkCoordinator, DeviceWorkOwner, StreamBudgetManager};

#[path = "common/mod.rs"]
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let serial = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: lease_conflict_gate <serial>"))?;

    let config = common::repo_config();
    println!("{}", common::describe_adb(&config));
    let driver = Arc::new(AndroidDriver::new(&config)?);
    let control = DeviceControlPlane::new(
        Arc::clone(&driver) as Arc<dyn riviu_core::DeviceDriver>,
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(StreamBudgetManager::new(1)?),
    );

    // The route table is built only from a listing, never by parsing a udid — so this call is
    // load-bearing, not a courtesy check. Without it the plane has no route for the serial and
    // every claim below would be about a device it has never heard of.
    let devices = control.list_devices().await?;
    anyhow::ensure!(
        devices.iter().any(|device| device.udid == serial),
        "adb does not see {serial} (thấy {} máy: {:?}) — kiểm cáp, hoặc đặt RIVIU_ADB_PATH",
        devices.len(),
        devices
            .iter()
            .map(|device| device.udid.as_str())
            .collect::<Vec<_>>()
    );
    println!("máy      {serial}\n");

    // Claim 1. `Nurture` stands in for the session that used to keep reporting success while
    // somebody else drove the phone out from under it.
    let held = control
        .try_acquire_exclusive_keeping_stream(&serial, DeviceWorkOwner::Nurture)
        .await
        .map_err(|reason| anyhow::anyhow!("máy rỗi mà không giữ được: {reason}"))?;
    println!("1. giữ được máy với owner Nurture");

    // Claims 2 and 3 must not leave the lease held if they fail, or the phone is stuck until
    // the process exits and the operator is left worse off than before the gate ran.
    let verdicts = check_while_held(&control, &serial).await;

    // `DeviceExclusiveContext` has no release call: its lease, activity permit and capacity
    // token are drop guards, so dropping it *is* the release. Checked against
    // `impl Drop for DeviceWorkLease` rather than assumed -- a context "released" by being
    // forgotten would make claim 4 vacuous.
    drop(held);
    println!("4a. nhả lease (drop)");

    let mut failures: Vec<String> = verdicts
        .into_iter()
        .filter_map(|verdict| verdict.err())
        .map(|reason| format!("{reason:#}"))
        .collect();

    if let Err(reason) = retakeable_after_release(&control, &serial).await {
        failures.push(format!("{reason:#}"));
    }

    // A wrong generation or a stop that never happened shows up only here. `control_plane.rs`
    // says the same thing and it is worth repeating: nothing else in a passing run reveals it.
    match control.cleanup_quarantine_count() {
        0 => println!("4c. 0 cleanup ticket bị quarantine"),
        count => failures.push(format!(
            "{count} cleanup ticket bị quarantine — một lease đã nhả sai cách"
        )),
    }

    println!();
    if failures.is_empty() {
        println!("ĐẠT — cả bốn khẳng định đúng trên {serial}");
        return Ok(());
    }
    for line in &failures {
        println!("KHÔNG ĐẠT  {line}");
    }
    anyhow::bail!("{} phép kiểm thất bại", failures.len())
}

/// Claims 2 and 3, run with the phone held. Collected rather than `?`-ed so the caller always
/// reaches the release.
async fn check_while_held(control: &DeviceControlPlane, serial: &str) -> Vec<anyhow::Result<()>> {
    vec![
        refused_and_names_the_holder(control, serial, DeviceWorkOwner::ManualControl, "2").await,
        refused_and_names_the_holder(control, serial, DeviceWorkOwner::Nurture, "3").await,
    ]
}

/// A refusal is only useful if it says what to stop.
async fn refused_and_names_the_holder(
    control: &DeviceControlPlane,
    serial: &str,
    requested: DeviceWorkOwner,
    claim: &str,
) -> anyhow::Result<()> {
    // `ManualControl` is what every command in `android_ops.rs` asks for, via
    // `AppState::device_lease`. Asking for it here is asking the same question `factory_reset`
    // asks — without doing anything to the phone.
    match control
        .try_acquire_exclusive_keeping_stream(serial, requested)
        .await
    {
        Ok(stolen) => {
            // Do not leave two contexts believing they own one phone.
            drop(stolen);
            anyhow::bail!(
                "{claim}. {requested:?} LẤY ĐƯỢC máy đang bị Nurture giữ — đây đúng là lỗi cũ: \
                 phiên đang giữ vẫn sẽ báo thành công"
            )
        }
        Err(error) => {
            let message = error.to_string();
            // The holder has to be named, not merely implied by "busy".
            anyhow::ensure!(
                message.contains("Nurture"),
                "{claim}. bị từ chối nhưng KHÔNG nêu tên việc đang giữ máy: {message:?} — một \
                 lời từ chối không nói phải dừng cái gì thì không giúp được ai"
            );
            println!("{claim}. {requested:?} bị từ chối, và nêu tên người giữ: {message}");
            Ok(())
        }
    }
}

/// Claim 4: the refusal was a lock, not a leak.
async fn retakeable_after_release(
    control: &DeviceControlPlane,
    serial: &str,
) -> anyhow::Result<()> {
    let again = control
        .try_acquire_exclusive_keeping_stream(serial, DeviceWorkOwner::ManualControl)
        .await
        .map_err(|reason| {
            anyhow::anyhow!("nhả rồi mà ManualControl vẫn không giữ được: {reason}")
        })?;
    println!("4b. sau khi nhả, ManualControl giữ được");
    drop(again);
    Ok(())
}
