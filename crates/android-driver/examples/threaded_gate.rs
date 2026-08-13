//! Gate H5: prove a reply attaches to the **right parent**, across two real phones.
//!
//! This is the only test that can prove it. Everything else about the reply path can pass
//! while the reply lands under a stranger's comment: the geometry rules have unit tests,
//! but "the nearest reply control below this body" is a claim about a real screen, and a
//! wrong answer is invisible in a log — it looks exactly like a success.
//!
//! Two devices, because that is what a thread is: each message in a chain is sent from a
//! *different* actor, so device B has to find device A's comment on a screen it opened
//! itself, with TikTok having re-ranked the list in between.
//!
//! ```text
//! RIVIU_ADB_PATH=… RIVIU_TIKTOK_PACKAGE=com.ss.android.ugc.trill \
//!   cargo run -p riviu-android-driver --example threaded_gate -- \
//!     <serial-A> <serial-B> <url> "<root text>" "<reply text>"
//! ```
//!
//! **It posts two public comments** — a root from A and a reply from B — under whichever
//! accounts those phones are logged into. Both texts come from the command line rather
//! than being invented here.
//!
//! What it checks, in order, and every one is a shipped function:
//!
//! 1. A arrives at the post (`open_target_by_hierarchy`) and posts a root
//!    (`send_root_by_hierarchy`), then **reads its own comment back** — that read-back is
//!    the parent identity, and without it there is nothing to reply to.
//! 2. B arrives at the same post independently and replies to that identity
//!    (`send_reply_by_hierarchy`), which locates the row by the exact text A typed, taps
//!    *that row's* reply control, and refuses before typing unless the composer's
//!    placeholder names A's author.
//! 3. A screenshot of B's screen is saved, because the nesting has to be seen.

use std::sync::atomic::AtomicBool;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use riviu_android_driver::{AndroidDriver, AndroidDriverConfig};
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::interaction::CommentLocatorIdentity;
use riviu_core::interaction_hierarchy::{
    open_target_by_hierarchy, send_reply_by_hierarchy, send_root_by_hierarchy, TargetArrival,
};
use riviu_core::tiktok_labels::{self, TikTokControls};

static TIKTOK: LazyLock<String> = LazyLock::new(|| {
    std::env::var("RIVIU_TIKTOK_PACKAGE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "com.ss.android.ugc.trill".to_string())
});

/// One phone, opened and with its labels resolved.
struct Actor {
    serial: String,
    session: Box<dyn UiSession>,
    labels: TikTokControls,
    screen: (f64, f64),
}

/// Open a device and resolve its labels.
///
/// Resolved **per device**, not once: the two phones in this fleet run different TikTok
/// versions (46.3.3 and 46.4.3) and the drawer's Send button is a different resource id on
/// each, so one shared label set would refuse on one of them.
async fn open_actor(driver: &AndroidDriver, serial: &str) -> anyhow::Result<Actor> {
    driver.launch_app(serial, TIKTOK.as_str()).await?;
    tokio::time::sleep(Duration::from_secs(10)).await;
    let session = driver.open_session(serial).await?;
    let language = session.ui_language().await.unwrap_or_default();
    let app_version = session
        .app_version(TIKTOK.as_str())
        .await
        .unwrap_or_default();
    let labels =
        tiktok_labels::controls_for(TIKTOK.as_str(), &language, &app_version).ok_or_else(|| {
            anyhow::anyhow!("no measured labels for {} + {language:?}", TIKTOK.as_str())
        })?;
    let screen = session.window_size().await?;
    println!("  {serial}: {language:?} app {app_version:?} screen {screen:?}");
    println!("    labels: {}", labels.provenance());
    if labels
        .label(riviu_core::tiktok_labels::TikTokControl::CommentSend)
        .is_none()
    {
        anyhow::bail!("{serial}: no measured Send control for app {app_version:?}");
    }
    Ok(Actor {
        serial: serial.to_string(),
        session: Box::new(session),
        labels,
        screen,
    })
}

/// Arrive at the post, reporting the proof level rather than assuming one.
///
/// Foregrounds TikTok and **proves it** before opening the link, which is exactly what
/// `DeviceControlPlane::start_interaction_session` does in the app. Not decoration: setting
/// up the second phone takes half a minute, and in that time the first one's TikTok had
/// gone back to the launcher — the arrival check then correctly reported
/// `target_open_wrong_app` for `com.miui.home`. Doing this per arrival makes the gate
/// order-independent for the same reason the product does it per assignment.
async fn arrive(actor: &Actor, url: &str, handle: &str) -> anyhow::Result<()> {
    actor
        .session
        .launch_app_foreground(TIKTOK.as_str())
        .await
        .or_else(|_| Ok::<(), anyhow::Error>(()))?;
    let mut foreground = String::new();
    for _ in 0..20 {
        foreground = actor.session.active_app_bundle().await.unwrap_or_default();
        if foreground == TIKTOK.as_str() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    anyhow::ensure!(
        foreground == TIKTOK.as_str(),
        "{}: TikTok would not come to the foreground (saw {foreground:?})",
        actor.serial
    );

    let stop = AtomicBool::new(false);
    let started = Instant::now();
    match open_target_by_hierarchy(
        actor.session.as_ref(),
        actor.labels,
        TIKTOK.as_str(),
        url,
        handle,
        &stop,
    )
    .await
    {
        Ok(TargetArrival::Identified { author_label }) => {
            println!(
                "  {}: arrival Identified ({author_label}) in {} ms",
                actor.serial,
                started.elapsed().as_millis()
            );
            Ok(())
        }
        Ok(TargetArrival::Structural) => {
            println!(
                "  {}: arrival Structural in {} ms",
                actor.serial,
                started.elapsed().as_millis()
            );
            Ok(())
        }
        Err(refusal) => anyhow::bail!(
            "{}: arrival refused ({}): {}",
            actor.serial,
            refusal.code(),
            refusal.message()
        ),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 5 {
        println!(
            "usage: threaded_gate <serial-A> <serial-B> <url> \"<root text>\" \"<reply text>\"\n\
             \n\
             POSTS TWO PUBLIC COMMENTS: a root from A and a reply from B."
        );
        return Ok(());
    }
    let (serial_a, serial_b, url, root_text, reply_text) =
        (&args[0], &args[1], &args[2], &args[3], &args[4]);
    let handle = url
        .split('@')
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default()
        .to_string();

    let driver = AndroidDriver::new(&AndroidDriverConfig::default())?;
    println!("== opening both actors ==");
    let actor_a = open_actor(&driver, serial_a).await?;
    let actor_b = open_actor(&driver, serial_b).await?;
    println!("\n  target {url}\n  handle {handle:?}");

    // ---- A: arrive and post the root ----
    println!("\n== A posts the root ==");
    arrive(&actor_a, url, &handle).await?;
    let stop = AtomicBool::new(false);
    let root = send_root_by_hierarchy(
        actor_a.session.as_ref(),
        actor_a.labels,
        actor_a.screen,
        root_text,
        &stop,
        String::new,
    )
    .await?;
    println!("  verdict = {:?} ({})", root.verdict, root.verdict.reason());
    let Some(parent) = root.identity.clone() else {
        anyhow::bail!(
            "A posted (verdict {:?}) but its comment could not be read back \
             unambiguously, so there is no parent identity to reply to. A real campaign \
             stops the chain here rather than replying to a row nobody confirmed.",
            root.verdict
        );
    };
    println!(
        "  parent identity: author={:?} text={:?}",
        parent.author_label, parent.text
    );

    // ---- B: arrive independently and reply to that identity ----
    println!("\n== B replies to A's comment ==");
    arrive(&actor_b, url, &handle).await?;
    // The identity travels verbatim. Nothing is re-derived on B's side: the whole point is
    // that B finds the row by the exact string A typed.
    let parent_for_b = CommentLocatorIdentity { ..parent.clone() };
    // Shot on the way in as well as on the way out. The reply path leaves the drawer open
    // by contract, but if that ever breaks the exit shot shows a closed drawer and proves
    // nothing about nesting — which is exactly what happened the first time this gate ran.
    let before_shot = std::env::temp_dir().join("riviu-gate-h5-before.png");
    let _ = driver.screenshot(&actor_b.serial, &before_shot).await;
    let reply = send_reply_by_hierarchy(
        actor_b.session.as_ref(),
        actor_b.labels,
        actor_b.screen,
        &parent_for_b,
        reply_text,
        &stop,
        String::new,
    )
    .await?;
    match reply {
        Ok(outcome) => {
            println!(
                "  verdict = {:?} ({})",
                outcome.verdict,
                outcome.verdict.reason()
            );
            match &outcome.identity {
                Some(identity) => println!(
                    "  read back: author={:?} text={:?}",
                    identity.author_label, identity.text
                ),
                None => println!("  ! the reply could not be read back unambiguously"),
            }
            if outcome.verdict.is_sent() {
                println!(
                    "\n  GATE H5: the reply was sent and the disarm confirmed.\n  \
                     LOOK AT THE SCREENSHOT — nesting is the claim, and it cannot be \
                     proved from a log line."
                );
            } else {
                println!("\n  GATE H5 INCOMPLETE: {}", outcome.verdict.reason());
            }
        }
        Err(refusal) => println!(
            "  reply refused ({}): {}\n  nothing was typed.",
            refusal.code(),
            refusal.message()
        ),
    }

    // The screenshot is the deliverable, not a nicety: a reply attached to the wrong
    // parent produces an identical log.
    let path = std::env::temp_dir().join("riviu-gate-h5-reply.png");
    let written = driver.screenshot(&actor_b.serial, &path).await?;
    let bytes = tokio::fs::metadata(&written)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0);
    println!("\n  B's screen: {} ({bytes} bytes)", written.display());
    Ok(())
}
