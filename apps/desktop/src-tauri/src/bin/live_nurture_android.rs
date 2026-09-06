//! Run the nurture engine on several real Android phones at once, and report what each did.
//!
//! ```text
//! RIVIU_ADB_PATH=… cargo run -p riviu-managers-phone --bin live_nurture_android -- \
//!   --only SERIAL_A,SERIAL_B --devices 2 --minutes 2 --videos 3
//! ```
//!
//! **Do not run this while the desktop app is open.** Two processes competing for the same
//! phones is the contention this project spent a week removing.
//!
//! All public actions default off. Comments require `--comment-prob`, and switching them on posts
//! real text under the logged-in account. That is the operator's call and this harness now
//! takes it as an instruction rather than refusing it outright — the earlier version could
//! not be asked at all, which meant the feature could only ever be exercised by hand.
//!
//! The AI key is **never typed on the command line**. Settings are inherited from the
//! desktop app's own database — model, base url, language, tone directions and word cap — by
//! explicitly opting in with `--inherit-settings 1` and copying that file into the scratch directory, so a run here can
//! never rewrite what the operator configured in the app. Secret writes stay in memory.
//!
//! **The key itself is not in that file, and this paragraph used to claim it was.** It moved
//! to the OS credential store, so a faithful copy of the operator's database reports
//! `khoá API TRỐNG` and the run refuses to comment — which is exactly what happened on
//! 25/08/2026 to a harness written from this instruction. The copy is opened *with* the same
//! keyring seam `AppState::bootstrap` uses, so the key is read from where the app put it and
//! never lands in a file. Following the old wording got an empty key and no explanation.
//!
//! What it exercises is everything underneath — an exclusive lease, a
//! stream-budget slot, a UI session, TikTok in the foreground, watch and swipe and like —
//! per device, concurrently. That is the part that decides whether the feature works for a
//! whole fleet or only for the first couple of phones.
//!
//! It exists because the answer turned out to be "the first couple": every session holds a
//! foreground slot for its whole run, and the desktop's budget was two.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app_lib::interaction_ocr::DesktopFrameTextSource;
use riviu_core::db::{Database, SecretStore};
use riviu_core::driver::DeviceDriver;
use riviu_core::{
    DeviceControlPlane, DeviceWorkCoordinator, NurtureEngine, NurtureSettings, StreamBudgetManager,
};
use riviu_ios_driver::StreamHub;
use uuid::Uuid;

/// The same seam `AppState::bootstrap` uses, so the key comes from where the app put it.
///
/// Copying the database is not enough: the AI key stopped living in the settings blob when it
/// moved to the credential store, and a copy therefore carries every setting except the one
/// that decides whether a comment can be written at all.
struct KeyringSecrets {
    credentials: Option<riviu_signing::CredentialStore>,
    overrides: parking_lot::Mutex<HashMap<String, String>>,
}

impl SecretStore for KeyringSecrets {
    fn get_secret(&self, name: &str) -> anyhow::Result<Option<String>> {
        if let Some(value) = self.overrides.lock().get(name) {
            return Ok(Some(value.clone()));
        }
        self.credentials
            .as_ref()
            .map(|store| store.app_secret(name))
            .transpose()
            .map(Option::flatten)
    }
    fn set_secret(&self, name: &str, value: &str) -> anyhow::Result<()> {
        self.overrides
            .lock()
            .insert(name.to_owned(), value.to_owned());
        Ok(())
    }
}

/// Print and flush. Output is piped to a file here, where Rust block-buffers stdout — a run
/// that is killed part way would otherwise report nothing at all about what it had learned.
fn say(line: &str) {
    println!("{line}");
    let _ = std::io::stdout().flush();
}

#[derive(Debug)]
struct Args {
    devices: usize,
    /// Restrict the run to named serials. Without it the harness takes the first N the
    /// driver reports, which is what "the whole fleet" means.
    only: Vec<String>,
    minutes: u64,
    videos: u32,
    /// Percent chance of liking each watched post.
    ///
    /// Configurable because the like path is the one that needs a *sample* to say anything:
    /// at the default rate a two-video run produces one or two attempts across the whole
    /// fleet, which cannot distinguish "confirmation works" from "we got lucky". Raising it
    /// is the only way to measure the confirmation, and a like is an action this feature
    /// exists to perform — unlike a comment, which is why that one stays at zero.
    like_prob: u32,
    /// Percent chance of following the author of a watched post.
    ///
    /// Off by default and separate from `--like-prob` because the two are not the same kind
    /// of action: a like can be taken back and leaves nothing behind, a follow is a lasting
    /// relationship on a real account. Configurable at all only because the path cannot be
    /// verified without performing one — and then it is worth doing on one phone rather than
    /// on twenty.
    follow_prob: u32,
    /// Percent chance of writing and posting a comment on each watched post.
    ///
    /// **This posts real text under the logged-in account.** Zero by default, and the only
    /// way to raise it is to say so here — there is no stored value that can switch it on
    /// behind the operator's back, because the inherited settings have this one field
    /// overwritten unconditionally.
    ///
    /// The text is not canned: it is generated per post from what is on the screen, in the
    /// language and tone the desktop app has stored, and a second model call scores it for
    /// relevance and genericity before it is typed. What can still go wrong is on the
    /// account, not in the code — so start at a low number on one phone and read what it
    /// wrote before raising it.
    comment_prob: u32,
    save_prob: u32,
    inherit_settings: bool,
    stop_after_seconds: u64,
}

fn parse_args(raw: &[String]) -> anyhow::Result<Args> {
    let mut args = Args {
        devices: 2,
        only: Vec::new(),
        minutes: 2,
        videos: 3,
        like_prob: 0,
        follow_prob: 0,
        comment_prob: 0,
        save_prob: 0,
        inherit_settings: false,
        stop_after_seconds: 0,
    };
    anyhow::ensure!(
        raw.len().is_multiple_of(2),
        "every argument requires a value"
    );
    let mut index = 0;
    while index + 1 < raw.len() {
        if raw[index] == "--only" {
            args.only = raw[index + 1].split(',').map(str::to_string).collect();
            anyhow::ensure!(
                args.only.iter().all(|serial| !serial.is_empty()),
                "--only requires serials"
            );
            index += 2;
            continue;
        }
        let value = raw[index + 1].parse::<u32>()?;
        if raw[index].ends_with("-prob") {
            anyhow::ensure!(value <= 100, "probabilities must be 0..100");
        }
        match raw[index].as_str() {
            "--devices" => args.devices = value as usize,
            "--minutes" => args.minutes = u64::from(value),
            "--videos" => args.videos = value,
            "--like-prob" => args.like_prob = value,
            "--follow-prob" => args.follow_prob = value,
            "--comment-prob" => args.comment_prob = value,
            "--save-prob" => args.save_prob = value,
            "--inherit-settings" => {
                anyhow::ensure!(value <= 1, "--inherit-settings must be 0 or 1");
                args.inherit_settings = value == 1;
            }
            "--stop-after-seconds" => args.stop_after_seconds = u64::from(value),
            flag => anyhow::bail!("unknown argument: {flag}"),
        }
        index += 2;
    }
    anyhow::ensure!((1..=20).contains(&args.devices), "--devices must be 1..20");
    anyhow::ensure!((1..=60).contains(&args.minutes), "--minutes must be 1..60");
    anyhow::ensure!((1..=500).contains(&args.videos), "--videos must be 1..500");
    anyhow::ensure!(
        !args.only.is_empty(),
        "--only requires an explicit target snapshot"
    );
    Ok(args)
}

fn run_settings(stored: NurtureSettings, args: &Args) -> NurtureSettings {
    NurtureSettings {
        num_videos: args.videos,
        num_rounds: 1,
        like_enabled: args.like_prob > 0,
        like_prob: args.like_prob,
        comment_enabled: args.comment_prob > 0,
        comment_prob: args.comment_prob,
        save_enabled: args.save_prob > 0,
        save_prob: args.save_prob,
        follow_enabled: args.follow_prob > 0,
        follow_prob: args.follow_prob,
        frenzy_prob: 0,
        schedule_enabled: false,
        watch_min: 2.0,
        watch_max: 4.0,
        stagger_delay_min: 1,
        stagger_delay_max: 3,
        ..stored
    }
}

fn session_passes(status: &riviu_core::NurtureSessionStatus, stopped: bool) -> bool {
    !status.running
        && status.phase.is_terminal()
        && (status.outcome == Some(riviu_core::Outcome::Done)
            || (stopped && status.outcome == Some(riviu_core::Outcome::Stopped)))
        && (status.videos_done > 0 || stopped)
        && status.cleanup_state == riviu_core::NurtureCleanupState::ProcessAbsent
        && status.cleanup_proof.is_some()
        && status.cleanup_error.is_none()
}

fn session_stop_flags(count: usize) -> Vec<Arc<AtomicBool>> {
    (0..count)
        .map(|_| Arc::new(AtomicBool::new(false)))
        .collect()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args(&std::env::args().skip(1).collect::<Vec<_>>())?;

    let android_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sidecars/android");
    let noarch = android_root.join("noarch");
    let android = Arc::new(riviu_android_driver::AndroidDriver::new(
        &riviu_android_driver::AndroidDriverConfig {
            bundled_adb_path: Some(android_root.join("win-x86_64/adb.exe")),
            bundled_minicap_apk: Some(noarch.join("minicap.apk")),
            bundled_scrcpy_server: Some(noarch.join("scrcpy-server")),
            bundled_riviu_agent_apk: Some(noarch.join("riviu-agent.apk")),
            bundled_agent_server_apk: Some(noarch.join("appium-uiautomator2-server.apk")),
            bundled_agent_test_apk: Some(noarch.join("appium-uiautomator2-server-test.apk")),
            ..Default::default()
        },
    )?);
    let streams = Arc::new(StreamHub::new());
    android.set_frame_sink(Arc::new(streams.as_ref().clone()));

    let devices = android.list_devices().await?;
    let targets: Vec<String> = devices
        .iter()
        .filter(|device| device.status != riviu_core::DeviceStatus::Disconnected)
        .filter(|device| args.only.contains(&device.udid))
        .take(args.devices)
        .map(|device| device.udid.clone())
        .collect();
    anyhow::ensure!(!targets.is_empty(), "no usable Android device is connected");
    anyhow::ensure!(
        args.only.len() == targets.len(),
        "every requested serial must be connected and within --devices"
    );

    // The number under test. Every nurture session holds one for its whole run, so this is
    // the ceiling on how many phones can nurture at once — whatever the fleet size is.
    let budget = StreamBudgetManager::new(
        std::env::var("RIVIU_STREAM_CAPACITY")
            .ok()
            .and_then(|raw| raw.trim().parse::<usize>().ok())
            .unwrap_or(targets.len()),
    );
    let budget = budget?;

    let scratch = std::env::temp_dir().join(format!("riviu-live-nurture-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&scratch)?;
    // **The desktop app's own database, copied.** The AI key, the model, the base url, the
    // comment language, the tone directions and the word cap all live there and nowhere
    // else, so a harness that starts from `Default` cannot write a comment at all — it has
    // no key. Copying rather than opening the real file means a run here can never rewrite
    // what the operator set in the app, and cannot be blamed for a settings change either.
    //
    // Inheritance is explicit; a missing/unreadable source must not change the requested profile.
    let app_db = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("riviu-managers-phone")
        .join("riviu.db");
    let inherited = if args.inherit_settings {
        anyhow::ensure!(
            app_db.is_file(),
            "requested settings database does not exist"
        );
        let wal = PathBuf::from(format!("{}-wal", app_db.display()));
        anyhow::ensure!(
            !wal.exists() || wal.metadata()?.len() == 0,
            "close the app and checkpoint its database before inheriting settings"
        );
        std::fs::copy(&app_db, scratch.join("riviu.db"))?;
        true
    } else {
        false
    };
    let database = Arc::new(
        Database::open(scratch.join("riviu.db"))?.with_secrets(Arc::new(KeyringSecrets {
            credentials: if inherited {
                Some(riviu_signing::CredentialStore::system()?)
            } else {
                None
            },
            overrides: parking_lot::Mutex::new(HashMap::new()),
        })),
    );
    let control = Arc::new(DeviceControlPlane::new(
        android.clone(),
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(budget),
    ));
    let engine = NurtureEngine::new(
        database.clone(),
        control.clone(),
        Arc::new(streams.as_ref().clone()),
        scratch.join("artifacts"),
    )
    .with_frame_text_source(Arc::new(DesktopFrameTextSource));

    // Start from what the app has stored rather than from `Default`, or the key, the model
    // and the tone directions are all empty and a comment cannot be written at all. Only the
    // fields this harness is *asked* about are overwritten.
    //
    // `comment_prob` is overwritten **unconditionally**, including with zero. The stored
    // value must never be able to switch commenting on for a run that did not ask for it —
    // and the app's stored value is a number the operator set for the app, not for this.
    let stored = database.get_nurture_settings()?;
    let has_key = !stored.api_key.trim().is_empty();
    println!(
        "cấu hình: {} — model {}, ngôn ngữ {}, tối đa {} từ, định hướng {:?}, khoá API {}",
        if inherited {
            "kế thừa từ app"
        } else {
            "fixture riêng, không đọc cấu hình app"
        },
        stored.model,
        stored.comment_lang,
        stored.max_comment_words,
        stored.ai_directions,
        if has_key { "có" } else { "TRỐNG" }
    );
    if args.comment_prob > 0 && !has_key {
        anyhow::bail!(
            "--comment-prob {} nhưng không có khoá API trong cấu hình của app. Điền khoá \
             trong menu Nuôi TikTok rồi chạy lại — ở đây cố tình không nhận khoá qua dòng \
             lệnh, vì một khoá gõ trên dòng lệnh sẽ nằm lại trong lịch sử shell",
            args.comment_prob
        );
    }
    if args.comment_prob > 0 {
        println!(
            "** BÌNH LUẬN ĐANG BẬT ({}%) — sẽ đăng chữ thật lên tài khoản thật **",
            args.comment_prob
        );
    }

    let settings = run_settings(stored, &args);
    say(&format!(
        "public rates: like={} save={} comment={} follow={}; evidence={}",
        settings.like_prob,
        settings.save_prob,
        settings.comment_prob,
        settings.follow_prob,
        scratch.display()
    ));

    // **Write them down before running, or they are not the settings that run.**
    //
    // `run_session` re-reads the stored settings row once per post — that is the desktop's
    // live-tuning mechanism, where "Lưu" in the panel *is* how a running session is
    // retuned — and what it reads overwrites what was passed in. A fresh database answers
    // with `NurtureSettings::default()`, whose `follow_prob` is 3.
    //
    // Measured, not reasoned: on 18/08/2026 ce0717171c2a64d50d followed an author during a
    // run whose settings said `follow_prob: 0`. A follow is a real relationship on a real
    // account, and the same channel governs `comment_prob` — so a run asked for zero
    // comments could otherwise post them from a stored value it never saw. Persisting the
    // merged settings first is what makes `--comment-prob` mean what it says.
    database.save_nurture_settings(&settings)?;

    println!(
        "nurturing {} device(s) for up to {} minute(s), {} video(s) each\n",
        targets.len(),
        args.minutes,
        args.videos
    );

    let started = Instant::now();
    let mut running = Vec::new();
    let shared_stop = Arc::new(AtomicBool::new(false));
    let stop_signal = shared_stop.clone();
    let session_stops = session_stop_flags(targets.len());
    let stop_signals = session_stops.clone();
    let stop_after = args.stop_after_seconds;
    let signal_task = tokio::spawn(async move {
        if stop_after > 0 {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = tokio::time::sleep(Duration::from_secs(stop_after)) => {},
            }
        } else {
            let _ = tokio::signal::ctrl_c().await;
        }
        stop_signal.store(true, std::sync::atomic::Ordering::Relaxed);
        for stop in stop_signals {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    for (index, udid) in targets.iter().cloned().enumerate() {
        let engine = engine.clone();
        let settings = settings.clone();
        let stop = session_stops[index].clone();
        running.push(tokio::spawn(async move {
            // Staggered the way the desktop staggers, so twenty phones do not all reach for
            // a lease in the same millisecond.
            tokio::time::sleep(Duration::from_millis(index as u64 * 700)).await;
            let began = Instant::now();
            say(&format!("  [{udid}] session starting"));
            // Keep the trail. A session that ends "0/2 video" has already said *why* in
            // its intermediate status lines, and throwing them away leaves only the number.
            let trail = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
            let collector = Arc::clone(&trail);
            let outcome = engine
                .run_session(
                    &udid,
                    settings,
                    stop,
                    Some(Duration::from_secs(args.minutes * 60)),
                    move |status| {
                        let mut seen = collector.lock().expect("status trail");
                        if seen.last().map(String::as_str) != Some(status.last_message.as_str()) {
                            seen.push(status.last_message.clone());
                        }
                    },
                )
                .await;
            let trail = trail.lock().expect("status trail").clone();
            match &outcome {
                Ok(status) => {
                    say(&format!(
                        "  [{udid}] done in {:.0}s: videos={} likes={} — {}",
                        began.elapsed().as_secs_f64(),
                        status.videos_done,
                        status.likes,
                        status.last_message
                    ));
                    if status.videos_done == 0 {
                        for line in &trail {
                            say(&format!("      [{udid}] {line}"));
                        }
                    } else {
                        // The carousel is the thing under test, and a session that met a
                        // photo post and *finished* says so nowhere else: the trail is only
                        // dumped when a session ends at zero. That is exactly backwards for
                        // proving a traversal no longer eats the session, so the photo lines
                        // come out either way.
                        for line in trail.iter().filter(|line| line.contains("bài ảnh")) {
                            say(&format!("      [{udid}] {line}"));
                        }
                    }
                }
                Err(error) => say(&format!(
                    "  [{udid}] FAILED after {:.0}s: {error:#}",
                    began.elapsed().as_secs_f64()
                )),
            }
            (udid, began.elapsed(), outcome)
        }));
    }

    let mut worked = 0;
    let mut watched_any = 0;
    let mut passed = 0;
    let mut results = Vec::new();
    say(&format!(
        "{:<22} {:>7} {:>7} {:>7}  outcome",
        "device", "secs", "videos", "likes"
    ));
    for handle in running {
        let (udid, elapsed, outcome) = handle.await?;
        match outcome {
            Ok(status) => {
                if session_passes(
                    &status,
                    shared_stop.load(std::sync::atomic::Ordering::Relaxed),
                ) {
                    passed += 1;
                }
                say(&format!("status={}", serde_json::to_string(&status)?));
                results.push(serde_json::json!({ "udid": udid, "status": status }));
                if status.videos_done > 0 {
                    watched_any += 1;
                }
                worked += 1;
                println!(
                    "{udid:<22} {:>7.0} {:>7} {:>7}  {}",
                    elapsed.as_secs_f64(),
                    status.videos_done,
                    status.likes,
                    status.last_message
                );
            }
            Err(error) => {
                results.push(serde_json::json!({"udid":udid,"error":format!("{error:#}")}));
                println!(
                    "{udid:<22} {:>7.0} {:>7} {:>7}  ERROR: {error:#}",
                    elapsed.as_secs_f64(),
                    0,
                    0
                );
            }
        }
    }
    println!(
        "\n{worked}/{} session(s) returned, {watched_any} watched at least one video, in {:.0}s",
        targets.len(),
        started.elapsed().as_secs_f64()
    );

    // Every comment the run *considered*, not just the ones that posted. A skip is the more
    // interesting row: it says the evidence was unusable or the verifier rejected the draft,
    // and both are working as intended rather than failures to chase.
    match database.list_nurture_comment_attempts(200) {
        Ok(attempts) if !attempts.is_empty() => {
            println!("\nbình luận — {} lượt:", attempts.len());
            let (mut prompt_tokens, mut completion_tokens) = (0u64, 0u64);
            for attempt in &attempts {
                // Tokens over every attempt, sent or rejected: a comment the gate threw
                // away still burned API calls, and the USD this used to sum was two
                // hand-typed prices multiplied by exactly these counts.
                prompt_tokens += u64::from(attempt.prompt_tokens);
                completion_tokens += u64::from(attempt.completion_tokens);
                let scores = match (attempt.relevance, attempt.evidence_support) {
                    (Some(relevance), Some(evidence)) => {
                        // The frame count belongs next to the evidence score, not somewhere
                        // else: `bằng chứng 40` on one frame and on three are different
                        // findings, and until this column existed they printed identically.
                        let frames = match attempt.distinct_frames {
                            Some(0) => ", không ảnh".to_string(),
                            Some(1) => ", 1 khung (bài tĩnh)".to_string(),
                            Some(n) => format!(", {n} khung"),
                            None => String::new(),
                        };
                        // The slide count only next to the frame count: seven slides and one
                        // frame says the pager turned and the stream never repainted.
                        let slides = match attempt.carousel_slides {
                            Some(0) | None => String::new(),
                            Some(n) => format!(", lướt {n} ảnh"),
                        };
                        format!("  [hợp đề {relevance}, bằng chứng {evidence}{frames}{slides}]")
                    }
                    _ => String::new(),
                };
                println!(
                    "  {:<22} {:<22} {}{}",
                    attempt.udid,
                    attempt.outcome,
                    if attempt.preview.is_empty() {
                        format!("(caption: {})", attempt.caption_preview)
                    } else {
                        format!("{:?}", attempt.preview)
                    },
                    scores
                );
            }
            println!("  token: {prompt_tokens} vào / {completion_tokens} ra");
        }
        Ok(_) if args.comment_prob > 0 => {
            println!("\nbình luận: bật {}% nhưng không lượt nào được thử — xác suất chưa nổ, hoặc phiên kết thúc trước đó", args.comment_prob);
        }
        Ok(_) => {}
        Err(error) => println!("\nkhông đọc được các lượt bình luận: {error}"),
    }

    signal_task.abort();
    let cleanup = control.shutdown_cleanup().await;
    let report = serde_json::json!({ "targets": targets, "passed": passed, "results": results,
        "controlCleanup": cleanup.as_ref().err().map(ToString::to_string),
        "reservedStreamCapacity": control.reserved_stream_capacity(),
        "cleanupQuarantineCount": control.cleanup_quarantine_count(),
        "publicRates": {"like":args.like_prob,"save":args.save_prob,"comment":args.comment_prob,"follow":args.follow_prob} });
    std::fs::write(
        scratch.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    say(&format!("report={}", scratch.join("report.json").display()));
    cleanup?;
    anyhow::ensure!(
        passed == targets.len(),
        "only {passed}/{} sessions passed; inspect report",
        targets.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finishing_one_session_does_not_stop_another() {
        let stops = session_stop_flags(3);
        stops[0].store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(!stops[1].load(std::sync::atomic::Ordering::Relaxed));
        assert!(!stops[2].load(std::sync::atomic::Ordering::Relaxed));
    }

    fn args(extra: &[&str]) -> Args {
        let mut raw = vec!["--only".to_string(), "fixture".to_string()];
        raw.extend(extra.iter().map(|value| value.to_string()));
        parse_args(&raw).unwrap()
    }

    #[test]
    fn inherited_public_actions_are_off_until_explicitly_requested() {
        let stored = NurtureSettings {
            like_prob: 100,
            save_prob: 100,
            comment_prob: 100,
            follow_prob: 100,
            save_enabled: true,
            ..Default::default()
        };
        let configured = run_settings(stored, &args(&[])).into_effective();
        assert_eq!(
            (
                configured.like_prob,
                configured.save_prob,
                configured.comment_prob,
                configured.follow_prob
            ),
            (0, 0, 0, 0)
        );
        assert!(!args(&[]).inherit_settings);
    }

    #[test]
    fn explicit_rates_enable_all_four_independent_switches() {
        let stored = NurtureSettings {
            like_enabled: false,
            comment_enabled: false,
            follow_enabled: false,
            save_enabled: false,
            ..Default::default()
        };
        let configured = run_settings(
            stored,
            &args(&[
                "--like-prob",
                "100",
                "--save-prob",
                "75",
                "--comment-prob",
                "50",
                "--follow-prob",
                "25",
            ]),
        )
        .into_effective();
        assert_eq!(
            (
                configured.like_prob,
                configured.save_prob,
                configured.comment_prob,
                configured.follow_prob
            ),
            (100, 75, 50, 25)
        );
    }

    #[test]
    fn invalid_flags_and_unbounded_runs_are_rejected() {
        for raw in [
            vec!["--only"],
            vec!["--typo", "1"],
            vec!["--like-prob", "101"],
            vec!["--minutes", "0"],
            vec!["--devices", "21"],
            vec!["--videos", "0"],
            vec![],
        ] {
            assert!(parse_args(
                &raw.iter()
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
            )
            .is_err());
        }
    }

    #[test]
    fn scratch_secret_write_never_needs_the_system_store() {
        let store = KeyringSecrets {
            credentials: None,
            overrides: Default::default(),
        };
        store.set_secret("fixture", "temporary").unwrap();
        assert_eq!(
            store.get_secret("fixture").unwrap().as_deref(),
            Some("temporary")
        );
    }

    #[test]
    fn zero_work_or_missing_process_proof_is_not_success() {
        let mut status = riviu_core::NurtureSessionStatus {
            phase: riviu_core::NurturePhase::Finished,
            outcome: Some(riviu_core::Outcome::Done),
            cleanup_state: riviu_core::NurtureCleanupState::ProcessAbsent,
            cleanup_proof: Some(riviu_core::ProcessAbsenceProof {
                bundle_id: "com.ss.android.ugc.trill".into(),
                old_pid: Some(42),
            }),
            ..Default::default()
        };
        assert!(!session_passes(&status, false));
        status.videos_done = 1;
        assert!(session_passes(&status, false));
        status.cleanup_proof = None;
        assert!(!session_passes(&status, false));
    }
    /// The promise in this file's own header, pinned the only way a promise about *source*
    /// can be — by reading the source, the same way the scheduler's gate is pinned.
    ///
    /// `run_session` re-reads the stored settings row once per post, and since this harness
    /// started inheriting the desktop app's database that row carries the operator's own
    /// `commentProb`, set for the app and not for this. If the merged settings ever take
    /// that field instead of overwriting it, a run asked for zero comments starts posting
    /// them on real accounts without anyone asking — which is exactly what happened with
    /// `follow_prob` on 18/08/2026, from the same channel.
    #[test]
    fn the_inherited_settings_can_never_switch_commenting_on() {
        // Only the program, never this module: the negative assertion below quotes the
        // string it is forbidding, so a whole-file scan finds its own footnote and passes
        // for the wrong reason.
        let whole = include_str!("live_nurture_android.rs");
        let source = whole
            .split("#[cfg(test)]")
            .next()
            .expect("the program above the tests");
        assert!(
            source.contains("comment_prob: args.comment_prob,"),
            "the rate has to come from the flag, unconditionally"
        );
        assert!(
            !source.contains("comment_prob: stored"),
            "the app's stored rate must not reach a run that did not ask for it"
        );
        assert!(
            source.contains("..stored"),
            "everything else is inherited on purpose — the key lives there and nowhere else"
        );
    }
}
