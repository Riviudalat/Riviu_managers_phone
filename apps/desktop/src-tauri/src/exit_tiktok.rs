//! Close every measured TikTok build on every connected Android phone as the app exits.
//!
//! Measured 28/09/2026: a normal exit left TikTok running on 30/30 phones, because the exit
//! drain released leases and tore the driver down without ever stopping an app. This runs after
//! the drain and before `shutdown_cleanup`, through the same exclusive leases and
//! `terminate_app` every other close uses — no second controller and no adb-server kill.
//!
//! `terminate_app` proves absence with `pidof` on the main package only, so the result here is
//! read again from `ps -A -o PID,NAME`, where `package:child` processes also show. A listing
//! that cannot be read is `unknown`, never `closed`. iOS is untouched.
use std::sync::Arc;
use std::time::Duration;

use futures_util::{stream, StreamExt};
use riviu_core::{
    db::Database, DeviceControlError, DeviceControlPlane, DeviceExclusiveContext, DeviceWorkOwner,
};
use serde::Serialize;

/// Where the last exit's per-phone outcome is kept for the next launch and the operator.
pub(crate) const EXIT_TIKTOK_CLOSE_SETTING: &str = "app.exit.tiktokClose";
const CONCURRENCY: usize = 2;
const INVENTORY_DEADLINE: Duration = Duration::from_secs(10);
const LEASE_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum CloseStatus {
    Closed,
    Failed,
    Unknown,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceClose {
    udid: String,
    status: CloseStatus,
    packages: Vec<String>,
    /// Processes of those packages still listed after the stop, as `pid name`.
    remaining: Vec<String>,
    reason: Option<String>,
}

impl DeviceClose {
    fn new(udid: &str) -> Self {
        Self {
            udid: udid.to_owned(),
            status: CloseStatus::Unknown,
            packages: Vec::new(),
            remaining: Vec::new(),
            reason: None,
        }
    }

    fn with(mut self, status: CloseStatus, reason: impl Into<String>) -> Self {
        self.status = status;
        self.reason = Some(reason.into());
        self
    }
}

pub(crate) async fn close_tiktok_on_exit(control: Arc<DeviceControlPlane>, db: Arc<Database>) {
    let started = chrono::Utc::now().to_rfc3339();
    let mut disconnected = Vec::new();
    let (devices, listing_error) = match control.list_devices().await {
        Ok(devices) => {
            let mut connected = Vec::new();
            for device in devices {
                if device.platform != riviu_core::DevicePlatform::Android {
                    continue;
                }
                if device.status == riviu_core::DeviceStatus::Disconnected {
                    // A known phone we cannot reach is unknown, not silently absent.
                    disconnected.push(
                        DeviceClose::new(&device.udid)
                            .with(CloseStatus::Unknown, "máy mất kết nối khi thoát"),
                    );
                } else {
                    connected.push(device.udid);
                }
            }
            (connected, None)
        }
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let mut results: Vec<DeviceClose> = stream::iter(devices)
        .map(|udid| {
            let control = control.clone();
            async move { close_device(&control, &udid).await }
        })
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;
    results.extend(disconnected);
    for result in &results {
        match result.status {
            CloseStatus::Closed => log::info!("exit: TikTok closed on {}", result.udid),
            _ => log::warn!(
                "exit: TikTok {:?} on {}: {}",
                result.status,
                result.udid,
                result.reason.as_deref().unwrap_or("")
            ),
        }
    }
    let record = serde_json::json!({
        "startedAt": started,
        "finishedAt": chrono::Utc::now().to_rfc3339(),
        "listingError": listing_error,
        "devices": results,
    });
    if let Err(error) = db.set_setting(EXIT_TIKTOK_CLOSE_SETTING, &record.to_string()) {
        log::error!("exit: could not persist TikTok close results: {error:#}; {record}");
    }
}

async fn close_device(control: &DeviceControlPlane, udid: &str) -> DeviceClose {
    let mut result = DeviceClose::new(udid);
    // Exit is an operator-level close, so it takes a ManualControl lease: IdleSweep's
    // pending-publication guard would leave TikTok running on exactly the phones that owe a
    // link. Stopping the app deletes no DB intent, receipt or link debt; the next launch's
    // verification still owns those, and nothing here replays a Post.
    let context = match acquire(control, udid).await {
        Ok(context) => context,
        Err(error) => {
            return result.with(CloseStatus::Failed, format!("không nhận được máy: {error}"))
        }
    };
    let outcome = async {
        // Read raw package inventory under the same lease. The label-enriched app list
        // may attach/provision a helper, which must never happen during shutdown.
        // Only this non-mutating read is deadline-bound; dispatched stops still drain.
        let listing = match tokio::time::timeout(
            INVENTORY_DEADLINE,
            control.device_shell(&context, "cmd package list packages --user 0"),
        )
        .await
        {
            Ok(listing) => listing.map_err(|error| {
                (
                    CloseStatus::Unknown,
                    format!("không đọc được danh sách app: {error}"),
                )
            })?,
            Err(_) => {
                return Err((
                    CloseStatus::Unknown,
                    "hết thời gian đọc danh sách app".into(),
                ))
            }
        };
        if listing.exit_code != 0 || !listing.stderr.trim().is_empty() {
            return Err((
                CloseStatus::Unknown,
                format!(
                    "danh sách app không đọc được (exit {}): {}",
                    listing.exit_code,
                    listing.stderr.trim()
                ),
            ));
        }
        let mut packages = Vec::new();
        let mut rows = 0;
        for line in listing.stdout.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let Some(package) = line.strip_prefix("package:").filter(|package| {
                !package.is_empty()
                    && package
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_'))
            }) else {
                return Err((
                    CloseStatus::Unknown,
                    "danh sách app trả về dòng không đọc được".into(),
                ));
            };
            rows += 1;
            if riviu_core::tiktok_target::is_measured_android_tiktok(package) {
                packages.push(package.to_owned());
            }
        }
        if rows == 0 {
            return Err((
                CloseStatus::Unknown,
                "danh sách app không có dòng gói".into(),
            ));
        }
        packages.sort();
        packages.dedup();
        result.packages = packages.clone();
        stop_and_prove(control, &context, &packages).await
    }
    .await;
    // Inventory failures and all stop/proof outcomes release the acquired context here.
    let released = control.close_exclusive_context(context);
    let mut result = match outcome {
        Ok(remaining) if remaining.is_empty() => {
            result.status = CloseStatus::Closed;
            result
        }
        Ok(remaining) => {
            result.remaining = remaining;
            result.with(
                CloseStatus::Failed,
                "TikTok vẫn còn tiến trình sau khi dừng",
            )
        }
        Err((status, reason)) => result.with(status, reason),
    };
    if let Err(error) = released {
        log::warn!("exit: lease release on {udid}: {error}");
        if result.status == CloseStatus::Closed {
            result.reason = Some(format!("đã đóng; nhả máy lỗi: {error}"));
        }
    }
    result
}

/// The drain already stopped every owner, so a busy lease is a straggler finishing its release.
async fn acquire(
    control: &DeviceControlPlane,
    udid: &str,
) -> Result<DeviceExclusiveContext, DeviceControlError> {
    let deadline = tokio::time::Instant::now() + LEASE_WAIT;
    loop {
        match control
            .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::ManualControl)
            .await
        {
            Err(DeviceControlError::Busy(_)) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(250)).await
            }
            other => return other,
        }
    }
}

/// Stop each package, then list processes; one repeat stop covers a respawned child.
async fn stop_and_prove(
    control: &DeviceControlPlane,
    context: &DeviceExclusiveContext,
    packages: &[String],
) -> Result<Vec<String>, (CloseStatus, String)> {
    let mut stop_errors = Vec::new();
    for package in packages {
        if let Err(error) = control.terminate_app(context, package).await {
            stop_errors.push(format!("{package}: {error}"));
        }
    }
    let mut remaining = running_processes(control, context).await?;
    if !remaining.is_empty() {
        for package in packages {
            let _ = control.terminate_app(context, package).await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        remaining = running_processes(control, context).await?;
    }
    if remaining.is_empty() {
        // Absence observed in `ps` outranks a `pidof` timeout reported by `terminate_app`.
        return Ok(remaining);
    }
    if !stop_errors.is_empty() {
        log::warn!("exit: terminate errors: {}", stop_errors.join("; "));
    }
    Ok(remaining)
}

async fn running_processes(
    control: &DeviceControlPlane,
    context: &DeviceExclusiveContext,
) -> Result<Vec<String>, (CloseStatus, String)> {
    let listing = control
        .device_shell(context, "ps -A -o PID,NAME")
        .await
        .map_err(|error| (CloseStatus::Unknown, format!("không đọc được ps: {error}")))?;
    let mut lines = listing.stdout.lines();
    let header_ok = lines
        .next()
        .is_some_and(|header| header.split_whitespace().eq(["PID", "NAME"]));
    if listing.exit_code != 0 || !header_ok || !listing.stderr.trim().is_empty() {
        return Err((
            CloseStatus::Unknown,
            format!(
                "ps không đọc được (exit {}): {}",
                listing.exit_code,
                listing.stderr.trim()
            ),
        ));
    }
    let mut remaining = Vec::new();
    let mut rows = 0;
    for line in lines.filter(|line| !line.trim().is_empty()) {
        let columns: Vec<_> = line.split_whitespace().collect();
        if columns.len() != 2 || columns[0].parse::<u64>().ok().is_none_or(|pid| pid == 0) {
            return Err((CloseStatus::Unknown, "ps trả về dòng không đọc được".into()));
        }
        rows += 1;
        let name = columns[1];
        // Prove all measured packages absent, even when inventory returned no installed app.
        if riviu_core::tiktok_target::measured_android_packages().any(|package| {
            name == package
                || name
                    .strip_prefix(package)
                    .is_some_and(|rest| rest.starts_with(':'))
        }) {
            remaining.push(format!("{} {name}", columns[0]));
        }
    }
    if rows == 0 {
        return Err((CloseStatus::Unknown, "ps không có dòng tiến trình".into()));
    }
    Ok(remaining)
}
