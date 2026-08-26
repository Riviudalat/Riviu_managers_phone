//! Gate G1: drive the real `AndroidDriver` against a real phone.
//!
//! Compiling is not evidence. This exercises the actual trait implementations
//! — `DeviceDriver` and `UiSession`, no test doubles — and times each call, so
//! the numbers in `docs/ANDROID_PROBE_REPORT_2026-08-09.md` can be checked
//! against the shipped code rather than against a shell transcript.
//!
//! ```text
//! cargo run -p riviu-android-driver --example probe -- <serial>
//! RIVIU_TIKTOK_PACKAGE=com.ss.android.ugc.trill \
//!   cargo run -p riviu-android-driver --example probe -- <serial>
//! ```
//!
//! Read-only by default. It launches TikTok, reads labels, and captures a
//! screenshot. Pass `--terminate` to also exercise the force-stop path, which
//! closes the app on the device.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use riviu_android_driver::{AndroidDriver, Locator};
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::nurture::touch::TouchPointPlanner;
use riviu_core::tiktok_labels::{self, LabelMatch, TikTokControl, TikTokControls};

#[path = "common/mod.rs"]
mod common;

/// A catalogued label becomes the driver's own locator, keeping exact vs substring
/// as measured — the comment button's label embeds a count, so exact never hits.
fn to_android_locator(label: LabelMatch) -> Locator {
    match label {
        LabelMatch::Exact(value) => Locator::Description(value.to_string()),
        LabelMatch::Contains(value) => Locator::DescriptionContains(value.to_string()),
        LabelMatch::Text(value) => Locator::Text(value.to_string()),
        LabelMatch::TextContains(value) => Locator::TextContains(value.to_string()),
    }
}

/// TikTok's Android package, overridable with `RIVIU_TIKTOK_PACKAGE`.
///
/// Not a constant, because the package name is regional: the global build is
/// `com.zhiliaoapp.musically` and the South-East Asian build is
/// `com.ss.android.ugc.trill`. A phone carrying the other one made this probe
/// fail at `launch_app` with `monkey -p … failed` and go no further, so none of
/// the agent measurements below were reachable on it. Default is unchanged.
static TIKTOK: LazyLock<String> = LazyLock::new(|| {
    std::env::var("RIVIU_TIKTOK_PACKAGE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "com.zhiliaoapp.musically".to_string())
});

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

    let driver = AndroidDriver::new(&common::repo_config())?;

    println!("== list_devices ==");
    let devices = timed!("list_devices", driver.list_devices().await)?;
    for device in &devices {
        println!(
            "  {:<20} {:<12} os={:<5} status={:?} battery={:?} agent={}",
            device.udid,
            device.model,
            device.os_version,
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
    println!("  target package: {}", TIKTOK.as_str());

    timed!(
        "launch_app(tiktok)",
        driver.launch_app(&serial, TIKTOK.as_str()).await
    )?;
    tokio::time::sleep(Duration::from_secs(8)).await;

    println!("\n== open_session (starts the agent if needed) ==");
    let session = timed!("open_session", driver.open_session(&serial).await)?;

    println!("\n== UiSession ==");
    let size = timed!("window_size", session.window_size().await)?;
    println!("    screen = {:?} (must be the wm Override size)", size);

    let bundle = timed!("active_app_bundle", session.active_app_bundle().await)?;
    println!("    foreground = {bundle}");
    if bundle != TIKTOK.as_str() {
        println!("    ! not TikTok; the label probes below will not find anything");
    }

    // Labels come from the measured catalog keyed two ways, because they fail two
    // ways. Translated `content-desc` strings are localised — the English strings find
    // nothing on a Vietnamese UI even though the rail is on screen — while unresolved
    // resource ids are language-independent and move on every app rebuild
    // (`riviu_core::tiktok_labels`). So both the locale and the app version are read
    // here, and both are printed: this is the measurement, not a lookup detail.
    println!("\n== label set for this build, UI language and app version ==");
    let locale = session.ui_locale().await;
    println!("  ui locale   = {locale:?}   (persist.sys.locale, not ro.product.locale)");
    let app_version = session
        .app_version_name(TIKTOK.as_str())
        .await
        .unwrap_or_default();
    println!("  app version = {app_version:?}   (dumpsys package … versionName)");
    let labels = locale
        .as_deref()
        .and_then(|locale| tiktok_labels::controls_for(TIKTOK.as_str(), locale, &app_version));
    let Some(labels) = labels else {
        println!(
            "  no measured label set for {} + {locale:?} — refusing the label probes rather \
             than trying another language's strings",
            TIKTOK.as_str()
        );
        println!("\nG1 probe finished (label probes skipped).");
        return Ok(());
    };
    println!("  using {}", labels.provenance());
    // Keyed on whether the Send button actually resolved, not on whether a resource set
    // matched. Those are different questions, and confusing them is what made the session
    // log warn at the eleven healthy phones on this farm while reassuring the three that
    // needed the id (see `TikTokControls::provenance`). `trill` 38.3.2 has no resource set
    // and needs none — it renders `Post comment` as text.
    if labels
        .label(riviu_core::tiktok_labels::TikTokControl::CommentSend)
        .is_none()
    {
        println!(
            "  ! the drawer Send button cannot be named on app version {app_version:?}: no              `@2131...` resource id measured for it, and this build does not render it as              text. Liking and reading still work; commenting will refuse. Run              --measure-comment and add a TIKTOK_RESOURCE_SETS entry."
        );
    }

    if let Some(feed) = labels.label(TikTokControl::FeedTab) {
        let _ = timed!(
            format!("assert_visible({:?})", feed.value()),
            session.assert_visible(feed.value()).await
        );
    }

    // The like control carries its own state in the label, which is the whole
    // reason the CV layer is not needed here.
    println!("\n== label state, the evidence that replaces pixel matching ==");
    for control in [
        TikTokControl::Like,
        TikTokControl::Liked,
        TikTokControl::Comments,
        TikTokControl::Share,
        TikTokControl::Bookmark,
        TikTokControl::LiveRoom,
    ] {
        let Some(label) = labels.label(control) else {
            println!(
                "  {:<34}   unmeasured on this build/language",
                format!("{control:?}")
            );
            continue;
        };
        let started = Instant::now();
        let outcome = session.agent().find(&to_android_locator(label)).await;
        let elapsed = started.elapsed().as_millis();
        match outcome {
            Ok(Some(element)) => {
                let desc = session
                    .agent()
                    .attribute(&element, "content-desc")
                    .await
                    .ok()
                    .flatten();
                println!(
                    "  {:<34} {:>6} ms  PRESENT  content-desc={desc:?}",
                    format!("{control:?} {:?}", label.value()),
                    elapsed
                );
            }
            Ok(None) => println!(
                "  {:<34} {:>6} ms  absent",
                format!("{control:?} {:?}", label.value()),
                elapsed
            ),
            Err(error) => println!("  {control:?} FAILED: {error}"),
        }
    }

    // The seam the hierarchy nurture loop actually uses. Deliberately called
    // through `&dyn UiSession`, not through the inherent `find_bounds`: what has
    // to work is the *trait* method, because that is all `riviu_core` can see.
    println!("\n== UiSession::locate via the trait ==");
    let ui: &dyn UiSession = &session;
    println!(
        "    supports_element_bounds = {}   ui_language = {:?}",
        ui.supports_element_bounds(),
        ui.ui_language().await
    );
    for control in [
        TikTokControl::FeedTab,
        TikTokControl::Like,
        TikTokControl::Comments,
        TikTokControl::Share,
        TikTokControl::Bookmark,
    ] {
        let Some(label) = labels.label(control) else {
            continue;
        };
        let started = Instant::now();
        let found = ui.locate(label.to_query()).await;
        let elapsed = started.elapsed().as_millis();
        match found {
            Ok(Some(element)) => {
                let centre = element.centre();
                let (rx, ry) = element.jitter_radius();
                println!(
                    "  {:<20} {:>6} ms  {:>4.0},{:>4.0} {:>3.0}x{:<3.0} tap≈{:.0},{:.0} ±{rx:.0},{ry:.0}",
                    format!("{control:?}"),
                    elapsed,
                    element.x,
                    element.y,
                    element.width,
                    element.height,
                    centre.x,
                    centre.y
                );
            }
            Ok(None) => println!("  {:<20} {:>6} ms  absent", format!("{control:?}"), elapsed),
            Err(error) => println!("  {control:?} FAILED: {error}"),
        }
    }

    // Proof that a swipe advanced the feed without looking at a single pixel: the
    // comment and share labels carry per-post counts, so the pair changes with the
    // card. Opt-in, because it moves the feed on a real account.
    if args.iter().any(|arg| arg == "--feed") {
        println!("\n== swipe proved by label change ==");
        let read = |label: LabelMatch| async move {
            ui.locate(label.to_query())
                .await
                .ok()
                .flatten()
                .and_then(|element| element.description)
        };
        let comments = labels.label(TikTokControl::Comments);
        let share = labels.label(TikTokControl::Share);
        let snapshot = || async {
            let mut parts = Vec::new();
            if let Some(label) = comments {
                parts.push(read(label).await);
            }
            if let Some(label) = share {
                parts.push(read(label).await);
            }
            parts
        };
        let before = snapshot().await;
        println!("  before: {before:?}");
        let size = ui.window_size().await?;
        ui.swipe(riviu_core::SwipeGesture {
            from: riviu_core::TapPoint {
                x: size.0 * 0.5,
                y: size.1 * 0.72,
            },
            to: riviu_core::TapPoint {
                x: size.0 * 0.5,
                y: size.1 * 0.28,
            },
            duration_ms: 220,
        })
        .await?;
        tokio::time::sleep(Duration::from_millis(900)).await;
        let after = snapshot().await;
        println!("  after:  {after:?}");
        let advanced = after.iter().any(Option::is_some) && after != before;
        println!(
            "  advanced = {advanced}  ({})",
            if advanced {
                "label pair changed — the card moved"
            } else {
                "unchanged — NOT counted as a video"
            }
        );
    } else {
        println!("\n(skipping the swipe proof; pass --feed to move the feed one card)");
    }

    // Measure the liked-state label, which cannot be read off a post that is not
    // liked. Opt-in and self-reverting: it taps like, reads the label the control
    // now carries, then taps again and checks the original label came back, so the
    // account is left as it was found.
    if args.iter().any(|arg| arg == "--measure-liked") {
        println!("\n== measuring the liked-state label ==");
        match measure_liked_label(ui, labels).await {
            Ok(()) => {}
            Err(error) => println!("  FAILED: {error:#}"),
        }
    } else {
        println!("\n(skipping the liked-label measurement; pass --measure-liked — it likes and then unlikes one post)");
    }

    // Measure the comment drawer, which is the last thing standing between the
    // Android nurture loop and comment parity. Nothing is typed and nothing is
    // sent: it opens the drawer, reads what is in it, and presses Back.
    if args.iter().any(|arg| arg == "--measure-comment") {
        println!("\n== measuring the comment drawer ==");
        match measure_comment_drawer(&session, ui, labels).await {
            Ok(()) => {}
            Err(error) => println!("  FAILED: {error:#}"),
        }
    } else {
        println!("\n(skipping the comment-drawer measurement; pass --measure-comment — it opens and closes the drawer, sends nothing)");
    }

    // The share sheet's `Copy link`, read back off the clipboard. The only way to learn a
    // post's URL from the device — the hierarchy never states an id — and a URL the fleet has
    // never opened is what separates "views are not counted" from "these accounts were
    // already counted".
    if args.iter().any(|arg| arg == "--copy-link") {
        println!(
            "
== copying this post's link =="
        );
        match copy_post_link(&session, ui, labels).await {
            Ok(link) => println!("  link = {link}"),
            Err(error) => println!("  FAILED: {error:#}"),
        }
    }

    // Whether an `@handle` can be turned into a real mention rather than plain text.
    // Types `@<prefix>` into the drawer and reports whatever list TikTok puts up; sends
    // nothing and taps nothing but the comment opener.
    if let Some(index) = args.iter().position(|arg| arg == "--measure-mention") {
        let prefix = args
            .get(index + 1)
            .filter(|arg| !arg.starts_with("--"))
            .cloned()
            .unwrap_or_else(|| "ri".to_string());
        println!(
            "
== mention suggestions for @{prefix} =="
        );
        match measure_mention_suggestions(&session, ui, labels, &prefix).await {
            Ok(()) => {}
            Err(error) => println!("  FAILED: {error:#}"),
        }
    } else {
        println!(
            "
(skipping the mention measurement; pass --measure-mention <prefix>)"
        );
    }

    // The seam the Interaction reply path needs: many matches for one label, and a
    // geometric choice among them. Opens the drawer, reads the rows, and runs the
    // real `locate_parent_in_elements` against a body it read off this phone.
    // Sends nothing and taps nothing but the comment opener.
    if args.iter().any(|arg| arg == "--measure-comment-list") {
        println!("\n== comment list rows ==");
        match measure_comment_list(&session, ui, labels).await {
            Ok(()) => {}
            Err(error) => println!("  FAILED: {error:#}"),
        }
    } else {
        println!("\n(skipping the comment-list measurement; pass --measure-comment-list)");
    }

    // The last unknown in the Interaction reply path: what tapping `Trả lời` opens.
    // Four properties decide the design and none is guessable. Sends nothing.
    if args.iter().any(|arg| arg == "--measure-reply") {
        println!("\n== reply composer ==");
        match measure_reply_composer(&session, ui, labels).await {
            Ok(()) => {}
            Err(error) => println!("  FAILED: {error:#}"),
        }
    } else {
        println!("\n(skipping the reply-composer measurement; pass --measure-reply — opens a reply box, sends nothing)");
    }

    // The last unmeasured thing on the Interaction path: what a post page opened from a
    // real link actually looks like. Read-only — it opens the link and reads the tree,
    // taps nothing and sends nothing.
    if let Some(url) = args
        .iter()
        .position(|arg| arg == "--measure-target-open")
        .and_then(|at| args.get(at + 1))
    {
        println!("\n== post page opened from a link ==");
        match measure_target_open(ui, labels, url).await {
            Ok(()) => {}
            Err(error) => println!("  FAILED: {error:#}"),
        }
    } else {
        println!(
            "\n(skipping the link-open measurement; pass --measure-target-open <url> — reads \
             only)"
        );
    }

    // The last promised nurture feature with nothing behind it on Android: swiping a photo
    // post sideways. The pixel engine decides "the page turned" from a *new frame*, and this
    // loop reads no frames at all, so the question is whether the tree carries a per-slide
    // signal. Swipes horizontally on the post it opens; posts nothing.
    if let Some(url) = args
        .iter()
        .position(|arg| arg == "--measure-carousel")
        .and_then(|at| args.get(at + 1))
    {
        println!("\n== photo carousel, swiped sideways ==");
        match measure_carousel(ui, labels, url).await {
            Ok(()) => {}
            Err(error) => println!("  FAILED: {error:#}"),
        }
    } else {
        println!(
            "\n(skipping the carousel measurement; pass --measure-carousel <photo url> — \
             swipes sideways, posts nothing)"
        );
    }

    // The counter was measured on a post page opened from a link. The nurture loop runs on
    // the **feed**, which is a different surface, and a signal that is only on one of them
    // is a feature that never fires. Walks N feed cards and reports what each one carries.
    if let Some(count) = args
        .iter()
        .position(|arg| arg == "--measure-feed-carousel")
        .and_then(|at| args.get(at + 1))
        .and_then(|n| n.parse::<u32>().ok())
    {
        println!("\n== photo cards on the feed, {count} cards ==");
        match measure_feed_carousel(ui, labels, count).await {
            Ok(()) => {}
            Err(error) => println!("  FAILED: {error:#}"),
        }
    } else {
        println!(
            "\n(skipping the feed-carousel measurement; pass --measure-feed-carousel <n> — \
             swipes the feed n times, and on every photo card compares three sideways \
             gestures: plan_swipe, plan_flick, and a plain straight swipe. Posts nothing.)"
        );
    }

    // Gate H4: the whole Standalone Interaction path on one real post, through the
    // **shipped** functions. This POSTS A PUBLIC COMMENT under the logged-in account, so
    // it needs both a link and the text spelled out on the command line.
    if let Some(at) = args.iter().position(|arg| arg == "--gate-standalone") {
        let url = args.get(at + 1).cloned().unwrap_or_default();
        let text = args.get(at + 2).cloned().unwrap_or_default();
        println!(
            "
== gate H4: Standalone send on a real post =="
        );
        if url.is_empty() || text.is_empty() {
            println!("  usage: --gate-standalone <url> <comment text>");
        } else {
            match gate_standalone(ui, labels, size, &url, &text).await {
                Ok(()) => {}
                Err(error) => println!("  FAILED: {error:#}"),
            }
        }
    } else {
        println!(
            "
(skipping gate H4; pass --gate-standalone <url> <text> — POSTS A PUBLIC COMMENT)"
        );
    }

    // Step one of measuring the publish composer, and deliberately **read-only**:
    // the tab bar's own labels, so the control that opens the composer is known
    // before anything taps it. Tapping `+` opens TikTok's camera, which is a bigger
    // step than a measurement needs to take blind.
    if args.iter().any(|arg| arg == "--measure-tab-bar") {
        println!("\n== bottom tab bar (read-only) ==");
        match measure_tab_bar(&session, size).await {
            Ok(()) => {}
            Err(error) => println!("  FAILED: {error:#}"),
        }
    } else {
        println!("\n(skipping the tab-bar measurement; pass --measure-tab-bar — reads only)");
    }

    // Step two: what the composer contains. Opens TikTok's camera, so it is opt-in and
    // backs out without granting anything.
    if let Some(index) = args
        .iter()
        .position(|arg| arg == "--measure-gallery")
        .and_then(|at| args.get(at + 1))
        .and_then(|value| value.parse::<usize>().ok())
    {
        println!(
            "
== gallery entry candidate {index} =="
        );
        match measure_gallery_entry(&session, ui, labels, index).await {
            Ok(()) => {}
            Err(error) => println!("  FAILED: {error:#}"),
        }
    }

    // M4/M5. Read-only, but it needs a post of ours already open on screen — the probe
    // does not navigate there, on purpose: getting there by hand keeps this measurement
    // free of assumptions about a profile grid nobody has measured yet.
    if let Some(caption) = args
        .iter()
        .position(|arg| arg == "--measure-own-post")
        .and_then(|at| args.get(at + 1))
    {
        println!(
            "
== our own post: is the caption readable verbatim? =="
        );
        match measure_own_post_caption(&session, caption).await {
            Ok(()) => {}
            Err(error) => println!("  FAILED: {error:#}"),
        }
    } else {
        println!("
(skipping the own-post caption measurement; open one of your own posts and pass --measure-own-post \"<caption>\" — reads only)");
    }

    if args.iter().any(|arg| arg == "--measure-composer") {
        println!("\n== composer contents ==");
        match measure_composer(&session, ui, labels, size).await {
            Ok(()) => {}
            Err(error) => println!("  FAILED: {error:#}"),
        }
    } else {
        println!("\n(skipping the composer measurement; pass --measure-composer — opens the camera, grants nothing)");
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
        driver.inspect_app_process(&serial, TIKTOK.as_str()).await
    )?;
    println!("    {state:?}");

    // Pha 5: a real JPEG frame source, exercised through `crate::frames` rather
    // than described. Off by default because it pushes an APK to the device.
    if let Ok(apk) = std::env::var("RIVIU_MINICAP_APK") {
        println!("\n== minicap frames ==");
        match measure_frames(&serial, std::path::Path::new(&apk)).await {
            Ok(()) => {}
            Err(error) => println!("  frames FAILED: {error:#}"),
        }
    } else {
        println!("\n(skipping minicap frames; set RIVIU_MINICAP_APK to the noarch minicap.apk)");
    }

    if terminate {
        println!("\n== terminate_app (proved by pidof, not by the command's exit code) ==");
        let proof = timed!(
            "terminate_app",
            driver.terminate_app(&serial, TIKTOK.as_str()).await
        )?;
        println!("    proof = {proof:?}");
        let after = driver.inspect_app_process(&serial, TIKTOK.as_str()).await?;
        anyhow::ensure!(!after.running, "TikTok still running after terminate_app");
        println!("    confirmed gone");
    } else {
        println!("\n(skipping terminate_app; pass --terminate to exercise it)");
    }

    println!("\nG1 probe finished.");
    Ok(())
}

/// Read the `content-desc` the like control carries once a post *is* liked.
///
/// `TikTokControl::Liked` is the state evidence the nurture loop prefers, and it
/// is unmeasurable without liking something — there is no way to derive the
/// translation, which is the whole reason `tiktok_labels` is a measured catalog.
/// So: like, read, unlike, and prove the original label returned. If the unlike
/// does not take, that is reported loudly rather than left for the operator to
/// discover on their account.
async fn measure_liked_label(ui: &dyn UiSession, labels: TikTokControls) -> anyhow::Result<()> {
    let Some(not_liked) = labels.label(TikTokControl::Like) else {
        anyhow::bail!("this build has no measured not-liked label to start from");
    };
    let exact = matches!(not_liked, LabelMatch::Exact(_));
    let before = ui
        .locate(riviu_core::ElementQuery::Description {
            value: not_liked.value(),
            exact,
        })
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "the not-liked label {:?} is not on screen — is this post already liked?",
                not_liked.value()
            )
        })?;
    println!(
        "  starting from {:?} at {:.0},{:.0}",
        not_liked.value(),
        before.x,
        before.y
    );

    let centre = before.centre();
    ui.tap(centre.clone()).await?;
    tokio::time::sleep(Duration::from_millis(1_500)).await;

    // The control keeps its position, so read whatever label now sits there by
    // asking for the element at that spot through the only handle we have: the
    // hierarchy. A substring that both states share is the way in — every TikTok
    // like label so far contains the verb root.
    let root = not_liked.value().chars().take(4).collect::<String>();
    let now = ui
        .locate(riviu_core::ElementQuery::description_contains(&root))
        .await?;
    match now.as_ref().and_then(|element| element.description.as_deref()) {
        Some(label) if label != not_liked.value() => {
            println!("  LIKED label = {label:?}   <-- add this to TIKTOK_LABEL_SETS");
        }
        Some(label) => println!("  label unchanged ({label:?}) — the tap did not register"),
        None => println!(
            "  no element contains {root:?} any more; the liked label does not share the verb root, \
             so dump the tree to read it"
        ),
    }

    // Put it back.
    ui.tap(centre).await?;
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let restored = ui
        .locate(riviu_core::ElementQuery::Description {
            value: not_liked.value(),
            exact,
        })
        .await?;
    if restored.is_some() {
        println!("  unliked again — {:?} is back", not_liked.value());
    } else {
        println!(
            "  WARNING: {:?} did not come back. The post may still be liked on this account; \
             check it by hand",
            not_liked.value()
        );
    }
    Ok(())
}

/// One element from the agent's page source, reduced to what a locator can use.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    class: String,
    desc: String,
    text: String,
    bounds: String,
    clickable: bool,
    enabled: bool,
}

/// Scan the agent's page source.
///
/// The tag name is the **class**, not `node`: `appium-uiautomator2-server` emits
/// `<android.widget.EditText …>` where `adb shell uiautomator dump` emits
/// `<node class="…">`. Splitting on `<node ` therefore matched nothing at all and
/// reported an empty drawer over 157 KB of real hierarchy — which is why this
/// keys on the `class="` attribute every element carries instead of on the tag.
fn scan_source(source: &str) -> Vec<Node> {
    let mut nodes = Vec::new();
    for chunk in source.split('<').skip(1) {
        let Some(end) = chunk.find('>') else { continue };
        let attributes = &chunk[..end];
        let attribute = |name: &str| -> String {
            let needle = format!(" {name}=\"");
            let Some(start) = attributes.find(&needle) else {
                return String::new();
            };
            let rest = &attributes[start + needle.len()..];
            rest.find('"')
                .map(|to| rest[..to].to_string())
                .unwrap_or_default()
        };
        let class = attribute("class");
        if class.is_empty() || class == "hierarchy" {
            continue;
        }
        nodes.push(Node {
            class,
            desc: attribute("content-desc"),
            text: attribute("text"),
            bounds: attribute("bounds"),
            clickable: attribute("clickable") == "true",
            enabled: attribute("enabled") == "true",
        });
    }
    nodes
}

/// Print the elements a locator could target, and save the raw source.
fn report_nodes(stage: &str, source: &str) -> Vec<Node> {
    let dump = std::path::Path::new("target").join(format!("drawer-{stage}.xml"));
    std::fs::write(&dump, source).ok();
    let nodes = scan_source(source);
    println!(
        "  [{stage}] {} bytes, {} elements -> {}",
        source.len(),
        nodes.len(),
        dump.display()
    );
    for node in &nodes {
        if node.class.contains("EditText") {
            println!(
                "    EDITABLE  {} text={:?} desc={:?} {}",
                node.class, node.text, node.desc, node.bounds
            );
        } else if !node.desc.is_empty() {
            println!(
                "    LABEL     desc={:?} class={} clickable={} enabled={} {}",
                node.desc, node.class, node.clickable, node.enabled, node.bounds
            );
        }
    }
    nodes
}

/// Read what the comment drawer is actually made of, in the three states that
/// matter: opened, focused, and holding text.
///
/// The nurture loop needs the input field, the send control, and a signal that the
/// drawer is open. Guessing any of them produces a locator that matches nothing,
/// and a comment flow that silently does nothing is worse than one that refuses.
///
/// **Sends nothing.** It types a probe string because the send control does not
/// exist until there is text to send, then clears the field and backs out. The
/// only way this leaves a comment behind is if TikTok posts on Back, which it does
/// not — and the run reports what it sees at the end either way.
async fn measure_comment_drawer(
    session: &riviu_android_driver::AndroidUiSession,
    ui: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<()> {
    const KEYCODE_BACK: i64 = 4;
    let Some(comments) = labels.label(TikTokControl::Comments) else {
        anyhow::bail!("no measured comment label on this build");
    };
    let element = ui
        .locate(comments.to_query())
        .await?
        .ok_or_else(|| anyhow::anyhow!("the comment control is not on screen"))?;
    println!("  opening the drawer from {:?}", element.description);
    ui.tap(element.centre()).await?;
    tokio::time::sleep(Duration::from_millis(2_500)).await;

    let opened = report_nodes("opened", &session.agent().source().await?);
    let before_labels: std::collections::HashSet<String> =
        opened.iter().map(|node| node.desc.clone()).collect();

    // The input row. Located by class, because its label is a hint string that
    // changes with the placeholder and its `content-desc` is empty.
    let input = session
        .agent()
        .find(&Locator::ClassName("android.widget.EditText".into()))
        .await?
        .ok_or_else(|| anyhow::anyhow!("no EditText in the opened drawer"))?;
    let rect = session.agent().rect(&input).await?;
    let (x, y) = rect.centre();
    println!("  input row at {x:.0},{y:.0} — tapping to focus");
    session.agent().tap(x, y).await?;
    tokio::time::sleep(Duration::from_millis(1_800)).await;
    let _focused = report_nodes("focused", &session.agent().source().await?);

    // Now put text in it, which is what makes a send control appear.
    let probe = "riviu";
    println!("  typing {probe:?} (never sent)");
    let input = session
        .agent()
        .find(&Locator::ClassName("android.widget.EditText".into()).focused())
        .await?
        .ok_or_else(|| anyhow::anyhow!("the EditText did not take focus"))?;
    session.agent().set_text(&input, probe).await?;
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let armed = report_nodes("armed", &session.agent().source().await?);

    // What appeared once there was text is the send control, and naming it that way
    // beats guessing which of the drawer's icons it is.
    let appeared: Vec<&Node> = armed
        .iter()
        .filter(|node| !node.desc.is_empty() && !before_labels.contains(&node.desc))
        .collect();
    if appeared.is_empty() {
        println!("  ! nothing new appeared — the send control is not label-driven on this build");
    } else {
        println!("  labels that appeared only once there was text:");
        for node in appeared {
            println!(
                "    {:?}  class={} clickable={} enabled={} {}   <-- send candidate",
                node.desc, node.class, node.clickable, node.enabled, node.bounds
            );
        }
    }

    // Put everything back: empty the field, then leave.
    session.agent().clear(&input).await.ok();
    tokio::time::sleep(Duration::from_millis(600)).await;
    for _ in 0..3 {
        session.agent().press_key(KEYCODE_BACK).await?;
        tokio::time::sleep(Duration::from_millis(1_000)).await;
        if let Some(feed) = labels.label(TikTokControl::FeedTab) {
            // `feed.to_query()` rather than a hand-built exact-description query: the
            // catalogue decides both the attribute and exact-versus-substring, and
            // rebuilding it here is how a probe ends up testing a locator the product
            // does not use.
            if ui.locate(feed.to_query()).await?.is_some() {
                println!("  backed out — feed tab visible again");
                return Ok(());
            }
        }
    }
    println!("  ! still not back on the feed after three Backs — check the phone");
    Ok(())
}

/// Tap Share, then `Copy link`, then read the clipboard.
///
/// Taps only the share control and the copy row; posts nothing and sends nothing.
async fn copy_post_link(
    session: &riviu_android_driver::AndroidUiSession,
    ui: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<String> {
    let Some(share) = labels.label(TikTokControl::Share) else {
        anyhow::bail!("no measured Share control on this build");
    };
    let element = ui
        .locate(share.to_query())
        .await?
        .ok_or_else(|| anyhow::anyhow!("the share control is not on screen"))?;
    ui.tap(element.centre()).await?;
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let sheet = report_nodes("share-sheet", &session.agent().source().await?);
    // `Copy link` carries its label as text on this build; matched case-insensitively so a
    // translated sheet still has a chance rather than failing on capitalisation.
    let copy = sheet
        .iter()
        .find(|node| {
            let hay = format!("{} {}", node.text, node.desc).to_lowercase();
            hay.contains("copy link") || hay.contains("sao chép liên kết")
        })
        .ok_or_else(|| anyhow::anyhow!("no `Copy link` row in the share sheet"))?;
    let (x1, y1, x2, y2) =
        parse_bounds(&copy.bounds).ok_or_else(|| anyhow::anyhow!("copy row has no bounds"))?;
    session
        .agent()
        .tap((x1 + x2) / 2.0, (y1 + y2) / 2.0)
        .await?;
    tokio::time::sleep(Duration::from_millis(2_000)).await;
    let (_kind, bytes) = ui.get_clipboard(4_096).await?;
    // **Put the sheet away.** Tapping `Copy link` leaves the share sheet up on this build, and
    // the probe's other measurements run in the same process on the same screen — so
    // `--copy-link --measure-mention` measured the share sheet's nodes and called them the
    // comment drawer's. Back, then settle, so the phone is where the next measurement expects.
    ui.back().await.ok();
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}

/// Can an `@handle` be made into a real mention, or only into text that looks like one?
///
/// The interaction feature prepends `@name` as plain characters, and TikTok does not linkify
/// that: the comment renders the literal string and the account is never notified. A real
/// mention is created by typing `@` and **choosing from the suggestion list** the app puts up,
/// which inserts a token. Whether that list is reachable through the accessibility tree is the
/// entire question, and it has to be measured rather than assumed.
///
/// Sends nothing: it types into the drawer, dumps what appeared, clears the field and backs
/// out.
async fn measure_mention_suggestions(
    session: &riviu_android_driver::AndroidUiSession,
    ui: &dyn UiSession,
    labels: TikTokControls,
    prefix: &str,
) -> anyhow::Result<()> {
    let Some(comments) = labels.label(TikTokControl::Comments) else {
        anyhow::bail!("no measured comment label on this build");
    };
    let element = ui
        .locate(comments.to_query())
        .await?
        .ok_or_else(|| anyhow::anyhow!("the comment control is not on screen"))?;
    println!("  opening the drawer from {:?}", element.description);
    ui.tap(element.centre()).await?;
    tokio::time::sleep(Duration::from_millis(2_500)).await;

    report_nodes("mention-opened", &session.agent().source().await?);

    let input = session
        .agent()
        .find(&Locator::ClassName("android.widget.EditText".into()))
        .await?
        .ok_or_else(|| anyhow::anyhow!("no EditText in the opened drawer"))?;
    let rect = session.agent().rect(&input).await?;
    let (x, y) = rect.centre();
    println!("  input row at {x:.0},{y:.0} — tapping to focus");
    session.agent().tap(x, y).await?;
    tokio::time::sleep(Duration::from_millis(1_800)).await;

    // Body first, mention second. `set_text` is the only path that carries Vietnamese, and it
    // replaces the whole field — so anything it writes has to be written *before* a token
    // exists, not after. A mention at the end still notifies the account.
    let body_first = "đi Đà Lạt thật đã";
    println!("  writing the body first: {body_first:?}");
    if let Some(edit) = session
        .agent()
        .find(&Locator::ClassName("android.widget.EditText".into()).focused())
        .await?
    {
        session.agent().set_text(&edit, body_first).await.ok();
        tokio::time::sleep(Duration::from_millis(1_200)).await;
    }

    report_nodes("mention-body", &session.agent().source().await?);

    // No icon tap at all. The `@` itself goes in as a **real key event**, which is the thing
    // the picker listens for — `set_text` writes the same character through accessibility and
    // TikTok never notices it. That also sidesteps the unlabelled icon strip, which moves
    // between three positions depending on the keyboard and whether the field has text.
    println!("  sending \"@{prefix}\" as real key events");
    // **Through the production path.** This used to be the one bare `Command::new("adb")` in
    // the tree, and it cost three things: it bypassed `RIVIU_ADB_PATH` (the app's own adb
    // precedence), so on a machine where adb is not on `PATH` it reported the *measurement* as
    // negative rather than reporting that there was no adb; it bypassed the character whitelist
    // `type_keys` exists to enforce; and it measured a re-implementation, so a pass here proved
    // nothing about what the campaign runner does.
    match session.type_keys(&format!("@{prefix}")).await {
        Ok(()) => println!("    type_keys -> ok"),
        Err(error) => println!("    type_keys -> không gửi được: {error:#}"),
    }
    tokio::time::sleep(Duration::from_millis(3_000)).await;
    let filtered = report_nodes("mention-keyed", &session.agent().source().await?);

    // Rows that look like a handle, inside the picker panel and below the nav tabs. The nav
    // row (`Explore`, `Friends`, …) is ASCII too, which is how a looser filter tapped
    // `Explore` and measured nothing.
    let candidates: Vec<&Node> = filtered
        .iter()
        .filter(|node| node.class.contains("TextView") && !node.text.is_empty())
        .filter(|node| {
            parse_bounds(&node.bounds)
                .is_some_and(|(x1, y1, _, _)| (250.0..900.0).contains(&y1) && x1 < 500.0)
        })
        .collect();
    println!("  rows in the picker after filtering:");
    for node in &candidates {
        println!("    text={:?} {}", node.text, node.bounds);
    }
    let handle_row = candidates.iter().find(|node| {
        node.text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
            && node.text.to_lowercase().contains(&prefix.to_lowercase())
    });
    let Some(row) = handle_row else {
        println!("  ! no row whose handle contains {prefix:?} — the filter did not narrow to it");
        return back_out(session, ui, labels).await;
    };
    let Some((x1, y1, x2, y2)) = parse_bounds(&row.bounds) else {
        return back_out(session, ui, labels).await;
    };
    println!("  tapping the suggestion {:?} at {}", row.text, row.bounds);
    session
        .agent()
        .tap((x1 + x2) / 2.0, (y1 + y2) / 2.0)
        .await?;
    tokio::time::sleep(Duration::from_millis(2_000)).await;
    if let Some(edit) = session
        .agent()
        .find(&Locator::ClassName("android.widget.EditText".into()))
        .await?
    {
        println!(
            "  field after picking: {:?}  <-- a real mention if this is the handle, not just @",
            session.agent().text(&edit).await.unwrap_or_default()
        );

        session.agent().clear(&edit).await.ok();
    }

    back_out(session, ui, labels).await
}

/// Leave the drawer without sending anything.
async fn back_out(
    session: &riviu_android_driver::AndroidUiSession,
    ui: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<()> {
    const KEYCODE_BACK: i64 = 4;
    for _ in 0..4 {
        session.agent().press_key(KEYCODE_BACK).await?;
        tokio::time::sleep(Duration::from_millis(1_000)).await;
        if let Some(feed) = labels.label(TikTokControl::FeedTab) {
            if ui.locate(feed.to_query()).await?.is_some() {
                println!("  backed out — feed tab visible again");
                return Ok(());
            }
        }
    }
    println!("  ! still not back on the feed after four Backs — check the phone");
    Ok(())
}

/// Read the comment rows through `locate_all`, then resolve one by geometry.
///
/// This is the end-to-end check for the Interaction reply path: `locate_all` must
/// return **every** reply control (one per row, not just the first), and
/// `riviu_core::locate_parent_in_elements` must pick the one belonging to a body read
/// off this phone. A wrong pick posts a reply under a stranger's comment, which is
/// invisible in a log — so it is proved here against real geometry.
async fn measure_comment_list(
    session: &riviu_android_driver::AndroidUiSession,
    ui: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<()> {
    const KEYCODE_BACK: i64 = 4;
    let Some(opener) = labels.label(TikTokControl::Comments) else {
        anyhow::bail!("no measured comment label on this build");
    };
    let Some(reply) = labels.label(TikTokControl::CommentReply) else {
        anyhow::bail!("no measured reply label on this build");
    };
    let element = ui
        .locate(opener.to_query())
        .await?
        .ok_or_else(|| anyhow::anyhow!("the comment control is not on screen"))?;
    ui.tap(element.centre()).await?;
    tokio::time::sleep(Duration::from_millis(2_500)).await;

    let started = Instant::now();
    let replies = ui.locate_all(reply.to_query()).await?;
    let reply_ms = started.elapsed().as_millis();
    // Bodies and authors are not label-matched — they are whatever the rows carry —
    // so they come from a class sweep and keep their text.
    let started = Instant::now();
    let text_nodes = ui
        .locate_all(riviu_core::ElementQuery::ClassName(
            "android.widget.TextView",
        ))
        .await?;
    let text_ms = started.elapsed().as_millis();
    let buttons = ui
        .locate_all(riviu_core::ElementQuery::ClassName("android.widget.Button"))
        .await?;
    println!(
        "  locate_all: {} reply control(s) in {reply_ms} ms, {} TextView in {text_ms} ms, {} Button",
        replies.len(),
        text_nodes.len(),
        buttons.len()
    );
    for (index, control) in replies.iter().enumerate() {
        println!(
            "    reply[{index}] {:>4},{:>5} {:>3}x{:<3}",
            control.x, control.y, control.width, control.height
        );
    }

    // `locate_all` skips attribute reads by design, so the text has to be fetched for
    // the rows we intend to reason about. Take the same source of truth the runtime
    // path would: the page source, which is one round trip for all of them.
    let source = session.agent().source().await?;
    let rows: Vec<Node> = scan_source(&source)
        .into_iter()
        .filter(|node| node.class.ends_with("TextView") && !node.text.trim().is_empty())
        .collect();
    println!("  {} TextView rows carry text", rows.len());
    let Some(sample) = rows
        .iter()
        .filter(|node| node.text.chars().count() >= 6 && !node.text.contains("bình luận"))
        .nth(1)
    else {
        println!("  ! not enough comment rows on screen to test the resolver");
        session.agent().press_key(KEYCODE_BACK).await.ok();
        return Ok(());
    };
    println!("  resolving against row text {:?}", sample.text);

    // Rebuild the three candidate lists in `ElementBox` shape, carrying each node's
    // own text in `description` — which is what `locate_parent_in_elements` reads.
    let boxed = |nodes: Vec<&Node>| -> Vec<riviu_core::ElementBox> {
        nodes
            .into_iter()
            .filter_map(|node| {
                let (x, y, right, bottom) = parse_bounds(&node.bounds)?;
                Some(riviu_core::ElementBox {
                    x,
                    y,
                    width: right - x,
                    height: bottom - y,
                    description: Some(node.text.clone()),
                    enabled: node.enabled,
                })
            })
            .collect()
    };
    let all = scan_source(&source);
    let bodies = boxed(
        all.iter()
            .filter(|node| node.class.ends_with("TextView") && !node.text.trim().is_empty())
            .collect(),
    );
    let authors = boxed(
        all.iter()
            .filter(|node| {
                node.class.ends_with("Button") && node.clickable && !node.text.is_empty()
            })
            .collect(),
    );
    let reply_boxes = boxed(
        all.iter()
            .filter(|node| node.text == reply.value())
            .collect(),
    );
    let identity = riviu_core::CommentLocatorIdentity {
        author_label: String::new(),
        text: sample.text.clone(),
        locator_version: "android-hierarchy-v1".into(),
        frame_sha256: "0".repeat(64),
    };
    match riviu_core::locate_parent_in_elements(&bodies, &reply_boxes, &authors, &identity) {
        Some(found) => println!(
            "  RESOLVED author={:?} reply at {:.0},{:.0}",
            found.identity.author_label, found.reply.x, found.reply.y
        ),
        None => println!(
            "  refused — no unambiguous row for that text ({} bodies, {} replies, {} authors)",
            bodies.len(),
            reply_boxes.len(),
            authors.len()
        ),
    }

    session.agent().press_key(KEYCODE_BACK).await.ok();
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    Ok(())
}

/// What tapping a comment's `Trả lời` actually opens.
///
/// Four properties decide how `send_reply` has to be written, and none of them can be
/// inferred:
///
/// 1. is the reply input a **new** `EditText`, and does `.focused(true)` pick the
///    right one? An open drawer already has two, and a composer stacked over the list
///    would be a third — the two-`EditText` trap was the most expensive wrong turn of
///    the earlier probe session;
/// 2. **is `@nickname` pre-filled?** `set_text` replaces the whole value, so if it is,
///    typing would delete the mention — and the mention may be what makes the reply
///    nest. This is the single most consequential unknown here;
/// 3. is Send the same `@2131823284`, and does its `enabled` still flip?
/// 4. what does Back do from the composer — back to the list, or out of the drawer?
///
/// Sends nothing: it types a probe string, reads the tree, clears, and backs out.
async fn measure_reply_composer(
    session: &riviu_android_driver::AndroidUiSession,
    ui: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<()> {
    const KEYCODE_BACK: i64 = 4;
    let Some(opener) = labels.label(TikTokControl::Comments) else {
        anyhow::bail!("no measured comment label on this build");
    };
    let Some(reply) = labels.label(TikTokControl::CommentReply) else {
        anyhow::bail!("no measured reply label on this build");
    };
    let Some(send) = labels.label(TikTokControl::CommentSend) else {
        anyhow::bail!("no measured send label on this build");
    };

    let element = ui
        .locate(opener.to_query())
        .await?
        .ok_or_else(|| anyhow::anyhow!("the comment control is not on screen"))?;
    ui.tap(element.centre()).await?;
    tokio::time::sleep(Duration::from_millis(2_500)).await;

    let edit_texts = |source: &str| -> Vec<Node> {
        scan_source(source)
            .into_iter()
            .filter(|node| node.class.ends_with("EditText"))
            .collect()
    };
    let before = session.agent().source().await?;
    println!("  drawer open: {} EditText", edit_texts(&before).len());

    // Take the first reply control on screen. Which row it belongs to does not matter
    // for a measurement — what is being measured is the composer, not the choice.
    let controls = ui.locate_all(reply.to_query()).await?;
    let Some(target) = controls.first() else {
        println!("  ! no reply control on screen; open a post that has comments");
        session.agent().press_key(KEYCODE_BACK).await.ok();
        return Ok(());
    };
    println!(
        "  tapping reply at {:.0},{:.0} (of {} on screen)",
        target.x,
        target.y,
        controls.len()
    );
    ui.tap(target.centre()).await?;
    tokio::time::sleep(Duration::from_millis(2_000)).await;

    let opened = session.agent().source().await?;
    let fields = edit_texts(&opened);
    println!("  after tapping reply: {} EditText", fields.len());
    for (index, field) in fields.iter().enumerate() {
        println!(
            "    EditText[{index}] text={:?} desc={:?} {}",
            field.text, field.desc, field.bounds
        );
    }
    // Property 2, the consequential one.
    let prefilled: Vec<&Node> = fields
        .iter()
        .filter(|field| field.text.contains('@'))
        .collect();
    if prefilled.is_empty() {
        println!("  no '@' in any field: set_text is safe, no mention to preserve");
    } else {
        for field in prefilled {
            println!(
                "  ! PRE-FILLED MENTION {:?} — set_text would delete it; append instead",
                field.text
            );
        }
    }

    // Property 1: does the focused-narrowed locator find one?
    match session
        .agent()
        .find(&Locator::ClassName("android.widget.EditText".into()).focused())
        .await?
    {
        Some(_) => println!("  focused EditText: found"),
        None => println!("  ! focused EditText: NOT found — type_text would refuse"),
    }

    // Property 3.
    let armed_before = ui.locate(send.to_query()).await?;
    println!(
        "  send control before typing: {:?}",
        armed_before.as_ref().map(|element| element.enabled)
    );
    let probe = "riviu";
    if let Some(field) = session
        .agent()
        .find(&Locator::ClassName("android.widget.EditText".into()).focused())
        .await?
    {
        session.agent().set_text(&field, probe).await?;
        tokio::time::sleep(Duration::from_millis(1_200)).await;
        let armed_after = ui.locate(send.to_query()).await?;
        println!(
            "  send control after typing {probe:?}: {:?}",
            armed_after.as_ref().map(|element| element.enabled)
        );
        match (
            armed_before.map(|e| e.enabled),
            armed_after.map(|e| e.enabled),
        ) {
            (Some(false), Some(true)) => {
                println!("  the same false->true arming proof holds for replies")
            }
            (before, after) => println!(
                "  ! arming differs from the root-comment flow ({before:?} -> {after:?}); \
                 send_reply needs its own proof"
            ),
        }
        session.agent().clear(&field).await.ok();
    }

    // Property 4.
    tokio::time::sleep(Duration::from_millis(600)).await;
    session.agent().press_key(KEYCODE_BACK).await?;
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let after_back = session.agent().source().await?;
    let still_drawer = !edit_texts(&after_back).is_empty();
    let on_feed = match labels.label(TikTokControl::FeedTab) {
        Some(feed) => ui.locate(feed.to_query()).await?.is_some(),
        None => false,
    };
    println!(
        "  after Back: drawer still open = {still_drawer}, feed tab visible = {on_feed}  \
         ({} EditText)",
        edit_texts(&after_back).len()
    );

    // Leave the phone on the feed whatever Back did.
    for _ in 0..3 {
        if let Some(feed) = labels.label(TikTokControl::FeedTab) {
            if ui.locate(feed.to_query()).await?.is_some() {
                break;
            }
        }
        session.agent().press_key(KEYCODE_BACK).await.ok();
        tokio::time::sleep(Duration::from_millis(900)).await;
    }
    Ok(())
}

/// Read the bottom tab bar's labels.
///
/// Read-only on purpose. The publish path starts by tapping the composer opener, and
/// nothing in the catalog names it yet — so the first measurement is *what it is
/// called*, not what happens when it is pressed. Tapping `+` opens TikTok's camera,
/// Gate H4: arrive at a real post and post one root comment, through the shipped code.
///
/// Every step is a shipped function — `open_target_by_hierarchy` then
/// `send_root_by_hierarchy` — because the point of a gate is to exercise what ships, not a
/// transcription of it. The drawer is left open afterwards, exactly as the Interaction path
/// requires so its evidence frame shows the comment in the list.
///
/// **This posts publicly.** It is opt-in on the command line and takes the text from the
/// operator rather than inventing one.
async fn gate_standalone(
    ui: &dyn UiSession,
    labels: TikTokControls,
    screen: (f64, f64),
    url: &str,
    text: &str,
) -> anyhow::Result<()> {
    use riviu_core::interaction_hierarchy::{
        open_target_by_hierarchy, send_root_by_hierarchy, TargetArrival,
    };
    let handle = url
        .split('@')
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default()
        .to_string();
    let stop = std::sync::atomic::AtomicBool::new(false);

    let started = Instant::now();
    let arrival = open_target_by_hierarchy(ui, labels, TIKTOK.as_str(), url, &handle, &stop).await;
    match &arrival {
        Ok(TargetArrival::Identified { author_label }) => println!(
            "  arrival: Identified ({author_label}) in {} ms",
            started.elapsed().as_millis()
        ),
        Ok(TargetArrival::Structural) => println!(
            "  arrival: Structural in {} ms — the post changed but the nickname does not \
             reveal the handle",
            started.elapsed().as_millis()
        ),
        Err(refusal) => {
            println!(
                "  arrival REFUSED: {} — {}",
                refusal.code(),
                refusal.message()
            );
            println!("  nothing was typed. Gate H4 did not run.");
            return Ok(());
        }
    }

    println!("  sending {text:?} …");
    let sent = Instant::now();
    let outcome = send_root_by_hierarchy(ui, labels, screen, text, &[], &stop, String::new).await?;
    println!(
        "  verdict = {:?} ({}) in {} ms",
        outcome.verdict,
        outcome.verdict.reason(),
        sent.elapsed().as_millis()
    );
    match &outcome.identity {
        Some(identity) => println!(
            "  read back from the open list: author={:?} text={:?} locator={}",
            identity.author_label, identity.text, identity.locator_version
        ),
        None => println!(
            "  ! the posted comment could not be read back unambiguously — a Threaded \
             chain would stop here rather than reply to a row nobody confirmed"
        ),
    }
    if outcome.verdict.is_sent() && outcome.identity.is_some() {
        println!("  GATE H4 PASSED: sent, disarm confirmed, and read back by text.");
    } else {
        println!("  GATE H4 INCOMPLETE — see the verdict above.");
    }
    Ok(())
}

/// Does a photo card **on the feed** carry the page counter the traversal gates on?
///
/// Asked separately because the counter was measured on a post page opened from a link, and
/// the nurture loop never sees that surface — it runs on the For-You feed. A signal present
/// on one and absent on the other is a feature that silently never fires, which is exactly
/// what a 25-card run showed: not one carousel line, on a feed full of photo posts.
///
/// Reads each card, then swipes the feed vertically. Nothing is tapped.
async fn measure_feed_carousel(
    ui: &dyn UiSession,
    labels: TikTokControls,
    cards: u32,
) -> anyhow::Result<()> {
    let (w, h) = ui.window_size().await?;
    let mut with_counter = 0u32;
    // Turns offered to each gesture, and turns that actually paged. Counted only where
    // the answer is knowable: the previous reading has to be known, and the post has to
    // have images left. This is the regression check for the whole finding — a future
    // change to the planner that quietly turns a flick back into a drag shows up here.
    let mut offered = [0u32; 3];
    let mut paged = [0u32; 3];
    let mut with_photo_word = 0u32;
    for card in 1..=cards {
        let counter = ui
            .locate(riviu_core::driver::ElementQuery::Text {
                value: " / ",
                exact: true,
            })
            .await
            .ok()
            .flatten();
        let look = CarouselLook::read(ui, labels).await?;
        let hints = look.slide_hints();
        if counter.is_some() {
            with_counter += 1;
        }
        if !hints.is_empty() {
            with_photo_word += 1;
        }
        println!(
            "  thẻ {card:>2}: đếm \" / \"={}  nhãn={:?}  {} TextView  Comments={}",
            if counter.is_some() { "CÓ" } else { "không" },
            hints,
            look.texts.len(),
            // The one that decides whether the nurture loop ever *reaches* a photo card:
            // it treats a card with no comment control as a LIVE card and swipes straight
            // past, so a photo post without this label can never be paged.
            match &look.comments {
                Some(_) => "CÓ",
                None => "KHÔNG — vòng lặp sẽ coi là thẻ LIVE và bỏ qua",
            }
        );
        if let Some(node) = counter {
            // Where the indicator sits, and which TextViews are near it. The feed and a
            // link-opened post page render this differently — on the post page the digits
            // are TextViews either side of the slash, on the feed the slash node is found by
            // `locate` yet does not appear in the TextView list at all — so the neighbours
            // are the only way to know what the shipped parse can actually see.
            println!(
                "    node \" / \": x={:.0} y={:.0} w={:.0} h={:.0} desc={:?}",
                node.x, node.y, node.width, node.height, node.description
            );
            // And the question the implementation actually turns on: does a sideways swipe
            // change anything the loop can read, *here on the feed*? Two gestures, on the
            // same card, in this order — because the **shape** of the swipe is the one
            // variable no earlier measurement ever changed. Every mode above swipes a
            // dead-straight line; the shipped loop swipes `plan_swipe`'s path, which is
            // bowed, eased, and held still for a moment before the lift.
            //
            // The bow is perpendicular to travel, and travel here is horizontal — so the
            // bow is **vertical**, up to ~4.5% of the path, plus endpoint jitter. A
            // link-opened post page has nothing competing for that axis; the feed has its
            // own vertical pager. If the planned gesture turns nothing and the straight one
            // turns pages on the same card, that is the bug — and it is also why the card
            // afterwards has no action rail: the feed is left stranded between two posts.
            //
            // `parse` is what `carousel_position` would return, read the way it reads it. A
            // gesture that moves pixels but leaves that `None` is as broken as one that
            // moves nothing, so both are printed rather than one standing in for the other.
            // Four gestures on the same card. The first is what the engine actually sends;
            // the last is what every measurement before this one sent. The two in between
            // take the engine's own path and remove **one component each**, so the answer
            // names a component rather than a vibe.
            //
            // Order is deliberate and costs nothing: a gesture that fails to turn the page
            // consumes no image, so the suspects go first and the known-good straight swipe
            // goes last, where running out of post no longer confounds anything.
            // Three gestures on the same card: what the engine used to send, what it sends
            // now, and the plain straight swipe every earlier measurement used as its only
            // gesture. Both planned variants call the **shipped** functions rather than
            // lookalikes — the point is to measure `plan_flick`, not something shaped like it.
            //
            // A gesture that fails to turn the page consumes no image, so the control goes
            // first and the reference last, where running out of post confounds nothing.
            let mut planner = TouchPointPlanner::new((w, h));
            let mut before = CarouselLook::read(ui, labels).await?;
            let mut turn = 0u32;
            // Seeded from the screen, not from zero: the counter is already up here, so
            // the very first turn is judgeable too.
            let mut previous = shipped_counter(ui).await;
            for (variant, (style, repeats)) in [
                ("plan_swipe — cử chỉ cũ", 3u32),
                ("plan_flick — cử chỉ mới", 4),
                ("thẳng một đoạn", 3),
            ]
            .into_iter()
            .enumerate()
            {
                for _ in 0..repeats {
                    turn += 1;
                    let from = riviu_core::types::TapPoint {
                        x: w * 0.78,
                        y: h * 0.40,
                    };
                    let to = riviu_core::types::TapPoint {
                        x: w * 0.22,
                        y: h * 0.40,
                    };
                    let settle_ms;
                    match variant {
                        0 => {
                            let path = planner.plan_swipe(from, to, 320);
                            settle_ms = path.settle_ms;
                            ui.swipe_path(path).await?;
                        }
                        1 => {
                            let path = planner.plan_flick(from, to, 320);
                            settle_ms = path.settle_ms;
                            ui.swipe_path(path).await?;
                        }
                        _ => {
                            settle_ms = 0;
                            ui.swipe(riviu_core::types::SwipeGesture {
                                from,
                                to,
                                duration_ms: 320,
                            })
                            .await?;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(900)).await;
                    let parsed = shipped_counter(ui).await;
                    // A turn only counts when the answer is knowable. If the previous
                    // reading was lost, or the post had no image left to turn to, the turn
                    // says nothing about the gesture and is not held against it.
                    if let (Some((was, total)), Some((now, _))) = (previous, parsed) {
                        if was < total {
                            offered[variant] += 1;
                            if now > was {
                                paged[variant] += 1;
                            }
                        }
                    }
                    previous = parsed;
                    let now = CarouselLook::read(ui, labels).await?;
                    println!(
                        "      ngang {turn:>2} [{style}]: parse={}  giữ={settle_ms}ms  \
                         Comments đổi={}",
                        match parsed {
                            Some((current, total)) => format!("{current}/{total}"),
                            None => "None".to_string(),
                        },
                        now.comments != before.comments
                    );
                    if now.comments.is_none() {
                        println!(
                            "        ! RAIL MẤT — đã rời khỏi bài, đúng triệu chứng phiên nuôi chết"
                        );
                    }
                    before = now;
                }
            }
        }
        ui.swipe(riviu_core::types::SwipeGesture {
            from: riviu_core::types::TapPoint {
                x: w * 0.5,
                y: h * 0.72,
            },
            to: riviu_core::types::TapPoint {
                x: w * 0.5,
                y: h * 0.28,
            },
            duration_ms: 280,
        })
        .await?;
        tokio::time::sleep(Duration::from_millis(1_400)).await;
    }
    println!(
        "\n  KẾT LUẬN: {with_counter}/{cards} thẻ có bộ đếm, {with_photo_word}/{cards} thẻ có \
         nhãn ảnh. {}",
        if with_counter == 0 && with_photo_word > 0 {
            "Feed CÓ bài ảnh nhưng KHÔNG có bộ đếm — gate hiện tại không bao giờ nổ trên feed"
        } else if with_counter > 0 {
            "Bộ đếm có trên feed — gate dùng được"
        } else {
            "Không gặp bài ảnh nào trong lần chạy này — chưa kết luận được"
        }
    );
    if offered.iter().any(|count| *count > 0) {
        println!("\n  Lượt lật được / lượt được trao, theo cử chỉ:");
        for (variant, style) in [
            "plan_swipe (cũ)",
            "plan_flick (đang dùng)",
            "thẳng một đoạn",
        ]
        .into_iter()
        .enumerate()
        {
            println!("    {style:<24} {}/{}", paged[variant], offered[variant]);
        }
        // The claim this mode exists to keep honest. `plan_flick` was measured at 19 of 19
        // against `plan_swipe`'s 13 of 40; a run where it drops back toward the old number
        // means the planner has quietly started sending a drag again.
        println!(
            "    -> plan_flick phải bám sát cột 'thẳng', không bám 'cũ'. Xem \
             TouchPointPlanner::plan_flick."
        );
    }
    Ok(())
}

/// `(current, total)` exactly as `carousel_position` would read it on this screen.
///
/// A copy of the shipped three-node parse rather than a call to it, because that function
/// is private to the nurture module — and the copy is faithful in the one way that matters
/// here: it reads `locate_all_described` **unfiltered**, empty descriptions and all, which
/// is what the engine does and is not what `CarouselLook` does. If an empty node ever lands
/// between the digits and the slash, the engine sees `None` where the eye sees `2 / 10`.
async fn shipped_counter(ui: &dyn UiSession) -> Option<(u32, u32)> {
    let texts: Vec<String> = ui
        .locate_all_described(riviu_core::driver::ElementQuery::ClassName(
            "android.widget.TextView",
        ))
        .await
        .ok()?
        .into_iter()
        .filter_map(|element| element.description)
        .collect();
    texts.windows(3).find_map(|window| {
        if window[1].trim() != "/" {
            return None;
        }
        let current = window[0].trim().parse::<u32>().ok()?;
        let total = window[2].trim().parse::<u32>().ok()?;
        (current >= 1 && total >= current).then_some((current, total))
    })
}

/// What one look at a photo post sees, in the only terms the hierarchy loop has.
///
/// Deliberately not a screenshot and not `/source`: the shipped loop can only ask
/// `locate`/`locate_all`, so a signal this snapshot cannot see is a signal the loop cannot
/// use, however visible it is on the phone.
struct CarouselLook {
    /// Every image rectangle, rounded. The obvious candidate for "the page turned".
    images: Vec<(i64, i64, i64, i64)>,
    /// Rendered text on the card. A page indicator, if one exists as text, is here.
    texts: Vec<String>,
    /// The comment control's label. Post-level, so it must **not** change between slides —
    /// it is the control for "did we stay on the same post".
    comments: Option<String>,
}

impl CarouselLook {
    async fn read(ui: &dyn UiSession, labels: TikTokControls) -> anyhow::Result<Self> {
        let images = ui
            .locate_all(riviu_core::driver::ElementQuery::ClassName(
                "android.widget.ImageView",
            ))
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|b| {
                (
                    b.x.round() as i64,
                    b.y.round() as i64,
                    b.width.round() as i64,
                    b.height.round() as i64,
                )
            })
            .collect();
        let texts = ui
            .locate_all_described(riviu_core::driver::ElementQuery::ClassName(
                "android.widget.TextView",
            ))
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|b| b.description)
            .filter(|t| !t.trim().is_empty())
            .collect();
        let comments = match labels.label(TikTokControl::Comments) {
            Some(label) => ui
                .locate(label.to_query())
                .await
                .ok()
                .flatten()
                .and_then(|f| f.description),
            None => None,
        };
        Ok(Self {
            images,
            texts,
            comments,
        })
    }

    /// Anything that names a photo post or a position inside one.
    fn slide_hints(&self) -> Vec<&str> {
        self.texts
            .iter()
            .map(String::as_str)
            .filter(|t| {
                let lower = t.to_lowercase();
                lower.contains("ảnh")
                    || lower.contains("photo")
                    || lower.contains("image")
                    // `3/8`, `3 / 8` — a page counter in any spelling.
                    || (t.contains('/')
                        && t.chars().any(|c| c.is_ascii_digit())
                        && t.chars().count() <= 8)
            })
            .collect()
    }
}

/// Is there a per-slide signal in the accessibility tree, and does "swipe until nothing
/// changes" work in element space the way it works in frame space?
///
/// **Only give this a `/photo/` link.** A horizontal swipe on a *video* post is TikTok's
/// go-to-the-author's-profile gesture, so running this on the wrong kind of post measures
/// navigation rather than a carousel. That asymmetry is itself the finding that matters
/// most for the implementation: the traversal must know the card is a photo post *before*
/// it swipes sideways, or it walks off the feed.
async fn measure_carousel(
    ui: &dyn UiSession,
    labels: TikTokControls,
    url: &str,
) -> anyhow::Result<()> {
    println!("  url = {url}");
    if !url.contains("/photo/") {
        println!("  ! this is not a /photo/ link — a sideways swipe on a video opens the profile");
    }
    ui.open_url_in_app(url, TIKTOK.as_str()).await?;

    let Some(comments_label) = labels.label(TikTokControl::Comments) else {
        anyhow::bail!("Comments is unmeasured on this build, so arrival cannot be proved");
    };
    let opened = Instant::now();
    let mut arrived = false;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(600)).await;
        if ui
            .locate(comments_label.to_query())
            .await
            .ok()
            .flatten()
            .is_some()
        {
            arrived = true;
            break;
        }
    }
    if !arrived {
        anyhow::bail!(
            "no action rail {} ms after opening the link — never reached the post",
            opened.elapsed().as_millis()
        );
    }
    println!("  arrived after {} ms", opened.elapsed().as_millis());

    let first = CarouselLook::read(ui, labels).await?;
    println!(
        "  slide 0: {} ImageView, {} TextView, Comments={:?}",
        first.images.len(),
        first.texts.len(),
        first.comments.as_deref().unwrap_or("<none>")
    );
    let hints = first.slide_hints();
    println!(
        "  nhãn có thể chỉ ảnh / vị trí: {}",
        if hints.is_empty() {
            "KHÔNG CÓ".to_string()
        } else {
            format!("{hints:?}")
        }
    );

    // Sideways, through the image area: clear of the right-hand action rail and well above
    // the caption block, so the gesture lands on the pager and not on a control.
    let (w, h) = ui.window_size().await?;
    let mut previous = first;
    let mut changed_by = Vec::new();
    for slide in 1..=10u32 {
        ui.swipe(riviu_core::types::SwipeGesture {
            from: riviu_core::types::TapPoint {
                x: w * 0.78,
                y: h * 0.40,
            },
            to: riviu_core::types::TapPoint {
                x: w * 0.22,
                y: h * 0.40,
            },
            duration_ms: 320,
        })
        .await?;
        tokio::time::sleep(Duration::from_millis(900)).await;

        let now = CarouselLook::read(ui, labels).await?;
        // Left the post entirely? On a video that is the profile page; on a photo post a
        // vertical mis-swipe moves to the next post. Either way the measurement is over.
        if now.comments.is_none() {
            println!("  swipe {slide}: RAIL GONE — no longer on a post page, stopping");
            break;
        }
        let images_moved = now.images != previous.images;
        let texts_moved = now.texts != previous.texts;
        let post_changed = now.comments != previous.comments;
        println!(
            "  swipe {slide}: ImageView đổi={images_moved}  TextView đổi={texts_moved}  \
             Comments đổi={post_changed} ({} ImageView)",
            now.images.len()
        );
        if post_changed {
            println!(
                "    ! Comments đổi {:?} -> {:?} — đã sang BÀI KHÁC, không phải ảnh khác",
                previous.comments.as_deref().unwrap_or("<none>"),
                now.comments.as_deref().unwrap_or("<none>")
            );
            break;
        }
        let hints = now.slide_hints();
        if !hints.is_empty() {
            println!("    nhãn vị trí: {hints:?}");
        }
        if texts_moved {
            // **Which** string moved is the whole measurement. "some text changed" would
            // have me implement a fingerprint over the wrong nodes — a scrolling music
            // ticker changes on every read and would make the end of a carousel
            // undetectable, while a page counter is exactly the signal wanted.
            for text in &now.texts {
                if !previous.texts.contains(text) {
                    println!("    + {text:?}");
                }
            }
            for text in &previous.texts {
                if !now.texts.contains(text) {
                    println!("    - {text:?}");
                }
            }
        }
        if images_moved || texts_moved {
            changed_by.push(slide);
        } else {
            println!("    KHÔNG ĐỔI GÌ — đây là tín hiệu 'hết ảnh' mà vòng lặp cần");
            break;
        }
        previous = now;
    }
    println!(
        "\n  KẾT LUẬN: {} cú vuốt làm cây đổi ({changed_by:?}). {}",
        changed_by.len(),
        if changed_by.is_empty() {
            "KHÔNG có tín hiệu nào trong cây — traversal phải dùng frame".to_string()
        } else {
            "Xem các dòng +/- ở trên để biết CHỮ NÀO đổi; đó mới là tín hiệu, \
             không phải 'có gì đó đổi'"
                .to_string()
        }
    );
    Ok(())
}

/// Open a real link and report exactly what `open_target_by_hierarchy` relies on.
///
/// This is the measurement that decides whether the arrival check is sound, and it is
/// deliberately **read-only**: it opens the link and reads the tree at intervals. No tap,
/// no text, nothing posted — an arrival check that taps could dismiss a sheet or open a
/// profile, so the measurement must not either.
///
/// The four questions, in the order they matter:
///
/// 1. How long until `active_app_bundle()` reports TikTok.
/// 2. Is `Comments` present — the predicate that says "a post page is up".
/// 3. **Is `FeedTab` absent?** This is the discriminator the pixel path does not have. If
///    `Đề xuất` is on a link-opened post page, then "Comments present and FeedTab absent"
///    is *not* a valid arrival test and the code has to change.
/// 4. Does any node carry the author handle, or only the nickname.
async fn measure_target_open(
    ui: &dyn UiSession,
    labels: TikTokControls,
    url: &str,
) -> anyhow::Result<()> {
    let handle = url
        .split('@')
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default()
        .to_string();
    println!("  url    = {url}");
    println!("  handle = {handle:?} (from the link)");

    // **The pre-state matters, and the product guarantees it.**
    // `start_interaction_session` foregrounds TikTok and *proves* it with
    // `active_app_bundle` before anything opens a link, so by the time the arrival check
    // runs the app is warm and on the feed. Measured the other way round — force-stop,
    // then open the link — the deep link is **dropped** and TikTok lands on the For-You
    // feed instead. That is a real failure mode worth knowing, but it is not the sequence
    // the product performs, so this waits for the feed first.
    match labels.label(TikTokControl::FeedTab) {
        Some(feed) => {
            let mut ready = false;
            for _ in 0..25 {
                if ui.locate(feed.to_query()).await.ok().flatten().is_some() {
                    ready = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(600)).await;
            }
            println!("  pre-state: on the feed = {ready} (what the product guarantees)");
            if !ready {
                println!("  ! not on the feed — this is not the product's sequence");
            }
        }
        None => println!("  pre-state: cannot check, FeedTab unmeasured on this build"),
    }

    // The post on screen *before* the link is opened. The shipped check reads this too:
    // it is the only signal that separates an arrival from an intent that did nothing.
    let before_author = match labels.label(TikTokControl::Follow) {
        Some(label) => ui
            .locate(label.to_query())
            .await
            .ok()
            .flatten()
            .and_then(|found| found.description)
            .unwrap_or_default(),
        None => String::new(),
    };
    println!("  before: author on screen = {before_author:?}");

    let opened = Instant::now();
    // Pinned to the package, like the shipped path: a bare VIEW intent resolves to the
    // system app chooser because Chrome claims tiktok.com too.
    ui.open_url_in_app(url, TIKTOK.as_str()).await?;
    // Polled at the rate the product polls, and reported as the elapsed time the rail
    // first appeared — that number is what `ARRIVAL_WINDOW` has to cover, and a coarse
    // sample only gives a range.
    let mut rail_at: Option<u128> = None;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(600)).await;
        let foreground = ui.active_app_bundle().await.unwrap_or_default();
        let comments = match labels.label(TikTokControl::Comments) {
            Some(label) => ui.locate(label.to_query()).await.ok().flatten(),
            None => None,
        };
        let elapsed = opened.elapsed().as_millis();
        if let Some(found) = comments {
            rail_at = Some(elapsed);
            println!(
                "  [{elapsed:>6} ms] foreground={foreground}  Comments={:?}",
                found.description.as_deref().unwrap_or("<no desc>")
            );
            break;
        }
        println!("  [{elapsed:>6} ms] foreground={foreground}  no rail yet");
    }
    match rail_at {
        Some(ms) => println!(
            "
  RAIL APPEARED AFTER {ms} ms — ARRIVAL_WINDOW must exceed this"
        ),
        None => println!(
            "
  the rail never appeared within 24 s"
        ),
    }
    // Everything else on the page, once it has settled.
    let mut present = Vec::new();
    for control in [
        TikTokControl::Comments,
        TikTokControl::FeedTab,
        TikTokControl::Share,
        TikTokControl::Like,
        TikTokControl::Bookmark,
        TikTokControl::Follow,
    ] {
        let Some(label) = labels.label(control) else {
            continue;
        };
        if let Ok(Some(found)) = ui.locate(label.to_query()).await {
            present.push(format!(
                "{control:?}={:?}",
                found.description.as_deref().unwrap_or("<no desc>")
            ));
        }
    }
    println!("  on the page: {}", present.join(", "));

    // The verdict the shipped check actually rests on, spelled out. **Not** feed-tab
    // absence: measured, a deep-linked post is the current card of the For-You pager, so
    // the feed tab stays visible and requiring its absence refused every real arrival.
    // What is left is: a rail is up, and the post is not the one that was already there.
    let on_post = match labels.label(TikTokControl::Comments) {
        Some(label) => ui.locate(label.to_query()).await.ok().flatten().is_some(),
        None => false,
    };
    let on_feed = match labels.label(TikTokControl::FeedTab) {
        Some(label) => ui.locate(label.to_query()).await.ok().flatten().is_some(),
        None => false,
    };
    let after = match labels.label(TikTokControl::Follow) {
        Some(label) => ui
            .locate(label.to_query())
            .await
            .ok()
            .flatten()
            .and_then(|found| found.description)
            .unwrap_or_default(),
        None => String::new(),
    };
    println!(
        "
  author before = {before_author:?}"
    );
    println!("  author after  = {after:?}");
    println!("  rail present  = {on_post}, feed tab present = {on_feed} (informational only)");
    if !on_post {
        println!("  => NO RAIL: nothing is up. Refusal: NoPostPage.");
    } else if after.is_empty() || after == before_author {
        println!(
            "  => UNCHANGED: same post as before the link. This is the signature of an \
             unavailable post (deleted / private / region-blocked) — TikTok takes the \
             intent, fails server-side, and leaves the feed alone. Refusal: \
             ScreenNeverChanged. THE LINK IS THE PROBLEM, NOT THE PHONE."
        );
    } else {
        println!("  => ARRIVED: a different post is up.");
        if !handle.is_empty() {
            // The **shipped** predicate, not a lookalike: a probe that reimplements the
            // rule tests its own copy and can pass while the product fails.
            let hit = riviu_core::interaction_hierarchy::author_matches_handle(&after, &handle);
            println!(
                "     handle {handle:?} vs author {after:?} -> {}",
                if hit {
                    "Identified (the nickname reveals the handle)"
                } else {
                    "Structural (the nickname does not reveal the handle)"
                }
            );
        }
    }
    Ok(())
}

/// which is a bigger step than a measurement should take on a guess.
///
/// Prints every labelled node in the bottom band with its geometry, so the opener can
/// be identified by name and then added to `tiktok_labels` with provenance.
/// M4/M5 — can our own caption be read back off our own post page, verbatim?
///
/// **The gate the whole delete design rests on.** The operator's rule is that a delete tap
/// may only go out after the code has read the campaign's exact caption on the post that is
/// open. If TikTok truncates the caption below roughly two dozen characters, that rule is
/// not implementable and the honest answer is to refuse automatic deletion and keep it
/// manual — so this measurement decides a design question, not a parameter.
///
/// Read-only. It dumps the tree and reports; it taps nothing.
async fn measure_own_post_caption(
    session: &riviu_android_driver::AndroidUiSession,
    caption: &str,
) -> anyhow::Result<()> {
    let source = session.agent().source().await?;
    let dump = std::path::Path::new("target").join("own-post.xml");
    std::fs::write(&dump, &source).ok();
    println!(
        "    tree dumped to {} ({} bytes)",
        dump.display(),
        source.len()
    );

    let wanted: String = caption.trim().to_string();
    println!(
        "    campaign caption: {} chars, {:?}",
        wanted.chars().count(),
        wanted
    );

    // Verbatim first. If this is present the rule is implementable as written.
    if source.contains(&wanted) {
        println!("    VERBATIM: the full caption is on this screen — the rule is implementable");
    } else {
        println!("    NOT VERBATIM — measuring how much of it survives");
        // Longest prefix that appears, by characters rather than bytes: the caption is
        // Vietnamese and a byte-wise walk would split a code point.
        let chars: Vec<char> = wanted.chars().collect();
        let mut longest = 0usize;
        for take in 1..=chars.len() {
            let prefix: String = chars[..take].iter().collect();
            if source.contains(&prefix) {
                longest = take;
            } else {
                break;
            }
        }
        println!(
            "    longest prefix present: {longest} of {} chars",
            chars.len()
        );
        if longest == 0 {
            println!("    VERDICT: nothing of the caption is readable — automatic delete CANNOT be proved");
        } else if longest < 24 {
            println!("    VERDICT: prefix shorter than 24 chars — too weak to identify a post; keep deletion manual");
        } else {
            println!("    VERDICT: prefix is long enough to identify a post, record captionProof=\"prefix\"");
        }
    }

    // A `Follow ` control here would mean this is somebody else's post, which is the one
    // decisive refusal in the proof chain.
    println!(
        "    Follow control on this page: {}",
        if source.contains("Follow ") {
            "PRESENT — not our post"
        } else {
            "absent"
        }
    );
    Ok(())
}

async fn measure_tab_bar(
    session: &riviu_android_driver::AndroidUiSession,
    screen: (f64, f64),
) -> anyhow::Result<()> {
    let source = session.agent().source().await?;
    let dump = std::path::Path::new("target").join("tab-bar.xml");
    std::fs::write(&dump, &source).ok();
    println!("  wrote {}", dump.display());

    // The tab bar sits in the bottom band. Anything above it is feed chrome.
    let band = screen.1 * 0.86;
    let mut found = 0usize;
    for node in scan_source(&source) {
        let Some((x, y, right, bottom)) = parse_bounds(&node.bounds) else {
            continue;
        };
        if y < band {
            continue;
        }
        if node.desc.trim().is_empty() && node.text.trim().is_empty() {
            continue;
        }
        found += 1;
        println!(
            "  {:>4},{:>5} {:>4}x{:<4} clickable={:<5} class={:<28} desc={:?} text={:?}",
            x,
            y,
            right - x,
            bottom - y,
            node.clickable,
            node.class.rsplit('.').next().unwrap_or(&node.class),
            node.desc,
            node.text
        );
    }
    if found == 0 {
        println!(
            "  ! nothing labelled below y={band:.0}; the tab bar may be hidden on this screen"
        );
    }
    println!(
        "  (the composer opener is the middle one horizontally, around x={:.0})",
        screen.0 / 2.0
    );
    Ok(())
}

/// What TikTok's composer contains, so the publish path has labels to aim at.
///
/// Opens the camera, which is why this is opt-in. It **grants nothing**: if a runtime
/// permission dialog appears it is reported and backed out of, never accepted —
/// granting camera access on somebody's phone is not a measurement's decision.
///
/// Dumps twice, because the camera screen settles in stages and a single read catches
/// a half-built view.
async fn measure_composer(
    session: &riviu_android_driver::AndroidUiSession,
    ui: &dyn UiSession,
    labels: TikTokControls,
    screen: (f64, f64),
) -> anyhow::Result<()> {
    const KEYCODE_BACK: i64 = 4;
    let Some(opener) = labels.label(TikTokControl::ComposerOpen) else {
        anyhow::bail!("no measured composer opener on this build");
    };
    let element = ui
        .locate(opener.to_query())
        .await?
        .ok_or_else(|| anyhow::anyhow!("the composer opener is not on screen"))?;
    println!(
        "  opening via {:?} at {:.0},{:.0}",
        opener.value(),
        element.x,
        element.y
    );
    ui.tap(element.centre()).await?;

    let mut permission_seen = false;
    for (stage, wait) in [("2s", 2_000u64), ("5s", 3_000)] {
        tokio::time::sleep(Duration::from_millis(wait)).await;
        let source = match session.agent().source().await {
            Ok(source) => source,
            Err(error) => {
                println!("  [{stage}] source unavailable: {error}");
                continue;
            }
        };
        let dump = std::path::Path::new("target").join(format!("composer-{stage}.xml"));
        std::fs::write(&dump, &source).ok();
        let nodes = scan_source(&source);
        // A runtime permission dialog is an Android system window, not TikTok.
        let permission = nodes.iter().any(|node| {
            node.text.contains("cho phép")
                || node.text.contains("Cho phép")
                || node.desc.contains("permission")
        });
        if permission {
            permission_seen = true;
        }
        println!(
            "  [{stage}] {} elements -> {}{}",
            nodes.len(),
            dump.display(),
            if permission {
                "   ! a permission dialog is on screen; nothing will be granted"
            } else {
                ""
            }
        );
        // Only what a locator could target, and only in the lower two thirds where the
        // gallery entry and the shutter row live — the top is filter chrome.
        for node in &nodes {
            let Some((x, y, right, bottom)) = parse_bounds(&node.bounds) else {
                continue;
            };
            let labelled = !node.desc.trim().is_empty() || !node.text.trim().is_empty();
            if !labelled || y < screen.1 * 0.55 {
                continue;
            }
            println!(
                "      {:>4},{:>5} {:>4}x{:<4} click={:<5} {:<22} desc={:?} text={:?}",
                x,
                y,
                right - x,
                bottom - y,
                node.clickable,
                node.class.rsplit('.').next().unwrap_or(&node.class),
                node.desc,
                node.text
            );
        }
    }

    if permission_seen {
        println!("  backing out of the permission dialog without granting it");
    }
    // Back out until the feed tab is visible again. The camera can take a couple of
    // presses, and a permission dialog adds one.
    let feed = labels.label(TikTokControl::FeedTab);
    for attempt in 1..=5 {
        if let Some(feed) = feed {
            if ui.locate(feed.to_query()).await?.is_some() {
                println!("  back on the feed after {} Back press(es)", attempt - 1);
                return Ok(());
            }
        }
        session.agent().press_key(KEYCODE_BACK).await.ok();
        tokio::time::sleep(Duration::from_millis(1_200)).await;
    }
    println!("  ! still not on the feed after five Backs — check the phone");
    Ok(())
}

/// Find out which unlabelled control beside the shutter opens the gallery.
///
/// The composer's gallery entry carries **no `content-desc` and no `text`** — measured.
/// Three clickable 204x204 `View`s sit in a row to the right of the shutter and nothing
/// in the tree says which is which, so the only way to know is to press one and look.
/// That is what a probe is for; the runtime path must never do it.
///
/// Anchored on the shutter, which *is* labelled (`@2131823324`), so the candidates are
/// described relative to a known control rather than by absolute coordinates.
async fn measure_gallery_entry(
    session: &riviu_android_driver::AndroidUiSession,
    ui: &dyn UiSession,
    labels: TikTokControls,
    which: usize,
) -> anyhow::Result<()> {
    const KEYCODE_BACK: i64 = 4;
    let Some(opener) = labels.label(TikTokControl::ComposerOpen) else {
        anyhow::bail!("no measured composer opener on this build");
    };
    let element = ui
        .locate(opener.to_query())
        .await?
        .ok_or_else(|| anyhow::anyhow!("the composer opener is not on screen"))?;
    ui.tap(element.centre()).await?;
    tokio::time::sleep(Duration::from_millis(4_000)).await;

    let source = session.agent().source().await?;
    let nodes = scan_source(&source);
    // The shutter is the anchor: a clickable Button with a resource-id label, and the
    // widest one on screen. Widest rather than "the one at these coordinates", so the
    // anchor survives a different screen size.
    let shutter = nodes
        .iter()
        .filter(|node| {
            node.class.ends_with("Button") && node.clickable && node.desc.starts_with('@')
        })
        .filter_map(|node| parse_bounds(&node.bounds))
        .max_by(|a, b| (a.2 - a.0).total_cmp(&(b.2 - b.0)))
        .ok_or_else(|| anyhow::anyhow!("no labelled shutter to anchor on"))?;
    let (sx, sy, sright, sbottom) = shutter;
    println!("  shutter anchor at {sx:.0},{sy:.0}..{sright:.0},{sbottom:.0}");

    // Clickable, unlabelled, to the right of the shutter, on the shutter's row.
    let mut candidates: Vec<(f64, f64, f64, f64)> = nodes
        .iter()
        .filter(|node| node.clickable && node.desc.trim().is_empty() && node.text.trim().is_empty())
        .filter_map(|node| parse_bounds(&node.bounds))
        // Filter on the *centre* being right of the shutter, not the left edge. The
        // control immediately beside the shutter overlaps it by ~50 px, so a left-edge
        // test silently drops the nearest — and the nearest is the interesting one.
        .filter(|(x, y, right, bottom)| (x + right) / 2.0 >= sright && *bottom > sy && *y < sbottom)
        .collect();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    println!(
        "  {} unlabelled candidate(s) right of the shutter:",
        candidates.len()
    );
    for (index, (x, y, right, bottom)) in candidates.iter().enumerate() {
        println!(
            "    [{index}] {x:.0},{y:.0} {:.0}x{:.0}",
            right - x,
            bottom - y
        );
    }
    let Some(&(x, y, right, bottom)) = candidates.get(which) else {
        println!("  ! no candidate at index {which}; nothing tapped");
        for _ in 0..4 {
            session.agent().press_key(KEYCODE_BACK).await.ok();
            tokio::time::sleep(Duration::from_millis(1_000)).await;
        }
        return Ok(());
    };
    println!(
        "  tapping candidate [{which}] at {:.0},{:.0}",
        (x + right) / 2.0,
        (y + bottom) / 2.0
    );
    ui.tap(riviu_core::TapPoint {
        x: (x + right) / 2.0,
        y: (y + bottom) / 2.0,
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(3_500)).await;

    let after = session.agent().source().await?;
    std::fs::write(
        std::path::Path::new("target").join(format!("gallery-candidate-{which}.xml")),
        &after,
    )
    .ok();
    let opened = scan_source(&after);
    // A photo grid is a lot of same-sized image cells. Nothing else in this app looks
    // like that, so counting them is a usable signature.
    let images = opened
        .iter()
        .filter(|node| node.class.ends_with("ImageView"))
        .count();
    println!(
        "  after the tap: {} elements, {images} ImageView",
        opened.len()
    );
    for node in opened
        .iter()
        .filter(|node| !node.desc.trim().is_empty() || !node.text.trim().is_empty())
        .take(24)
    {
        println!(
            "    {:<22} desc={:?} text={:?}",
            node.class.rsplit('.').next().unwrap_or(&node.class),
            node.desc,
            node.text
        );
    }

    let feed = labels.label(TikTokControl::FeedTab);
    for attempt in 1..=6 {
        if let Some(feed) = feed {
            if ui.locate(feed.to_query()).await?.is_some() {
                println!("  back on the feed after {} Back press(es)", attempt - 1);
                return Ok(());
            }
        }
        session.agent().press_key(KEYCODE_BACK).await.ok();
        tokio::time::sleep(Duration::from_millis(1_200)).await;
    }
    println!("  ! still not on the feed — check the phone");
    Ok(())
}

/// `[x,y][right,bottom]` as four numbers.
fn parse_bounds(bounds: &str) -> Option<(f64, f64, f64, f64)> {
    let mut numbers = bounds
        .split(|c: char| !c.is_ascii_digit() && c != '-')
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<f64>().ok());
    Some((
        numbers.next()?,
        numbers.next()?,
        numbers.next()?,
        numbers.next()?,
    ))
}

/// Drive `crate::frames` against the device: push, launch, forward, read.
///
/// The FPS this reports is bounded by whatever is on screen — minicap publishes on
/// display change, so a still screen legitimately yields nothing. Run it with a
/// video playing if the number is meant to mean anything.
async fn measure_frames(serial: &str, apk: &std::path::Path) -> anyhow::Result<()> {
    use riviu_android_driver::frames::{self, MinicapOptions, MinicapStream, Projection};

    let adb = riviu_android_driver::AdbProgram::resolve(None, None)?;
    let size = frames::device_screen(&adb, serial).await?;
    let options = MinicapOptions::for_device(serial, Projection::half(size.0, size.1));
    println!(
        "  projection {}  Q{}",
        options.projection.to_arg(),
        options.quality
    );

    timed!("ensure_apk", frames::ensure_apk(&adb, serial, apk).await)?;

    // `am instrument`-style: leave it running and reap it at the end.
    let mut child = tokio::process::Command::new(adb.path())
        .args(["-s", serial, "shell", &frames::launch_command(&options)])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    tokio::time::sleep(Duration::from_secs(4)).await;

    let port = timed!(
        "forward tcp:0 + read back",
        frames::forward(&adb, serial, &options.socket).await
    )?;
    println!("    adb assigned local port {port}");

    let mut stream = timed!("connect + banner", MinicapStream::connect(port).await)?;
    let banner = stream.banner().clone();
    println!(
        "    banner real={}x{} virtual={}x{} orient={} quirks={}",
        banner.real_width,
        banner.real_height,
        banner.virtual_width,
        banner.virtual_height,
        banner.orientation,
        banner.quirks
    );

    let started = Instant::now();
    let deadline = started + Duration::from_secs(6);
    let mut count = 0usize;
    let mut bytes = 0usize;
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, stream.next_frame()).await {
            Ok(Ok(frame)) => {
                count += 1;
                bytes += frame.len();
            }
            // The screen simply stopped changing; that is not a failure.
            Err(_) => break,
            Ok(Err(error)) => {
                println!("    frame read stopped: {error:#}");
                break;
            }
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    if count == 0 {
        println!("    0 frames in {seconds:.2}s — nothing on screen changed");
    } else {
        println!(
            "    {count} frames in {seconds:.2}s = {:.1} FPS, avg {:.1} KB/frame",
            count as f64 / seconds,
            bytes as f64 / count as f64 / 1024.0
        );
    }

    frames::remove_forward(&adb, serial, port).await.ok();
    let _ = child.kill().await;
    Ok(())
}
