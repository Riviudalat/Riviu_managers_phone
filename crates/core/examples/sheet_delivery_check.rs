//! Prove the Sheet delivery path end to end — **without a phone, and without guessing.**
//!
//! ```text
//! cargo run -p riviu-core --example sheet_delivery_check                       # dry: print the payload
//! cargo run -p riviu-core --example sheet_delivery_check -- --webhook <url> --token <token>
//! ```
//!
//! # What it exercises
//!
//! The half of the path that lives on this machine, in the production functions, not copies:
//! a real SQLite database, a real campaign and assignment, the one-transaction success write
//! (`record_publish_success_with_sheet_row`), the pending-row read the sweeper does, and the
//! exact `SheetRow` those rows turn into. With no flags it stops there and prints the JSON
//! that would go on the wire — which is the thing an operator can compare against the Apps
//! Script's own field names before deploying anything.
//!
//! With `--webhook` it takes the last hop too, through `publish_sheet::push_row` — the same
//! call the sweeper makes — and then marks the row `sent` or `failed` exactly as the sweeper
//! would, so the DB ends in the state a real delivery leaves behind.
//!
//! # It writes to a scratch database, never the operator's
//!
//! The path is a temp file this program creates and deletes. Nothing here touches the
//! desktop app's database, and nothing here touches a phone.

use std::path::PathBuf;

use riviu_core::db::Database;
use riviu_core::publish::{
    PublishBundle, PublishCampaignRequest, PublishCleanupPolicy, PublishVisibility,
};
use riviu_core::publish_sheet::{self, SheetRow};

fn say(line: &str) {
    use std::io::Write;
    println!("{line}");
    let _ = std::io::stdout().flush();
}

const KNOWN_FLAGS: &[&str] = &["--webhook", "--token", "--assignment", "--twice"];

/// A boolean switch: off, on, or passed twice — refused like every other repeat.
fn switch(args: &[String], flag: &str) -> Result<bool, String> {
    match args.iter().filter(|arg| *arg == flag).count() {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(format!(
            "{flag} xuất hiện nhiều lần — không đoán lần nào là thật"
        )),
    }
}

fn refuse_unknown_flags(args: &[String]) -> Result<(), String> {
    let unknown: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|arg| arg.starts_with("--") && !KNOWN_FLAGS.contains(arg))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(format!(
        "không hiểu cờ {unknown:?}; tool này chỉ có {KNOWN_FLAGS:?}"
    ))
}

/// An optional flag with a value: absent, present-and-usable, or a refusal. Never a guess —
/// a mistyped `--webhook` that read as "absent" would silently downgrade a delivery run into
/// a dry run and report success.
fn value_of(args: &[String], flag: &str) -> Result<Option<String>, String> {
    let mut occurrences = args.iter().enumerate().filter(|(_, arg)| *arg == flag);
    let Some((at, _)) = occurrences.next() else {
        return Ok(None);
    };
    if occurrences.next().is_some() {
        return Err(format!(
            "{flag} xuất hiện nhiều lần — không đoán lần nào là thật"
        ));
    }
    match args.get(at + 1) {
        Some(value) if !value.starts_with("--") && !value.trim().is_empty() => {
            Ok(Some(value.clone()))
        }
        _ => Err(format!("{flag} cần một giá trị đi ngay sau nó")),
    }
}

fn bundle(id: &str) -> PublishBundle {
    PublishBundle {
        id: id.to_string(),
        source_path: "/fixture/root/bo1".into(),
        name: "bo1".into(),
        media_kind: riviu_core::publish::PublishMediaKind::Image,
        images: vec![riviu_core::publish::PublishImage {
            path: "/fixture/root/bo1/01.jpg".into(),
            file_name: "01.jpg".into(),
            order: 0,
            sha256: "52143c1adc509dc364626e36d0c2cd944b44c17283cfe1d90780a960a69ad795".into(),
            byte_len: 1024,
            width: 1080,
            height: 1350,
        }],
        caption_path: "/fixture/root/bo1/caption.txt".into(),
        caption: "Đà Lạt chiều nay.".into(),
        caption_sha256: "617f8871d042f285fcdcc5a2a4acf690f222d0b1d51ce788001783981a4bf8d0".into(),
        total_bytes: 1024,
        partners: vec!["Quán A".into(), "Quán B".into()],
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(complaint) = refuse_unknown_flags(&args) {
        say(&complaint);
        anyhow::bail!("{complaint}");
    }
    let webhook = match value_of(&args, "--webhook") {
        Ok(value) => value,
        Err(complaint) => {
            say(&complaint);
            anyhow::bail!("{complaint}");
        }
    };
    let token = match value_of(&args, "--token") {
        Ok(value) => value,
        Err(complaint) => {
            say(&complaint);
            anyhow::bail!("{complaint}");
        }
    };
    let twice = match switch(&args, "--twice") {
        Ok(on) => on,
        Err(complaint) => {
            say(&complaint);
            anyhow::bail!("{complaint}");
        }
    };
    if webhook.is_some() != token.is_some() {
        let complaint = "--webhook và --token đi cùng nhau: gửi mà không có token thì script \
                         từ chối, và có token mà không gửi thì token nằm không";
        say(complaint);
        anyhow::bail!("{complaint}");
    }

    let path: PathBuf =
        std::env::temp_dir().join(format!("riviu-sheet-check-{}.sqlite", uuid::Uuid::new_v4()));
    let db = Database::open(&path)?;
    say(&format!("db tạm    {}", path.display()));

    // A real campaign and a real assignment, through the production creation path.
    let bundle_id = format!("bundle-{}", uuid::Uuid::new_v4());
    let request = PublishCampaignRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        source_root: "/fixture/root".into(),
        bundle_ids: vec![bundle_id.clone()],
        udids: vec!["fixture-phone".into()],
        run_at: None,
        visibility: PublishVisibility::Public,
        cleanup_policy: PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
    };
    let campaign = db.create_publish_campaign(&request, &[bundle(&bundle_id)])?;
    let assignment_id = db
        .get_publish_campaign(&campaign.id)?
        .expect("campaign exists")
        .assignments
        .first()
        .expect("one assignment")
        .id
        .clone();
    say(&format!(
        "campaign  {}\nassign    {assignment_id}",
        campaign.id
    ));

    // The success write the publish path makes: state and the owed row, one transaction.
    let post_url = "https://vt.tiktok.com/ZSVcq8mha/";
    db.record_publish_success_with_sheet_row(
        &assignment_id,
        &serde_json::json!({"state": "posted", "postUrl": post_url}).to_string(),
        &campaign.id,
        post_url,
        "@cn.qut.lt4",
        &["Quán A".to_string(), "Quán B".to_string()],
    )?;

    // Exactly what the sweeper reads.
    let pending = db.pending_publish_sheet_rows(50)?;
    say(&format!("\nhàng chờ  {} dòng", pending.len()));
    let Some(row) = pending.first() else {
        anyhow::bail!("không có hàng nào trong outbox — đường ghi một-transaction hỏng");
    };

    let payload = SheetRow {
        token: token.clone().unwrap_or_else(|| "<token>".into()),
        post_url: row.post_url.clone(),
        poster: row.poster.clone(),
        partners: row.partners.clone(),
        assignment_id: row.assignment_id.clone(),
    };
    // Printed with the token masked unless one was supplied on purpose: this output goes in
    // terminal scrollback and screenshots.
    let mut shown = serde_json::to_value(&payload)?;
    if token.is_some() {
        shown["token"] = serde_json::Value::String("«token»".into());
    }
    say(&format!(
        "\nJSON sẽ POST (đúng tên trường Apps Script đọc):\n{}",
        serde_json::to_string_pretty(&shown)?
    ));

    let outcome = match (&webhook, &token) {
        (Some(url), Some(_)) => {
            say(&format!("\nĐANG GỬI THẬT tới {url}"));
            match publish_sheet::push_row(url, &payload).await {
                Ok(()) => {
                    let marked = db.mark_publish_sheet_sent(&row.assignment_id, row.revision)?;
                    say(&format!(
                        "gửi OK — script nhận. đánh dấu 'sent': {}",
                        if marked {
                            "xong"
                        } else {
                            "revision đã đổi"
                        }
                    ));
                    // **The guarantee that protects the operator's sheet, exercised rather
                    // than trusted.** A sweeper that cannot record a delivery re-sends the
                    // same assignment, and the script must answer `duplicate` and leave the
                    // sheet alone — otherwise one post becomes two rows with the same link
                    // and nobody can tell which to delete.
                    if twice {
                        say("\nGỬI LẠI ĐÚNG assignment ĐÓ (thử chống trùng)");
                        match publish_sheet::push_row(url, &payload).await {
                            Ok(()) => {
                                say("gửi lần hai OK — script coi là trùng và KHÔNG thêm hàng \
                                 (đếm lại số dòng trên sheet để xác nhận)")
                            }
                            Err(error) => {
                                say(&format!("gửi lần hai HỎNG — {error:#}"));
                                return Err(anyhow::anyhow!("lần gửi lại thất bại: {error:#}"));
                            }
                        }
                    }
                    Ok(())
                }
                Err(error) => {
                    let reason = format!("{error:#}");
                    let _ = db.mark_publish_sheet_failed(&row.assignment_id, row.revision, &reason);
                    say(&format!("gửi HỎNG — {reason}"));
                    Err(anyhow::anyhow!(reason))
                }
            }
        }
        _ => {
            say(
                "\nKhông có --webhook: dừng ở đây, KHÔNG gửi gì. Deploy Apps Script rồi chạy \
                 lại kèm --webhook <url> --token <token>.",
            );
            Ok(())
        }
    };

    let left = db.pending_publish_sheet_rows(50)?.len();
    say(&format!("còn nợ    {left} dòng"));
    let _ = std::fs::remove_file(&path);
    outcome
}
