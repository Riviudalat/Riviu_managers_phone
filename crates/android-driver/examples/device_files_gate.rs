//! Prove the file manager on a real phone, headless.
//!
//! ```text
//! cargo run -p riviu-android-driver --example device_files_gate -- <serial>
//! ```
//!
//! Every fix this checks was a *silent* one — the UI showed a plausible answer and the answer
//! was wrong — so none of them can be confirmed by looking at the screen. That is the whole
//! reason this is a Rust example calling the production functions rather than a click-through:
//! driving the app by mouse once posted a real comment to a real account (AGENTS.md §9.112),
//! and a listing that lies looks exactly like a listing that is right.
//!
//! Six claims, each measured against the phone rather than against the code:
//!
//! 1. **A refused directory says refused, not empty.** `/data/data` is unreadable without
//!    root on every fleet phone. The old parser read exit 0 + stderr as "no entries", so the
//!    UI drew an empty folder — and an empty folder is a claim, not a shrug.
//! 2. **A file name with an apostrophe survives the round trip.** `quote_device_path` used to
//!    wrap in single quotes and stop there, so `it's.txt` broke the shell word and the delete
//!    hit something else or nothing.
//! 3. **The size is the real byte count.** The old parser took the size by counting columns
//!    from the timestamp, so a ROM printing one column fewer reported every file as 0 B.
//! 4. **The name is read verbatim.** A `ls` that prints `Jul 11 11:16` instead of an ISO date
//!    used to yield the name `"11 11:16 photo.jpg"` — and that fabricated name then went into
//!    `pull` and `rm -rf`. Both measured ROMs print ISO, so this is latent here; the gate
//!    still asserts it, because the fleet is not one ROM forever.
//! 5. **Push, pull and delete each confirm by reading back**, rather than by exit code.
//! 6. **A truncated listing is reported as truncated.** Nothing on a healthy phone truncates,
//!    so this one only reports what it saw — a claim it cannot make is not a claim it fakes.
//!
//! Writes: creates and deletes one file under `/sdcard/Download`. Nothing else on the phone
//! changes, and the file is removed before the gate exits even if an assertion fails.

use std::path::{Path, PathBuf};

use riviu_core::DeviceFileKind;

#[path = "common/mod.rs"]
mod common;

/// A name that breaks a naive single-quote wrapper, plus a space and a unicode character.
const AWKWARD_NAME: &str = "riviu it's a gate ✓.txt";
const BODY: &[u8] = b"riviu device_files_gate\n";
const SANDBOX: &str = "/sdcard/Download";
/// Unreadable without root on every phone in this fleet, and readable-with-root nowhere we
/// run this. If a rooted phone ever makes this pass, the gate says so rather than failing.
const REFUSED_DIR: &str = "/data/data";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let serial = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: device_files_gate <serial>"))?;

    let config = common::repo_config();
    println!("{}", common::describe_adb(&config));
    let driver = riviu_android_driver::detect_driver(&config)
        .await
        .map_err(|reason| anyhow::anyhow!("no usable adb on this host: {reason}"))?;
    common::require_device(&driver, &serial).await?;
    println!("máy      {serial}\n");

    let mut failures: Vec<String> = Vec::new();

    if let Err(reason) = refused_reads_as_refused(&driver, &serial).await {
        failures.push(format!("1. thư mục bị từ chối: {reason:#}"));
    }
    if let Err(reason) = round_trip_one_awkward_file(&driver, &serial).await {
        failures.push(format!("2-5. vòng đẩy/đọc/kéo/xoá: {reason:#}"));
    }
    if let Err(reason) = listing_says_when_it_is_partial(&driver, &serial).await {
        failures.push(format!("6. danh sách bị cắt: {reason:#}"));
    }

    println!();
    if failures.is_empty() {
        println!("ĐẠT — cả sáu khẳng định đúng trên {serial}");
        return Ok(());
    }
    for line in &failures {
        println!("KHÔNG ĐẠT  {line}");
    }
    anyhow::bail!("{} phép kiểm thất bại", failures.len())
}

/// Claim 1: unreadable is not empty.
async fn refused_reads_as_refused(
    driver: &riviu_android_driver::AndroidDriver,
    serial: &str,
) -> anyhow::Result<()> {
    // A refusal may arrive as an `Err` or as an `incomplete` listing depending on what the
    // ROM writes where; both are honest. Only "no entries, no reason" is the bug.
    match driver.list_device_dir(serial, REFUSED_DIR).await {
        Err(reason) => {
            println!("1. {REFUSED_DIR}: lỗi có lý do — {reason:#}");
            Ok(())
        }
        Ok(listing) => match (&listing.incomplete, listing.entries.len()) {
            (Some(reason), n) => {
                println!("1. {REFUSED_DIR}: {n} dòng đọc được, và nói rõ phần thiếu — {reason}");
                Ok(())
            }
            (None, 0) => anyhow::bail!(
                "0 dòng và KHÔNG có lý do — đây đúng là lỗi cũ: 'không đọc được' vẽ thành \
                 'thư mục rỗng'"
            ),
            (None, n) => {
                // A rooted phone can genuinely read this. Say so; do not call it a pass or
                // a failure of the parser.
                let rooted = driver.is_rooted(serial).await;
                println!(
                    "1. {REFUSED_DIR}: đọc được {n} dòng, không phần thiếu (máy rooted: \
                     {rooted}) — phép kiểm này không áp dụng cho máy này"
                );
                Ok(())
            }
        },
    }
}

/// Claims 2-5: one awkward file, pushed, listed, pulled, deleted, each step read back.
async fn round_trip_one_awkward_file(
    driver: &riviu_android_driver::AndroidDriver,
    serial: &str,
) -> anyhow::Result<()> {
    let host_dir = std::env::temp_dir().join(format!("riviu-files-gate-{serial}"));
    std::fs::create_dir_all(&host_dir)?;
    let local = host_dir.join(AWKWARD_NAME);
    std::fs::write(&local, BODY)?;

    let remote = format!("{SANDBOX}/{AWKWARD_NAME}");
    // Anything left from an interrupted earlier run would make "it is there" meaningless.
    let _ = driver.delete_device_path(serial, &remote).await;

    let outcome = round_trip_body(driver, serial, &local, &host_dir).await;

    // Delete is both the last assertion and the cleanup, so it runs either way.
    let deleted = driver.delete_device_path(serial, &remote).await;
    let _ = std::fs::remove_dir_all(&host_dir);

    outcome?;

    deleted.map_err(|reason| anyhow::anyhow!("xoá {remote}: {reason:#}"))?;
    let after = driver.list_device_dir(serial, SANDBOX).await?;
    anyhow::ensure!(
        !after.entries.iter().any(|entry| entry.name == AWKWARD_NAME),
        "đã xoá mà đọc lại vẫn thấy {AWKWARD_NAME} — exit code nói xoá được, phone nói khác"
    );
    println!("5. xoá: đọc lại xác nhận đã mất");
    Ok(())
}

async fn round_trip_body(
    driver: &riviu_android_driver::AndroidDriver,
    serial: &str,
    local: &Path,
    host_dir: &Path,
) -> anyhow::Result<()> {
    let pushed = driver
        .push_device_file(serial, local, SANDBOX)
        .await
        .map_err(|reason| anyhow::anyhow!("đẩy {}: {reason:#}", local.display()))?;
    println!("2. đẩy: {pushed}");

    let listing = driver.list_device_dir(serial, SANDBOX).await?;
    if let Some(reason) = &listing.incomplete {
        anyhow::bail!("{SANDBOX} trả về danh sách không đầy đủ: {reason}");
    }
    let entry = listing
        .entries
        .iter()
        .find(|entry| entry.name == AWKWARD_NAME)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "đẩy xong mà không thấy tên NGUYÊN VĂN {AWKWARD_NAME:?} trong {SANDBOX}. \
                 Các tên gần giống: {:?}",
                listing
                    .entries
                    .iter()
                    .filter(|other| other.name.contains("gate"))
                    .map(|other| other.name.as_str())
                    .collect::<Vec<_>>()
            )
        })?;
    println!("3. tên: đọc ra nguyên văn {:?}", entry.name);

    anyhow::ensure!(
        entry.kind == DeviceFileKind::File,
        "{AWKWARD_NAME} đọc ra là {:?}, không phải File",
        entry.kind
    );
    anyhow::ensure!(
        entry.size == BODY.len() as u64,
        "size đọc ra {} byte, thật là {} — đây đúng là lỗi lấy cột theo vị trí mốc thời gian",
        entry.size,
        BODY.len()
    );
    println!("4. size: {} byte, đúng từng byte", entry.size);

    let pulled: PathBuf = driver
        .pull_device_path(serial, &format!("{SANDBOX}/{AWKWARD_NAME}"), host_dir)
        .await
        .map_err(|reason| anyhow::anyhow!("kéo về: {reason:#}"))?;
    let back = std::fs::read(&pulled)
        .map_err(|reason| anyhow::anyhow!("đọc {}: {reason}", pulled.display()))?;
    anyhow::ensure!(
        back == BODY,
        "kéo về {} byte khác nội dung đã đẩy ({} byte)",
        back.len(),
        BODY.len()
    );
    println!("4b. kéo: {} byte khớp nội dung", back.len());
    Ok(())
}

/// Claim 6: a partial listing is labelled partial.
///
/// Reports rather than asserts. Nothing on a healthy phone truncates `ls`, so a gate that
/// demanded a truncation here would only ever be red for the wrong reason. What it can prove
/// is that the field exists and is consulted, and that `/` — the one directory on this fleet
/// with rows the phone cannot stat — comes back labelled rather than quietly short.
async fn listing_says_when_it_is_partial(
    driver: &riviu_android_driver::AndroidDriver,
    serial: &str,
) -> anyhow::Result<()> {
    let root = driver.list_device_dir(serial, "/").await?;
    let unstattable = root
        .entries
        .iter()
        .filter(|entry| entry.modified.is_none())
        .count();
    match &root.incomplete {
        Some(reason) => println!(
            "6. /: {} dòng, {unstattable} dòng không stat được, và phần thiếu được nói ra — \
             {reason}",
            root.entries.len()
        ),
        None => println!(
            "6. /: {} dòng, {unstattable} dòng không stat được, không phần nào bị cắt",
            root.entries.len()
        ),
    }
    anyhow::ensure!(
        !root.entries.is_empty(),
        "/ trả về 0 dòng, mà / không bao giờ rỗng — phép đọc đang hỏng"
    );
    Ok(())
}
