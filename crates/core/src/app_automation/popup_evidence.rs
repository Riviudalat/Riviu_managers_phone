//! Review captures of popups Riviu did not answer: a redacted hierarchy plus a screenshot
//! under the app data `artifacts/popups` folder. Evidence only; it never changes a verdict.
use crate::{tiktok_labels::TikTokControls, ui_automation::tree::Tree};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Mutex, OnceLock};
use std::time::Duration;
use tokio::time::Instant;

static ROOT: OnceLock<PathBuf> = OnceLock::new();
static TAKEN: OnceLock<Mutex<HashMap<&'static str, u32>>> = OnceLock::new();

/// Captures per popup kind for one process. A popup that never goes away is reviewed from
/// its first captures; it must not fill the disk.
const MAX_PER_KIND: u32 = 20;
/// The screenshot is optional evidence and never spends more than this of a caller window.
const SCREENSHOT_WINDOW: Duration = Duration::from_secs(3);

/// Set once at startup by the host, beside the other artifact folders.
pub fn set_root(path: PathBuf) {
    let _ = ROOT.set(path);
}

fn admit(kind: &'static str) -> bool {
    let taken = TAKEN.get_or_init(|| Mutex::new(HashMap::new()));
    let mut taken = taken
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let count = taken.entry(kind).or_insert(0);
    if *count >= MAX_PER_KIND {
        return false;
    }
    *count += 1;
    true
}

fn path_part(value: &str) -> String {
    let part: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if part.is_empty() {
        "khong-ro".into()
    } else {
        part
    }
}

/// Save `tree` (redacted) and a fresh screenshot for review. Failures are logged and
/// swallowed: missing evidence never blocks or unblocks the caller's step.
pub(crate) async fn capture(
    session: &dyn crate::UiSession,
    tree: &Tree,
    labels: TikTokControls,
    kind: &'static str,
    reason: &str,
    deadline: Instant,
    stop: &AtomicBool,
) {
    let Some(root) = ROOT.get() else {
        return;
    };
    if !admit(kind) {
        return;
    }
    let device = session
        .gui_scope()
        .map(|scope| scope.device_id)
        .unwrap_or_default();
    let now = chrono::Local::now();
    let directory = root.join(now.format("%Y%m%d").to_string()).join(format!(
        "{}-{}-{}",
        now.format("%H%M%S%.3f"),
        path_part(kind),
        path_part(&device)
    ));
    let window = deadline.min(Instant::now() + SCREENSHOT_WINDOW);
    let screenshot = match crate::ui_automation::runtime::read_before_deadline(
        session.screenshot_png(),
        window,
        stop,
    )
    .await
    {
        Ok(crate::ui_automation::runtime::ReadWaitResult::Ready(png)) => Some(png),
        _ => None,
    };
    let meta = serde_json::json!({
        "kind": kind,
        "reason": reason,
        "package": labels.package(),
        "version": labels.resource_version(),
        "language": labels.language(),
        "generation": tree.generation,
        "device": device,
        // Matches the capture to the phone's log lines when no GUI scope names the device.
        "sessionEpoch": session.gui_session_epoch(),
        "capturedAt": now.to_rfc3339(),
        "screenshot": screenshot.is_some(),
    });
    let xml = tree.redacted_xml();
    let written = async {
        tokio::fs::create_dir_all(&directory).await?;
        tokio::fs::write(directory.join("hierarchy.xml"), xml).await?;
        if let Some(png) = screenshot {
            tokio::fs::write(directory.join("screen.png"), png).await?;
        }
        tokio::fs::write(directory.join("meta.json"), meta.to_string()).await?;
        anyhow::Ok(())
    }
    .await;
    match written {
        Ok(()) => {
            tracing::info!(kind, path = %directory.display(), "đã lưu bằng chứng popup để rà soát")
        }
        Err(error) => tracing::warn!(kind, %error, "không lưu được bằng chứng popup"),
    }
}
