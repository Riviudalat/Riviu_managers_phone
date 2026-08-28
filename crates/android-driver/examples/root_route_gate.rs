//! Prove which privilege route each phone has, and that the wipe stays gated.
//!
//! ```text
//! cargo run -p riviu-android-driver --example root_route_gate -- [serial]
//! ```
//!
//! **"Rooted" was one question doing the work of two, and this fleet disagrees on nine of
//! twenty phones.** Measured 27/08/2026: the nine SM-G950F run `adbd` as uid 0
//! (`context=u:r:su:s0`) and carry **no `su` binary**; the eleven SM-G955F/N/U1 run as uid 2000
//! and also carry none. So `su -c id` fails everywhere, `is_rooted` answered `false` for the
//! whole fleet, and `root_shell` refused on nine phones where the command would have run.
//!
//! Split by operator decision on 28/08/2026: `root_shell` and the `serialno`/`mac` half of
//! `set_device_identity` accept either route; `factory_reset` still demands `su`, so an
//! irreversible remote wipe does **not** become reachable on nine phones as a side effect.
//!
//! This gate exists because that last sentence is the one nobody can verify by reading. It runs
//! a harmless privileged command and asks `factory_reset`'s gate what it would say — it never
//! calls `factory_reset`.
//!
//! Read-only on the phone: `id -u` and one `cat` of a root-only file. Nothing is written,
//! nothing is wiped.

use riviu_core::driver::DeviceDriver;

#[path = "common/mod.rs"]
mod common;

/// Readable only by a privileged shell on every Android here, and harmless to read.
const ROOT_ONLY_READ: &str = "cat /data/system/packages.list | head -n 1";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let wanted = std::env::args().nth(1);

    let config = common::repo_config();
    println!("{}", common::describe_adb(&config));
    let driver = riviu_android_driver::detect_driver(&config)
        .await
        .map_err(|reason| anyhow::anyhow!("no usable adb on this host: {reason}"))?;

    let devices = driver.list_devices().await?;
    let serials: Vec<String> = match wanted {
        Some(serial) => {
            anyhow::ensure!(
                devices.iter().any(|device| device.udid == serial),
                "adb does not see {serial}"
            );
            vec![serial]
        }
        None => devices.iter().map(|device| device.udid.clone()).collect(),
    };
    anyhow::ensure!(!serials.is_empty(), "adb sees no device");
    println!("{} máy\n", serials.len());

    let mut failures: Vec<String> = Vec::new();
    let (mut su_count, mut shell_root_count, mut plain_count) = (0usize, 0usize, 0usize);

    for serial in &serials {
        let has_su = driver.is_rooted(serial).await;
        let privileged = driver.can_run_privileged(serial).await;
        let shell_is_root = !has_su && privileged;

        let route = if has_su {
            su_count += 1;
            "su"
        } else if shell_is_root {
            shell_root_count += 1;
            "adb shell là root"
        } else {
            plain_count += 1;
            "không có đặc quyền"
        };

        // The claim under test: a phone reported as privileged really can run one.
        let ran = driver.root_shell(serial, ROOT_ONLY_READ).await;
        let consistent = match (&ran, privileged) {
            (Ok(_), true) => true,
            (Err(_), false) => true,
            // Reported privileged and refused, or the reverse. Either way the report is a lie.
            _ => false,
        };
        if !consistent {
            failures.push(format!(
                "{serial}: can_run_privileged={privileged} nhưng root_shell {} — {}",
                if ran.is_ok() {
                    "chạy được"
                } else {
                    "bị từ chối"
                },
                ran.as_ref()
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_default()
            ));
        }

        // **`factory_reset` must still refuse every phone without `su`.** Asked through the
        // same predicate the command itself uses, never by calling it.
        let wipe_allowed = driver.is_rooted(serial).await;
        if wipe_allowed != has_su {
            failures.push(format!("{serial}: cổng khôi phục gốc không còn khớp `su`"));
        }
        if shell_is_root && wipe_allowed {
            failures.push(format!(
                "{serial}: adb shell là root và KHÔI PHỤC GỐC đã mở — đúng thứ quyết định \
                 28/08/2026 nói phải giữ đóng"
            ));
        }

        println!(
            "  {serial}  route={route:<22} root_shell={:<12} khôi phục gốc={}",
            if ran.is_ok() {
                "chạy được"
            } else {
                "từ chối"
            },
            if wipe_allowed { "MỞ" } else { "đóng" }
        );
    }

    println!(
        "\ntổng: {su_count} máy có su · {shell_root_count} máy adb shell là root · \
         {plain_count} máy không đặc quyền"
    );

    if failures.is_empty() {
        println!("\nĐẠT — mỗi máy báo đúng lối nó có, và khôi phục gốc chỉ mở khi có su");
        return Ok(());
    }
    for line in &failures {
        println!("KHÔNG ĐẠT  {line}");
    }
    anyhow::bail!("{} phép kiểm thất bại", failures.len())
}
