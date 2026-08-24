//! Run the nurture engine on several real Android phones at once, and report what each did.
//!
//! ```text
//! RIVIU_ADB_PATH=… cargo run -p riviu-managers-phone --bin live_nurture_android -- \
//!   --devices 4 --minutes 2 --videos 3
//! ```
//!
//! **Do not run this while the desktop app is open.** Two processes competing for the same
//! phones is the contention this project spent a week removing.
//!
//! Comments are **off unless `--comment-prob` says otherwise**, and switching them on posts
//! real text under the logged-in account. That is the operator's call and this harness now
//! takes it as an instruction rather than refusing it outright — the earlier version could
//! not be asked at all, which meant the feature could only ever be exercised by hand.
//!
//! The AI key is **never typed on the command line**. Settings are inherited from the
//! desktop app's own database — key, model, base url, language, tone directions and word
//! cap — by copying that file into the scratch directory and working on the copy, so a run
//! here can never rewrite what the operator configured in the app. The copy carries the key
//! in plaintext for the life of the run and is deleted at the end; a crash leaves it in
//! `%TEMP%`, which is the same exposure the app's own database already has.
//!
//! What it exercises is everything underneath — an exclusive lease, a
//! stream-budget slot, a UI session, TikTok in the foreground, watch and swipe and like —
//! per device, concurrently. That is the part that decides whether the feature works for a
//! whole fleet or only for the first couple of phones.
//!
//! It exists because the answer turned out to be "the first couple": every session holds a
//! foreground slot for its whole run, and the desktop's budget was two.

use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app_lib::interaction_ocr::DesktopFrameTextSource;
use riviu_core::db::Database;
use riviu_core::driver::DeviceDriver;
use riviu_core::{
    DeviceControlPlane, DeviceWorkCoordinator, NurtureEngine, NurtureSettings, StreamBudgetManager,
};
use riviu_ios_driver::StreamHub;
use uuid::Uuid;

/// Print and flush. Output is piped to a file here, where Rust block-buffers stdout — a run
/// that is killed part way would otherwise report nothing at all about what it had learned.
fn say(line: &str) {
    println!("{line}");
    let _ = std::io::stdout().flush();
}

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
}

fn parse_args() -> Args {
    let mut args = Args {
        devices: 2,
        only: Vec::new(),
        minutes: 2,
        videos: 3,
        like_prob: 30,
        follow_prob: 0,
        comment_prob: 0,
    };
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;
    while index + 1 < raw.len() {
        let value = raw[index + 1].parse::<u64>().unwrap_or(0);
        match raw[index].as_str() {
            "--devices" => args.devices = value as usize,
            "--minutes" => args.minutes = value,
            "--videos" => args.videos = value as u32,
            "--like-prob" => args.like_prob = value.min(100) as u32,
            "--follow-prob" => args.follow_prob = value.min(100) as u32,
            "--comment-prob" => args.comment_prob = value.min(100) as u32,
            "--only" => args.only = raw[index + 1].split(',').map(str::to_string).collect(),
            _ => {}
        }
        index += 2;
    }
    args
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args();

    let android = Arc::new(riviu_android_driver::AndroidDriver::new(
        &riviu_android_driver::AndroidDriverConfig::default(),
    )?);
    let streams = Arc::new(StreamHub::new());
    android.set_frame_sink(Arc::new(streams.as_ref().clone()));

    let devices = android.list_devices().await?;
    let targets: Vec<String> = devices
        .iter()
        .filter(|device| device.status != riviu_core::DeviceStatus::Disconnected)
        .filter(|device| args.only.is_empty() || args.only.contains(&device.udid))
        .take(args.devices)
        .map(|device| device.udid.clone())
        .collect();
    anyhow::ensure!(!targets.is_empty(), "no usable Android device is connected");

    // The number under test. Every nurture session holds one for its whole run, so this is
    // the ceiling on how many phones can nurture at once — whatever the fleet size is.
    let budget = StreamBudgetManager::new(
        std::env::var("RIVIU_STREAM_CAPACITY")
            .ok()
            .and_then(|raw| raw.trim().parse::<usize>().ok())
            .unwrap_or(targets.len()),
    );
    let budget = match budget {
        Ok(budget) => budget,
        Err(error) => {
            println!("stream budget refused: {error}");
            println!("  -> that refusal is the finding: the fleet cannot exceed it.");
            return Ok(());
        }
    };

    let scratch = std::env::temp_dir().join(format!("riviu-live-nurture-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&scratch)?;
    // **The desktop app's own database, copied.** The AI key, the model, the base url, the
    // comment language, the tone directions and the word cap all live there and nowhere
    // else, so a harness that starts from `Default` cannot write a comment at all — it has
    // no key. Copying rather than opening the real file means a run here can never rewrite
    // what the operator set in the app, and cannot be blamed for a settings change either.
    //
    // Missing file is not an error: everything except commenting works from defaults, and
    // saying so beats refusing to start.
    let app_db = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("riviu-managers-phone")
        .join("riviu.db");
    let inherited = if app_db.is_file() {
        match std::fs::copy(&app_db, scratch.join("riviu.db")) {
            Ok(_) => true,
            Err(error) => {
                say(&format!(
                    "không chép được cấu hình từ app ({error}) — chạy bằng mặc định"
                ));
                false
            }
        }
    } else {
        false
    };
    let database = Arc::new(Database::open(scratch.join("riviu.db"))?);
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
    let stored = database.get_nurture_settings().unwrap_or_default();
    let has_key = !stored.api_key.trim().is_empty();
    println!(
        "cấu hình: {} — model {}, ngôn ngữ {}, tối đa {} từ, định hướng {:?}, khoá API {}",
        if inherited {
            "kế thừa từ app"
        } else {
            "mặc định (không thấy CSDL của app)"
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

    let settings = NurtureSettings {
        num_videos: args.videos,
        num_rounds: 1,
        like_prob: args.like_prob,
        comment_prob: args.comment_prob,
        follow_prob: args.follow_prob,
        frenzy_prob: 0,
        watch_min: 2.0,
        watch_max: 4.0,
        stagger_delay_min: 1,
        stagger_delay_max: 3,
        ..stored
    };

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
    for (index, udid) in targets.iter().cloned().enumerate() {
        let engine = engine.clone();
        let settings = settings.clone();
        let stop = Arc::new(AtomicBool::new(false));
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
    say(&format!(
        "{:<22} {:>7} {:>7} {:>7}  outcome",
        "device", "secs", "videos", "likes"
    ));
    for handle in running {
        let (udid, elapsed, outcome) = handle.await?;
        match outcome {
            Ok(status) => {
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
            Err(error) => println!(
                "{udid:<22} {:>7.0} {:>7} {:>7}  ERROR: {error:#}",
                elapsed.as_secs_f64(),
                0,
                0
            ),
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

    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}

#[cfg(test)]
mod tests {
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
