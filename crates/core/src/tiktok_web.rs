//! What the operator's own machine can learn about a TikTok post without touching a phone.
//!
//! Everything the comment writer knows today it read off a phone screen: the caption comes
//! out of the accessibility tree and the pictures come out of a live scrcpy stream. Both are
//! lossy in ways that were measured rather than guessed, and this module exists to fill in
//! what they lose — from the operator's desktop, over the operator's own network, with no
//! device lease, no stream and no gesture.
//!
//! **Measured 26/08/2026 on the seven real targets in this box's `riviu.db`:**
//!
//! | | |
//! |---|---|
//! | caption from the web | 157, 171, 184, 216, 399 characters |
//! | caption from the tree | truncated at ~116 characters (`docs/PLAN_STATUS_2026-08-13.md`) |
//! | carousel slides from the web | 2, 5, 7, 8 pictures at 1416x2008 |
//! | carousel slides from a phone | `CAROUSEL_SLIDE_CAP` = 4, reached by swiping a real account |
//! | posts with an ASR transcript | **0 of 7** |
//!
//! So on the everyday workload the prize is the **caption in full** and **every slide of a
//! photo post**. The transcript everybody reaches for first scored nothing here: six of seven
//! targets are photo posts, which have no speech, and the one video reported
//! `"hasOriginalAudio": false` with `"noCaptionReason": 3`.
//!
//! **But where a transcript exists it is the richest evidence by a wide margin**, so the path
//! is built and gated rather than skipped. Measured on a 52-second talking-head vlog: 222 words
//! naming six specific places, against a contact sheet built from roughly its first second.
//! [`PostWebContext::could_have_transcript`] is the gate, and it answers from the page the
//! lookup already fetched — a music-track post costs nothing to rule out.
//!
//! # Why yt-dlp and not `reqwest`
//!
//! A plain GET of a post URL with a browser user-agent returns **HTTP 200 and 1462 bytes with
//! no post data in it** — measured the same day. TikTok answers a bare request with a shell.
//! yt-dlp gets through because it solves a JS challenge and retries with the resulting cookie
//! (`Solving JS challenge using native Python implementation` → `Downloading webpage with
//! challenge cookie`). Reimplementing that here would be re-solving a problem that a
//! maintained project already solves and that TikTok changes on its own schedule.
//!
//! The binary is a sidecar, exactly like `scrcpy-server` and `pymobiledevice3` — see
//! [`resolve_ytdlp`] for the search order.
//!
//! # This is an enrichment, never a source of truth
//!
//! Two of the seven real targets answered `Your IP address is blocked from accessing this
//! post` on three consecutive attempts. The phones, on Vietnamese mobile networks, open those
//! same posts fine. So every function here is best-effort by contract: a failure means the
//! campaign writes its comment the way it always has, from what the phone can see. Nothing in
//! this module may ever be the reason a phone stays silent.

use std::path::PathBuf;
use std::time::Duration;

/// What one post looks like from the desktop's side of the network.
///
/// Every field is optional or empty-able on purpose: this is assembled from a page that
/// TikTok reshapes without notice, and a missing field has to degrade to "we did not learn
/// that" rather than to a wrong answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostWebContext {
    /// The post's caption, in full. This is the field that pays for the whole module.
    pub caption: Option<String>,
    /// Length in seconds. A photo post reports one too — TikTok gives carousels a duration
    /// from their backing audio, so this does **not** distinguish a video from a carousel.
    pub duration_secs: Option<u64>,
    /// Every slide of a photo post, in flip order, as CDN URLs.
    ///
    /// Empty for a video, and empty for a photo post whose page shape changed. Note these are
    /// signed and short-lived (`x-expires` measured a few hours out), so they are fetched
    /// during the campaign and never cached.
    pub slide_urls: Vec<String>,
    /// Whether the video carries speech at all.
    ///
    /// `Some(false)` is a **free** answer to "could this post ever have a transcript" — the
    /// one real video target on this farm reported exactly that, alongside an empty
    /// `captionInfos` and `noCaptionReason: 3`. Ask it before spending a request on subtitles.
    pub has_original_audio: Option<bool>,
    /// The subtitle tracks TikTok is willing to hand out, if any.
    ///
    /// Measured on a talking-head vlog: `vie-VN` with `"Source": "ASR"` — the original speech —
    /// and `eng-US` with `"Source": "MT"`, a machine translation of it. Empty on every photo
    /// post and on any video whose audio is a music track rather than a voice.
    pub subtitles: Vec<SubtitleTrack>,
    /// The post's cover picture.
    ///
    /// Not evidence a campaign uses — production frames come from the phone — but the one
    /// picture a headless run can put on a sheet without taking a device, which is what makes
    /// `carousel_comment --link` work on a video at all.
    pub cover_url: Option<String>,
}

/// One subtitle track offered for a post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleTrack {
    /// TikTok's own code, e.g. `vie-VN`.
    pub lang: String,
    /// `ASR` for the original speech, `MT` for a machine translation of it.
    ///
    /// **This is the field that picks a track, not the language code.** A comment has to be
    /// grounded in what was actually said; an `MT` track is a translation of that, one
    /// generation further from the audio and free to lose a proper noun on the way. The
    /// measured vlog offered `vie-VN`/ASR and `eng-US`/MT, and only the first is the video.
    pub source: String,
    /// Signed and short-lived — `UrlExpire` measured a few hours out — so it is fetched during
    /// the campaign and never stored.
    pub url: String,
}

impl PostWebContext {
    /// Whether this told us anything worth carrying.
    pub fn is_empty(&self) -> bool {
        self.caption.is_none() && self.slide_urls.is_empty()
    }

    /// The language codes, for a log line or a filed note.
    pub fn subtitle_langs(&self) -> Vec<String> {
        self.subtitles
            .iter()
            .map(|track| track.lang.clone())
            .collect()
    }

    /// The track a transcript should come from, if any.
    ///
    /// Prefers `ASR` — the original speech. Falls back to whatever is offered rather than to
    /// nothing, because a translated transcript still names the places a comment can be about,
    /// and no transcript names none of them.
    pub fn transcript_track(&self) -> Option<&SubtitleTrack> {
        self.subtitles
            .iter()
            .find(|track| track.source.eq_ignore_ascii_case("ASR"))
            .or_else(|| self.subtitles.first())
    }

    /// Whether asking for a transcript could possibly return one.
    ///
    /// **Two conditions, and the first is the cheap one.** `hasOriginalAudio: false` means the
    /// post is carrying a music track and there is no speech to transcribe — measured on this
    /// farm's only video target, which reported exactly that alongside `noCaptionReason: 3`.
    /// Asking anyway is a request spent to be told nothing.
    pub fn could_have_transcript(&self) -> bool {
        self.has_original_audio != Some(false) && self.transcript_track().is_some()
    }
}

/// Why a lookup did not produce a context, **and whether asking again could help**.
///
/// The distinction is the whole point of the type. Measured 26/08/2026 across the real
/// targets: `Unable to extract universal data for rehydration` cleared on the next attempt
/// (4 of 5 runs succeeded, the failing one succeeded on retry), while `Your IP address is
/// blocked` returned the identical message on three consecutive attempts. Retrying the second
/// kind is pure latency added to a campaign that is holding devices open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebLookupError {
    /// No usable `yt-dlp` on this machine. Not retryable, and not an error worth shouting
    /// about on every target — the campaign simply runs the way it did before this module.
    NoBinary(String),
    /// TikTok refused this post to this network. Retrying changes nothing.
    Blocked,
    /// TikTok says the post is gone, private, or not a post.
    Unavailable(String),
    /// A shape yt-dlp could not read, a timeout, a dropped connection. Worth one more go.
    Transient(String),
}

impl WebLookupError {
    /// Whether another attempt is worth the campaign's time.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient(_))
    }

    /// The short token that goes into a log line, so a run can be counted afterwards.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoBinary(_) => "no_ytdlp",
            Self::Blocked => "ip_blocked",
            Self::Unavailable(_) => "post_unavailable",
            Self::Transient(_) => "transient",
        }
    }
}

impl std::fmt::Display for WebLookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBinary(detail) => write!(f, "không có yt-dlp: {detail}"),
            Self::Blocked => write!(f, "TikTok chặn IP này với bài đó"),
            Self::Unavailable(detail) => write!(f, "bài không truy cập được: {detail}"),
            Self::Transient(detail) => write!(f, "lỗi tạm thời: {detail}"),
        }
    }
}

/// How long one `yt-dlp` invocation may take before the campaign stops waiting.
///
/// Measured runs land at 2–6 seconds including the JS challenge. Sixty is not a target, it is
/// the point past which something is wrong and a campaign holding twenty device leases should
/// stop waiting for it.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(60);

/// How many times a *retryable* failure is retried. Three attempts total.
const LOOKUP_ATTEMPTS: usize = 3;

/// How long one slide image download may take.
const SLIDE_TIMEOUT: Duration = Duration::from_secs(30);

/// Rewrite a post URL into the form yt-dlp's TikTok extractor accepts.
///
/// **Two rewrites, both learned the hard way** — the first in this project's sibling
/// `Riviudalat/RiviudownloadTik`, the second re-measured here on 26/08/2026:
///
/// - A `/photo/` URL is rejected outright with `ERROR: Unsupported URL`, and the identical
///   post under `/video/` resolves and carries its full `imagePost.images` array. TikTok
///   serves one object under two paths; only one of them has an extractor.
/// - A handle that begins with `.` (this farm really has one, `@.lt.gi.mang.v`) breaks the
///   extractor's own URL parsing. The numeric id is what actually selects the post, so the
///   handle is replaced with `x` rather than repaired.
///
/// Returns `None` when there is no post id to build from, which is the only case where
/// guessing would be inventing a target.
pub fn normalize_for_ytdlp(url: &str) -> Option<String> {
    let id = post_id_of(url)?;
    let handle = handle_of(url).filter(|h| !h.starts_with('.'));
    let handle = handle.unwrap_or("x");
    Some(format!("https://www.tiktok.com/@{handle}/video/{id}"))
}

/// The numeric post id inside a TikTok URL, under any of the three path words that carry one.
fn post_id_of(url: &str) -> Option<&str> {
    for marker in ["/video/", "/photo/", "/v/"] {
        let Some(rest) = url.split(marker).nth(1) else {
            continue;
        };
        let digits: &str = rest
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap_or_default();
        if digits.len() >= 10 {
            return Some(digits);
        }
    }
    None
}

/// The `@handle` segment, without its `@`.
fn handle_of(url: &str) -> Option<&str> {
    let rest = url.split("tiktok.com/@").nth(1)?;
    let handle = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .trim();
    (!handle.is_empty()).then_some(handle)
}

/// Read yt-dlp's complaint and decide whether asking again could change the answer.
///
/// Keyed on the message text because that is the only channel yt-dlp offers — its exit code
/// is `1` for every one of these. The strings are the ones this farm actually produced; an
/// unrecognised failure is treated as [`WebLookupError::Transient`] so that a message nobody
/// has seen yet gets its retry rather than being written off.
pub fn classify_lookup_error(stderr: &str) -> WebLookupError {
    let lower = stderr.to_lowercase();
    if lower.contains("ip address is blocked") {
        return WebLookupError::Blocked;
    }
    if lower.contains("video not available")
        || lower.contains("post not available")
        || lower.contains("content isn't available")
        || lower.contains("unsupported url")
        || lower.contains("account is private")
    {
        return WebLookupError::Unavailable(first_line(stderr));
    }
    WebLookupError::Transient(first_line(stderr))
}

/// The first non-empty line, trimmed and bounded — a whole yt-dlp stderr in a log line is
/// noise, and its first line is always the `ERROR:` one.
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string();
    line.chars().take(240).collect()
}

/// The copy the installer shipped, told to us rather than guessed at.
///
/// **This exists because guessing did not work, and the way it failed was silent.** Every
/// candidate `resolve_ytdlp` searches on its own is either next to the running executable or
/// derived from `CARGO_MANIFEST_DIR` — a *compile-time* path that on an operator's machine
/// points at the build agent's checkout. A packaged build therefore found nothing, every lookup
/// returned `NoBinary`, and the whole enrichment path did nothing at all while every test and
/// every dev run stayed green.
///
/// The desktop knows where its resources landed (`state::resolve_sidecar_root` handles both the
/// packaged and the dev layout), so it says so at bootstrap. Same convention, and same
/// precedence, as `AndroidDriverConfig::bundled_adb_path`: **below** `RIVIU_YTDLP_PATH`, because
/// a bundled path the operator cannot outrank is not a safety net.
static BUNDLED_YTDLP: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Record the packaged `yt-dlp`, once, at startup.
///
/// Ignores a path that is not a file, so a build without the sidecar behaves exactly as before
/// rather than pinning a resolution that will fail on every call.
pub fn set_bundled_ytdlp(path: PathBuf) {
    if path.is_file() {
        let _ = BUNDLED_YTDLP.set(path);
    }
}

/// Where to find `yt-dlp`, in the order a running build is most likely to have it.
///
/// The same shape as this repo's other sidecars, plus the two layouts a Tauri bundle uses.
/// `RIVIU_YTDLP_PATH` comes first so an operator can point a run at a newer binary without a
/// rebuild — which matters more here than for the other sidecars, because TikTok breaks
/// extractors on its own schedule and the fix is always "get a newer yt-dlp".
pub fn resolve_ytdlp() -> Result<PathBuf, WebLookupError> {
    let exe_name = if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };

    if let Ok(custom) = std::env::var("RIVIU_YTDLP_PATH") {
        let path = PathBuf::from(&custom);
        if path.is_file() {
            return Ok(path);
        }
        return Err(WebLookupError::NoBinary(format!(
            "RIVIU_YTDLP_PATH={custom} không tồn tại"
        )));
    }
    // Second, and above every guess: the path the host application handed over.
    if let Some(bundled) = BUNDLED_YTDLP.get() {
        return Ok(bundled.clone());
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(current) = std::env::current_exe() {
        if let Some(dir) = current.parent() {
            candidates.push(dir.join(exe_name));
            candidates.push(dir.join("binaries").join(exe_name));
            candidates.push(dir.join("sidecars").join("yt-dlp").join(exe_name));
            if let Some(parent) = dir.parent() {
                candidates.push(parent.join("Resources").join(exe_name));
                candidates.push(
                    parent
                        .join("Resources")
                        .join("sidecars")
                        .join("yt-dlp")
                        .join(exe_name),
                );
            }
        }
    }
    // The repo's own sidecar tree, for `cargo run` and for the test farm. **Compile-time**, so
    // it is worth nothing in a packaged build — see `BUNDLED_YTDLP`.
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/yt-dlp");
    candidates.push(repo.join(exe_name));

    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    // Last: whatever is on PATH. `is_file` cannot check this, so it is returned bare and the
    // spawn failure below is what reports it.
    Ok(PathBuf::from(exe_name))
}

/// Ask the desktop's own network what it can see of one post.
///
/// One `yt-dlp` invocation does both halves of the job. `-J` prints the extractor's metadata
/// to stdout, which is where the caption and duration come from; `--write-pages` drops the
/// page it already downloaded next to it, which is where `imagePost.images` and `claInfo`
/// come from. **Both are needed** — measured 26/08/2026, `-J` alone reports only two
/// thumbnails for an eight-slide carousel, while the page it was parsed from carries all
/// eight at 1416x2008.
///
/// `--skip-download` throughout: nothing here fetches the video itself.
pub async fn fetch_post_context(url: &str) -> Result<PostWebContext, WebLookupError> {
    let normalized = normalize_for_ytdlp(url)
        .ok_or_else(|| WebLookupError::Unavailable(format!("không có post id trong {url:?}")))?;

    // **One lookup per link, however many phones are about to comment on it — but links do
    // not queue behind each other.**
    //
    // A `Standalone` campaign gives every assignment its own task (§9.108), so twenty phones
    // on one link are twenty tasks running this function at the same moment. Twenty identical
    // requests to TikTok from one address inside a few seconds is the behaviour most likely
    // to earn the block that already costs this farm two targets in seven — so the same link
    // must collapse to one request.
    //
    // The first version held the memo's single mutex across the whole lookup, which did
    // collapse the same link — and serialised every *different* link behind it too: a
    // campaign over three posts, the first timing out three times, made posts two and three
    // wait ~180 s before they even started, each holding a device lease idle. So the memo
    // lock is now held only long enough to read or write the map, and the "one request per
    // link" guarantee moves to a **per-key** lock: same link waits, different links run at
    // once.
    if let Some(result) = fresh_memo(&normalized).await {
        return result;
    }
    let key_lock = inflight_lock(&normalized).await;
    let _guard = key_lock.lock().await;
    // Re-check under the key lock: a concurrent caller for this same link may have just
    // finished and filled the memo while we waited.
    if let Some(result) = fresh_memo(&normalized).await {
        remove_inflight_lock(&normalized, &key_lock).await;
        return result;
    }
    let result = fetch_post_context_uncached(&normalized).await;
    LOOKUP_MEMO.lock().await.insert(
        normalized.clone(),
        MemoEntry {
            at: std::time::Instant::now(),
            result: result.clone(),
        },
    );
    // The memo is visible before this removal, so a new caller does not start another lookup.
    // Remove by identity rather than by `strong_count`: queued waiters necessarily hold Arcs,
    // and counting them made every contended one-shot URL stay in this process-wide table.
    remove_inflight_lock(&normalized, &key_lock).await;
    result
}

/// The memoised result for `normalized`, if one is present and still within TTL.
///
/// A separate function so the memo mutex is taken, read, and released — never held across a
/// network call.
async fn fresh_memo(normalized: &str) -> Option<Result<PostWebContext, WebLookupError>> {
    let memo = LOOKUP_MEMO.lock().await;
    memo.get(normalized)
        .and_then(|entry| (entry.at.elapsed() < LOOKUP_MEMO_TTL).then(|| entry.result.clone()))
}

/// The single-flight lock for one link. Callers on the *same* link share it and so make one
/// request; callers on *different* links get different locks and do not wait on each other.
async fn inflight_lock(normalized: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    let mut inflight = LOOKUP_INFLIGHT.lock().await;
    inflight
        .entry(normalized.to_string())
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// Remove this completed generation without deleting a newer lock for the same key.
async fn remove_inflight_lock(
    normalized: &str,
    completed: &std::sync::Arc<tokio::sync::Mutex<()>>,
) {
    let mut inflight = LOOKUP_INFLIGHT.lock().await;
    if inflight
        .get(normalized)
        .is_some_and(|held| std::sync::Arc::ptr_eq(held, completed))
    {
        inflight.remove(normalized);
    }
}

/// One remembered lookup. Both outcomes are kept: a target TikTok refuses is refused for every
/// phone on it, and re-asking twenty times would only prove that twenty times over.
struct MemoEntry {
    at: std::time::Instant,
    result: Result<PostWebContext, WebLookupError>,
}

/// How long a lookup stands.
///
/// **Bounded by the CDN, not by taste.** The slide and subtitle URLs in a context are signed
/// with an `x-expires` a few hours out, so a context is only useful while they are live. Five
/// minutes covers a campaign's whole fan-out — twenty staggered tasks on one link — and is far
/// inside any measured expiry.
const LOOKUP_MEMO_TTL: Duration = Duration::from_secs(300);

static LOOKUP_MEMO: std::sync::LazyLock<
    tokio::sync::Mutex<std::collections::HashMap<String, MemoEntry>>,
> = std::sync::LazyLock::new(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));

/// Per-link single-flight locks. Keyed by the normalised link, so two callers on one link
/// share a lock and make one request, while two callers on different links do not queue.
#[allow(clippy::type_complexity)]
static LOOKUP_INFLIGHT: std::sync::LazyLock<
    tokio::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));

async fn fetch_post_context_uncached(normalized: &str) -> Result<PostWebContext, WebLookupError> {
    let binary = resolve_ytdlp()?;

    let mut last = WebLookupError::Transient("chưa chạy lần nào".into());
    for attempt in 1..=LOOKUP_ATTEMPTS {
        match run_lookup(&binary, normalized).await {
            Ok(context) => return Ok(context),
            Err(error) => {
                if !error.is_retryable() {
                    return Err(error);
                }
                tracing::debug!(
                    "tiktok_web: lượt {attempt}/{LOOKUP_ATTEMPTS} cho {normalized} lỗi: {error}"
                );
                last = error;
            }
        }
    }
    Err(last)
}

/// One invocation, in a scratch directory that is removed whatever happens.
///
/// The scratch directory is not a tidiness preference: `--write-pages` writes into the
/// **current directory**, and the current directory of this process is the operator's
/// working directory. Without `current_dir` a campaign would scatter half-megabyte `.dump`
/// files wherever the app happened to be launched from.
async fn run_lookup(binary: &PathBuf, normalized: &str) -> Result<PostWebContext, WebLookupError> {
    let scratch = std::env::temp_dir().join(format!("riviu-tiktok-web-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&scratch).await.map_err(|error| {
        WebLookupError::Transient(format!("không tạo được thư mục tạm: {error}"))
    })?;

    let result = run_lookup_in(binary, normalized, &scratch).await;
    // Best-effort: a scratch directory that survives is litter, not a failure to report.
    let _ = tokio::fs::remove_dir_all(&scratch).await;
    result
}

async fn run_lookup_in(
    binary: &PathBuf,
    normalized: &str,
    scratch: &std::path::Path,
) -> Result<PostWebContext, WebLookupError> {
    let mut command = tokio::process::Command::new(binary);
    command
        .current_dir(scratch)
        .arg("--no-warnings")
        .arg("--no-playlist")
        .arg("--skip-download")
        .arg("--write-pages")
        .arg("-J")
        .arg(normalized)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // **Killed when the lookup future is dropped, which the timeout below does.** Without
        // this, a `LOOKUP_TIMEOUT` firing drops `command.output()` but leaves yt-dlp running,
        // and the next of three attempts spawns another — up to three extractor processes
        // hitting one post from one IP at once, which is exactly how a fleet earns an IP
        // block, plus the scratch dir each leaves behind.
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // No console window for a background lookup: without it every target pops a
        // console on the operator's desktop mid-campaign. `CREATE_NO_WINDOW`.
        command.creation_flags(0x0800_0000);
    }

    let output = match tokio::time::timeout(LOOKUP_TIMEOUT, command.output()).await {
        Err(_) => {
            return Err(WebLookupError::Transient(format!(
                "quá {} giây",
                LOOKUP_TIMEOUT.as_secs()
            )))
        }
        Ok(Err(error)) => {
            return Err(WebLookupError::NoBinary(format!(
                "{} không chạy được: {error}",
                binary.display()
            )))
        }
        Ok(Ok(output)) => output,
    };

    if !output.status.success() {
        return Err(classify_lookup_error(&String::from_utf8_lossy(
            &output.stderr,
        )));
    }

    let info = String::from_utf8_lossy(&output.stdout).into_owned();
    let page = read_page_dump(scratch).await;
    Ok(parse_post_context(&info, page.as_deref().unwrap_or("")))
}

/// The page yt-dlp saved beside its JSON, if it saved one.
///
/// Only the largest is read: a lookup that followed a redirect leaves more than one dump, and
/// the post page is always the big one (430 KB measured, against a few KB for a challenge
/// interstitial).
async fn read_page_dump(scratch: &std::path::Path) -> Option<String> {
    let mut entries = tokio::fs::read_dir(scratch).await.ok()?;
    let mut best: Option<(u64, PathBuf)> = None;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("dump") {
            continue;
        }
        let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
        if best.as_ref().is_none_or(|(seen, _)| size > *seen) {
            best = Some((size, path));
        }
    }
    let (_, path) = best?;
    tokio::fs::read(&path)
        .await
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// Assemble a context from the two things one lookup produces.
///
/// Split out from the process handling so it can be tested against captured fixtures rather
/// than against TikTok — the page shape is the part that changes, and a change in it must
/// show up as a failing test here rather than as thinner comments in production.
pub fn parse_post_context(info_json: &str, page_dump: &str) -> PostWebContext {
    let info: serde_json::Value =
        serde_json::from_str(info_json).unwrap_or(serde_json::Value::Null);

    let caption = info
        .get("description")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    let duration_secs = info
        .get("duration")
        .and_then(|value| value.as_f64())
        .filter(|seconds| *seconds >= 0.0)
        .map(|seconds| seconds.round() as u64);

    let slide_urls = parse_slide_urls(page_dump);
    let (has_original_audio, subtitles) = parse_audio_facts(page_dump);
    let cover_url = info
        .get("thumbnail")
        .and_then(|value| value.as_str())
        .filter(|url| url.starts_with("http"))
        .map(str::to_string);

    PostWebContext {
        caption,
        duration_secs,
        slide_urls,
        has_original_audio,
        subtitles,
        cover_url,
    }
}

/// Every slide of a photo post, in flip order.
///
/// Reads `imagePost.images[].imageURL.urlList[]` out of the page. The list holds the same
/// picture on several CDN hosts, so only the first of each is taken — they are alternates,
/// not additional slides, and treating them as slides would report an eight-slide post as
/// sixteen.
fn parse_slide_urls(page_dump: &str) -> Vec<String> {
    let Some(raw) = json_value_after(page_dump, "\"imagePost\"") else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let Some(images) = value.get("images").and_then(|images| images.as_array()) else {
        return Vec::new();
    };
    images
        .iter()
        .filter_map(|image| {
            image
                .get("imageURL")?
                .get("urlList")?
                .as_array()?
                .iter()
                .find_map(|url| url.as_str())
                .map(str::to_string)
        })
        .collect()
}

/// Whether a transcript could exist, and which languages are on offer.
///
/// `claInfo.hasOriginalAudio` is the cheap gate — a post that reports `false` has no speech
/// to transcribe and no request should be spent on it. Measured on this farm's only video
/// target: `"hasOriginalAudio": false, "captionInfos": [], "noCaptionReason": 3`.
fn parse_audio_facts(page_dump: &str) -> (Option<bool>, Vec<SubtitleTrack>) {
    let has_audio = json_value_after(page_dump, "\"claInfo\"")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|value| {
            value
                .get("hasOriginalAudio")
                .and_then(|flag| flag.as_bool())
        });

    let tracks = json_value_after(page_dump, "\"subtitleInfos\"")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|value| value.as_array().cloned())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let url = entry.get("Url").and_then(|url| url.as_str())?;
                    Some(SubtitleTrack {
                        lang: entry
                            .get("LanguageCodeName")
                            .and_then(|name| name.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        source: entry
                            .get("Source")
                            .and_then(|source| source.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        url: url.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    (has_audio, tracks)
}

/// The most transcript words that travel into a prompt.
///
/// The measured 52-second vlog transcribes to 222 words, which is the shape this is sized
/// against: enough to carry the whole narration of a normal TikTok, and a hard stop so a
/// ten-minute upload cannot quietly put a few thousand words into every verification call.
pub const TRANSCRIPT_MAX_WORDS: usize = 240;

/// Fetch and flatten what is said in a video.
///
/// **Read straight off the CDN rather than through a second `yt-dlp` run.** The track URL is
/// already in the page the first lookup fetched, and it answers a plain request carrying a
/// browser user-agent and a tiktok.com referer — measured 26/08/2026, 1749 bytes of WebVTT for
/// the 52-second vlog. Going back through the extractor would mean a second JS challenge and a
/// second chance at the ~1-in-5 transient failure, for the same bytes.
///
/// `None` on anything unexpected. A campaign without a transcript writes what it always wrote.
pub async fn fetch_transcript(track: &SubtitleTrack) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(SLIDE_TIMEOUT)
        .user_agent(BROWSER_UA)
        .build()
        .ok()?;
    let response = client
        .get(&track.url)
        .header(reqwest::header::REFERER, "https://www.tiktok.com/")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        tracing::warn!(
            "tiktok_web: phụ đề {} trả về HTTP {}",
            track.lang,
            response.status()
        );
        return None;
    }
    let vtt = response.text().await.ok()?;
    transcript_from_vtt(&vtt, TRANSCRIPT_MAX_WORDS)
}

/// Turn a WebVTT track into one line of what was said.
///
/// Timings are dropped on purpose. A comment is a reaction to the whole post, not to a moment
/// in it, and timestamps would spend prompt budget on structure nothing reads — the measured
/// track is 25 cues carrying 222 words, and the cues are mid-sentence splits of continuous
/// speech (`ăn một` / `tô là đủ nạp đầy năng lượng`), so putting them back together is what
/// makes it readable at all.
///
/// **Consecutive repeats are collapsed.** TikTok's ASR re-emits a cue's tail as the head of the
/// next one on some tracks, and a transcript that says the same clause twice reads to a model
/// as emphasis.
pub fn transcript_from_vtt(vtt: &str, max_words: usize) -> Option<String> {
    let mut lines: Vec<&str> = Vec::new();
    for line in vtt.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("WEBVTT")
            || line.starts_with("NOTE")
            || line.contains("-->")
            // A bare cue number, which numbered tracks put above each timing line.
            || line.chars().all(|character| character.is_ascii_digit())
        {
            continue;
        }
        if lines.last() == Some(&line) {
            continue;
        }
        lines.push(line);
    }
    if lines.is_empty() {
        return None;
    }
    let joined = lines.join(" ");
    let mut words = joined.split_whitespace();
    let kept: Vec<&str> = words.by_ref().take(max_words).collect();
    let trimmed = words.next().is_some();
    let mut text = kept.join(" ");
    if trimmed {
        // Said rather than hidden: a model told this is the whole narration would happily
        // conclude the video ends where the text does.
        text.push_str(" […phần sau chưa đọc]");
    }
    Some(text)
}

/// Extract the `{...}` or `[...]` that follows `"key":` in a raw page.
///
/// The page is a JSON document embedded in HTML that is itself full of escaped JSON strings,
/// so this is a brace matcher that **tracks string and escape state**. A naive depth counter
/// gets this wrong the first time a caption contains a `}`, which on TikTok is often.
///
/// Returns the substring, which is valid JSON on its own and is handed to `serde_json` to
/// decode — including `/`, which the page uses for every `/` in every URL.
fn json_value_after(raw: &str, key: &str) -> Option<String> {
    let key_at = raw.find(key)?;
    let after_key = &raw[key_at + key.len()..];
    let colon = after_key.find(':')?;
    let value = after_key[colon + 1..].trim_start();

    let (open, close) = match value.as_bytes().first()? {
        b'{' => (b'{', b'}'),
        b'[' => (b'[', b']'),
        _ => return None,
    };

    let bytes = value.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match *byte {
            b'"' => in_string = true,
            b if b == open => depth += 1,
            b if b == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(value[..=index].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Which slides to put on the contact sheet when the post has more than fit.
///
/// **Not "the first `want`".** The sheet holds four pictures (`openai_client::
/// SHEET_MAX_FRAMES`) and this farm's real carousels run to five, seven and eight, so
/// something has to be dropped — and §9.109 measured what happens when the dropped ones are
/// the later ones: a two-slide post whose second slide carried a costed three-day itinerary
/// got every comment written from slide one, a person lying by a lake.
///
/// So the picks are spread evenly and **always include the first and the last**: the first is
/// the hook the post is built around, the last is where a carousel puts its summary or its
/// call to action. The middle picks are evenly spaced between them.
///
/// Indices come back sorted and unique, so the sheet still reads left to right in flip order.
pub fn pick_slide_indices(total: usize, want: usize) -> Vec<usize> {
    if total == 0 || want == 0 {
        return Vec::new();
    }
    if total <= want {
        return (0..total).collect();
    }
    if want == 1 {
        return vec![0];
    }
    let last = total - 1;
    let steps = want - 1;
    let mut picks: Vec<usize> = (0..want)
        .map(|step| (step * last).div_ceil(steps).min(last))
        .collect();
    picks.dedup();
    picks
}

/// Download the chosen slides, in order, skipping any that will not come.
///
/// **A `Referer` is required and a cookie is not** — measured 26/08/2026: the signed CDN URL
/// answers a request carrying a browser user-agent and `https://www.tiktok.com/` as referer
/// with the full 296 KB JPEG, and this is the same host that refuses the post page itself.
///
/// Best-effort per slide. Three slides out of four is a thinner sheet; a hard failure here
/// would be a target that gets no comment at all, which is never the right trade.
pub async fn fetch_slides(urls: &[String]) -> Vec<Vec<u8>> {
    let Ok(client) = reqwest::Client::builder()
        .timeout(SLIDE_TIMEOUT)
        .user_agent(BROWSER_UA)
        .build()
    else {
        return Vec::new();
    };

    let mut downloaded: Vec<Option<Vec<u8>>> = Vec::with_capacity(urls.len());
    for url in urls {
        match fetch_one_slide(&client, url).await {
            Ok(bytes) => downloaded.push(Some(bytes)),
            Err(error) => {
                tracing::warn!("tiktok_web: không tải được một ảnh của bài: {error}");
                downloaded.push(None);
            }
        }
    }
    slides_if_first_and_last_present(downloaded)
}

/// Keep the downloaded slides only when the **first and last** of them arrived; otherwise
/// hand back nothing so the caller falls back to on-device capture.
///
/// The picked URLs are in slide order and always include the first and last slide — the
/// contact-sheet prompt says so in as many words ("first and last are present"), because a
/// carousel's payload is measured to live on the last slide. The old version compacted
/// whatever downloaded into an unindexed vector, so a failed **last** slide vanished silently
/// and the model wrote about the cover while the prompt still claimed the payload was there —
/// a wrong-topic comment on a real account. A failed **middle** slide is harmless (first and
/// last still frame the post), so it is dropped rather than discarding the whole set; only a
/// missing bookend breaks the guarantee, and that returns empty to route to the phone, which
/// shows the real current slides.
fn slides_if_first_and_last_present(downloaded: Vec<Option<Vec<u8>>>) -> Vec<Vec<u8>> {
    let bookends_present = matches!(
        (downloaded.first(), downloaded.last()),
        (Some(Some(_)), Some(Some(_)))
    );
    if !bookends_present {
        return Vec::new();
    }
    downloaded.into_iter().flatten().collect()
}

/// The user-agent the CDN measurement was taken with.
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                          (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

/// The smallest response that could be a real slide.
///
/// TikTok answers a rejected image request with a short error body rather than a status code
/// in some regions, and a 300-byte "JPEG" put on a contact sheet is a grey rectangle the
/// model will describe. Measured slides are 200–400 KB.
const MIN_SLIDE_BYTES: usize = 4 * 1024;

async fn fetch_one_slide(client: &reqwest::Client, url: &str) -> anyhow::Result<Vec<u8>> {
    let response = client
        .get(url)
        .header(reqwest::header::REFERER, "https://www.tiktok.com/")
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status}");
    }
    let bytes = response.bytes().await?.to_vec();
    if bytes.len() < MIN_SLIDE_BYTES {
        anyhow::bail!("chỉ {} byte, không phải một tấm ảnh", bytes.len());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `/photo/` URL is rewritten to `/video/`, because only one of the two has an
    /// extractor. Measured: the `/photo/` form returns `ERROR: Unsupported URL`.
    #[test]
    fn a_photo_url_is_rewritten_to_the_form_that_resolves() {
        assert_eq!(
            normalize_for_ytdlp(
                "https://www.tiktok.com/@mongquynh.dalat/photo/7668954054680136967"
            )
            .as_deref(),
            Some("https://www.tiktok.com/@mongquynh.dalat/video/7668954054680136967")
        );
    }

    /// A handle that begins with `.` is replaced, not repaired.
    ///
    /// `@.lt.gi.mang.v` is a real target of this farm. The numeric id selects the post, so
    /// dropping the handle costs nothing and keeping it breaks the extractor.
    #[test]
    fn a_leading_dot_handle_is_replaced_by_a_placeholder() {
        assert_eq!(
            normalize_for_ytdlp("https://www.tiktok.com/@.lt.gi.mang.v/photo/7668947001618320660")
                .as_deref(),
            Some("https://www.tiktok.com/@x/video/7668947001618320660")
        );
    }

    /// Tracking query and fragment do not travel.
    #[test]
    fn query_and_fragment_are_dropped() {
        assert_eq!(
            normalize_for_ytdlp("https://www.tiktok.com/@a/video/7668947001618320660?is_from=x#y")
                .as_deref(),
            Some("https://www.tiktok.com/@a/video/7668947001618320660")
        );
    }

    /// No id means no target, and inventing one would mean commenting on someone else's post.
    #[test]
    fn a_url_without_a_post_id_normalizes_to_nothing() {
        assert_eq!(normalize_for_ytdlp("https://www.tiktok.com/@someone"), None);
        assert_eq!(
            normalize_for_ytdlp("https://www.tiktok.com/@a/video/12"),
            None
        );
    }

    /// **The retry decision, which is the whole reason this error type exists.**
    ///
    /// Both strings are verbatim from real runs on 26/08/2026. The blocked one repeated
    /// identically on three consecutive attempts; the rehydration one cleared on the next.
    #[test]
    fn an_ip_block_is_not_retried_and_a_rehydration_failure_is() {
        let blocked = classify_lookup_error(
            "ERROR: [TikTok] 7668980232241728776: Your IP address is blocked from accessing this post",
        );
        assert_eq!(blocked, WebLookupError::Blocked);
        assert!(
            !blocked.is_retryable(),
            "retrying an IP block is pure latency"
        );

        let transient = classify_lookup_error(
            "ERROR: [TikTok] 7668616467855723783: Unable to extract universal data for rehydration",
        );
        assert!(
            transient.is_retryable(),
            "this one cleared on the next attempt, measured"
        );
    }

    /// An unsupported URL is the post's problem, not the network's.
    #[test]
    fn an_unsupported_url_is_not_retried() {
        let error =
            classify_lookup_error("ERROR: Unsupported URL: https://www.tiktok.com/@a/photo/1");
        assert!(matches!(error, WebLookupError::Unavailable(_)));
        assert!(!error.is_retryable());
    }

    /// A message nobody has catalogued gets its retry rather than being written off.
    #[test]
    fn an_unknown_failure_is_treated_as_worth_one_more_go() {
        assert!(classify_lookup_error("ERROR: something new").is_retryable());
    }

    /// A caption containing a brace does not truncate the object being extracted.
    ///
    /// This is the defect a naive depth counter has, and TikTok captions carry braces and
    /// quotes often enough that it would be found in production rather than here.
    #[test]
    fn the_brace_matcher_survives_braces_inside_strings() {
        let raw = r#"junk{"imagePost":{"title":"a } b \" c {","images":[]},"after":1}"#;
        let extracted = json_value_after(raw, "\"imagePost\"").expect("found");
        let value: serde_json::Value = serde_json::from_str(&extracted).expect("valid json");
        assert_eq!(
            value.get("title").and_then(|v| v.as_str()),
            Some("a } b \" c {")
        );
    }

    /// Slides come out in flip order, one per slide, with CDN alternates collapsed.
    ///
    /// The measured page lists the same picture on `p16-` and `p19-` hosts. Counting both
    /// would report an eight-slide post as sixteen.
    #[test]
    fn cdn_alternates_are_not_counted_as_extra_slides() {
        let page = r#"x"imagePost":{"images":[
            {"imageURL":{"urlList":["https://p16/a.jpeg","https://p19/a.jpeg"]},"imageWidth":1416},
            {"imageURL":{"urlList":["https://p16/b.jpeg","https://p19/b.jpeg"]},"imageWidth":1416}
        ]}"#;
        assert_eq!(
            parse_slide_urls(page),
            vec![
                "https://p16/a.jpeg".to_string(),
                "https://p16/b.jpeg".to_string()
            ]
        );
    }

    /// `/` is how the page writes every `/`, and it has to survive into the URL.
    #[test]
    fn escaped_solidus_in_the_page_decodes_into_a_usable_url() {
        let page =
            r#"x"imagePost":{"images":[{"imageURL":{"urlList":["https://p16.example/a.jpeg"]}}]}"#;
        assert_eq!(parse_slide_urls(page), vec!["https://p16.example/a.jpeg"]);
    }

    /// A video post has no `imagePost`, and that is not an error.
    #[test]
    fn a_page_without_an_image_post_yields_no_slides() {
        assert!(parse_slide_urls(r#"{"video":{"duration":52}}"#).is_empty());
    }

    /// The free "could this ever have a transcript" answer.
    #[test]
    fn the_original_audio_flag_is_read_from_the_page() {
        let page = r#"x"claInfo":{"hasOriginalAudio":false,"enableAutoCaption":true,"captionInfos":[],"noCaptionReason":3}"#;
        let (has_audio, tracks) = parse_audio_facts(page);
        assert_eq!(has_audio, Some(false));
        assert!(tracks.is_empty());
    }

    /// **The ASR track wins, whatever order the page lists them in.**
    ///
    /// This is the measured shape: the vlog offered `eng-US` first with `"Source":"MT"` and
    /// `vie-VN` second with `"Source":"ASR"`. Taking the first would hand the model a machine
    /// translation of the speech instead of the speech — one generation further from the
    /// audio, and free to lose a place name on the way.
    #[test]
    fn the_original_speech_track_is_preferred_over_its_translation() {
        let page = r#"x"subtitleInfos":[{"LanguageCodeName":"eng-US","Source":"MT","Url":"https://cdn/en.vtt"},{"LanguageCodeName":"vie-VN","Source":"ASR","Url":"https://cdn/vi.vtt"}]"#;
        let context = parse_post_context("{}", page);
        assert_eq!(context.subtitle_langs(), vec!["eng-US", "vie-VN"]);
        let track = context.transcript_track().expect("a track");
        assert_eq!(track.lang, "vie-VN");
        assert_eq!(track.url, "https://cdn/vi.vtt");
    }

    /// A track with no URL is not a track — there is nothing to fetch.
    #[test]
    fn a_subtitle_entry_without_a_url_is_dropped() {
        let page = r#"x"subtitleInfos":[{"LanguageCodeName":"vie-VN","Source":"ASR"}]"#;
        assert!(parse_post_context("{}", page).subtitles.is_empty());
    }

    /// **`hasOriginalAudio: false` closes the door before a request is spent.**
    ///
    /// Measured on this farm's only video target: a music-track post reported exactly that,
    /// with `captionInfos: []` and `noCaptionReason: 3`. Even if a track were somehow listed,
    /// there is no speech behind it.
    #[test]
    fn a_music_track_post_is_never_asked_for_a_transcript() {
        let page = r#"x"claInfo":{"hasOriginalAudio":false},"subtitleInfos":[{"LanguageCodeName":"vie-VN","Source":"ASR","Url":"https://cdn/vi.vtt"}]"#;
        let context = parse_post_context("{}", page);
        assert!(!context.could_have_transcript());
    }

    /// A page that says nothing about the audio is still worth asking, if it lists a track.
    ///
    /// `None` is "not measured", which is not the same as "no speech" — refusing on it would
    /// turn a shape change in the page into a silently thinner comment.
    #[test]
    fn an_unstated_audio_flag_does_not_block_a_listed_track() {
        let page = r#"x"subtitleInfos":[{"LanguageCodeName":"vie-VN","Source":"ASR","Url":"https://cdn/vi.vtt"}]"#;
        assert!(parse_post_context("{}", page).could_have_transcript());
    }

    /// **Timings out, sentences back together.**
    ///
    /// The measured track splits continuous speech mid-sentence (`ăn một` / `tô là đủ nạp đầy
    /// năng lượng`), so the cues only read as language once they are rejoined.
    #[test]
    fn a_vtt_track_becomes_one_line_of_speech() {
        let vtt = "WEBVTT\n\n\n00:00:07.920 --> 00:00:08.440\năn một\n\n00:00:08.441 --> 00:00:11.161\ntô là đủ nạp đầy năng lượng\n";
        assert_eq!(
            transcript_from_vtt(vtt, 240).as_deref(),
            Some("ăn một tô là đủ nạp đầy năng lượng")
        );
    }

    /// Numbered cues and repeated lines do not survive.
    #[test]
    fn cue_numbers_and_repeated_lines_are_dropped() {
        let vtt = "WEBVTT\n\n1\n00:00:00.000 --> 00:00:01.000\nsăn mây Cầu Đất\n\n2\n00:00:01.000 --> 00:00:02.000\nsăn mây Cầu Đất\n\n3\n00:00:02.000 --> 00:00:03.000\nchill thật\n";
        assert_eq!(
            transcript_from_vtt(vtt, 240).as_deref(),
            Some("săn mây Cầu Đất chill thật")
        );
    }

    /// **A truncated transcript says it is truncated.**
    ///
    /// A model told this is the narration would otherwise conclude the video ends where the
    /// text does — the same dishonesty a short contact sheet had before it started announcing
    /// its own length.
    #[test]
    fn a_transcript_past_the_cap_admits_it_was_cut() {
        let vtt = format!(
            "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\n{}\n",
            "từ ".repeat(50)
        );
        let text = transcript_from_vtt(&vtt, 10).expect("a transcript");
        let (said, marker) = text.split_once(" […").expect("the marker is appended");
        assert_eq!(
            said.split_whitespace().count(),
            10,
            "the cap is on words said"
        );
        assert!(marker.contains("phần sau chưa đọc"), "{text}");
    }

    /// An empty or timings-only track is nothing, not an empty string.
    #[test]
    fn a_track_with_no_speech_in_it_is_absent() {
        assert_eq!(transcript_from_vtt("WEBVTT\n\n", 240), None);
        assert_eq!(
            transcript_from_vtt("WEBVTT\n\n00:00:00.000 --> 00:00:01.000\n\n", 240),
            None
        );
    }

    /// Caption and duration come off the extractor's own JSON.
    #[test]
    fn caption_and_duration_come_from_the_extractor_json() {
        let info = r#"{"description":"  Một lịch trình vừa đủ chậm  ","duration":37.0}"#;
        let context = parse_post_context(info, "");
        assert_eq!(
            context.caption.as_deref(),
            Some("Một lịch trình vừa đủ chậm")
        );
        assert_eq!(context.duration_secs, Some(37));
        assert!(!context.is_empty());
    }

    /// An empty caption is `None`, not `Some("")`.
    ///
    /// The difference matters downstream: `Some("")` would be threaded into the prompt as a
    /// caption the model is told is authoritative, and an authoritative empty caption is a
    /// worse answer than no caption at all.
    #[test]
    fn a_blank_caption_is_absent_rather_than_empty() {
        let context = parse_post_context(r#"{"description":"   "}"#, "");
        assert_eq!(context.caption, None);
        assert!(context.is_empty());
    }

    /// Unreadable output degrades to an empty context rather than to a panic.
    #[test]
    fn garbage_output_produces_an_empty_context() {
        assert_eq!(
            parse_post_context("not json", "not a page"),
            PostWebContext::default()
        );
    }

    /// **Everything fits: take everything, in order.**
    #[test]
    fn a_short_carousel_is_taken_whole() {
        assert_eq!(pick_slide_indices(2, 4), vec![0, 1]);
        assert_eq!(pick_slide_indices(4, 4), vec![0, 1, 2, 3]);
    }

    /// **The first and the last are always in.**
    ///
    /// These are this farm's real carousel lengths. The old walk saw indices 0..3 of all of
    /// them; the point of spreading is that slide 7 of an eight-slide post is where the
    /// summary lives.
    #[test]
    fn a_long_carousel_keeps_its_first_and_last_slide() {
        for total in [5usize, 7, 8, 12] {
            let picks = pick_slide_indices(total, 4);
            assert_eq!(picks.len(), 4, "total {total}");
            assert_eq!(picks[0], 0, "total {total} must open on slide 1");
            assert_eq!(
                *picks.last().expect("non-empty"),
                total - 1,
                "total {total} must include the last slide"
            );
            assert!(
                picks.windows(2).all(|pair| pair[0] < pair[1]),
                "flip order must survive: {picks:?}"
            );
        }
    }

    /// Degenerate asks do not panic and do not invent an index.
    #[test]
    fn degenerate_asks_are_answered_with_nothing_or_the_first() {
        assert!(pick_slide_indices(0, 4).is_empty());
        assert!(pick_slide_indices(8, 0).is_empty());
        assert_eq!(pick_slide_indices(8, 1), vec![0]);
    }

    /// **A missing last slide routes to the phone; a missing middle does not.**
    ///
    /// The contact-sheet prompt promises the model the first and last slide are present,
    /// because a carousel's payload lives on the last one. A failed last-slide download used
    /// to vanish into a compacted vector and the model wrote about the cover — a wrong-topic
    /// comment. Now a missing bookend hands back nothing, so the caller falls back to
    /// on-device capture; a missing middle keeps the set, since first and last still frame it.
    #[test]
    fn a_missing_bookend_slide_discards_the_web_set_but_a_missing_middle_does_not() {
        let s = |n: u8| Some(vec![n; MIN_SLIDE_BYTES]);
        // Complete set: kept.
        assert_eq!(
            slides_if_first_and_last_present(vec![s(1), s(2), s(3)]).len(),
            3
        );
        // Middle failed: first and last still frame the post, so keep both.
        assert_eq!(
            slides_if_first_and_last_present(vec![s(1), None, s(3)]).len(),
            2
        );
        // Last (payload) failed: discard the whole web set → device fallback.
        assert!(slides_if_first_and_last_present(vec![s(1), s(2), None]).is_empty());
        // First failed: same.
        assert!(slides_if_first_and_last_present(vec![None, s(2), s(3)]).is_empty());
        // Nothing downloaded: empty, as before.
        assert!(slides_if_first_and_last_present(vec![None, None]).is_empty());
        assert!(slides_if_first_and_last_present(vec![]).is_empty());
    }

    /// **Same link shares one single-flight lock; different links get their own.**
    ///
    /// This is what stops the memo from serialising unrelated posts: one lock per link, held
    /// only across that link's own lookup, so a slow post one never blocks post two. Same
    /// link returns the same lock (pointer-equal), which is what collapses twenty phones on
    /// one link to a single request.
    #[tokio::test]
    async fn the_single_flight_lock_is_per_link_not_global() {
        let a1 = inflight_lock("riviu-test://link-a").await;
        let a2 = inflight_lock("riviu-test://link-a").await;
        let b = inflight_lock("riviu-test://link-b").await;
        assert!(
            std::sync::Arc::ptr_eq(&a1, &a2),
            "the same link must reuse one lock so its callers single-flight"
        );
        assert!(
            !std::sync::Arc::ptr_eq(&a1, &b),
            "different links must NOT share a lock, or they serialise"
        );
        // A held lock on link A leaves link B free to be taken at once.
        let _held_a = a1.lock().await;
        assert!(
            b.try_lock().is_ok(),
            "link B's lookup must not wait on link A's"
        );
        let mut inflight = LOOKUP_INFLIGHT.lock().await;
        inflight.remove("riviu-test://link-a");
        inflight.remove("riviu-test://link-b");
    }

    /// A waiter holds its own `Arc` while the leader completes. Completion must still remove
    /// the map entry; otherwise every contended one-shot URL remains there for the process life.
    #[tokio::test]
    async fn a_contended_single_flight_entry_is_removed_after_completion() {
        let url = "https://www.tiktok.com/@cleanup/video/7668947001618320661";
        let normalized = normalize_for_ytdlp(url).expect("fixture URL");
        LOOKUP_MEMO.lock().await.remove(&normalized);
        LOOKUP_INFLIGHT.lock().await.remove(&normalized);

        let leader_lock = inflight_lock(&normalized).await;
        let leader_guard = leader_lock.lock().await;
        let waiter = tokio::spawn(async move { fetch_post_context(url).await });
        for _ in 0..100 {
            if std::sync::Arc::strong_count(&leader_lock) >= 3 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            std::sync::Arc::strong_count(&leader_lock) >= 3,
            "the waiter must be queued on the same per-key lock"
        );

        let memo_result = Err(WebLookupError::Unavailable("completion fixture".into()));
        LOOKUP_MEMO.lock().await.insert(
            normalized.clone(),
            MemoEntry {
                at: std::time::Instant::now(),
                result: memo_result.clone(),
            },
        );
        drop(leader_guard);
        let observed = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("waiter completes")
            .expect("waiter task");
        assert_eq!(observed, memo_result);
        assert!(
            !LOOKUP_INFLIGHT.lock().await.contains_key(&normalized),
            "completion must remove the one-shot entry even while a waiter held an Arc"
        );

        LOOKUP_MEMO.lock().await.remove(&normalized);
        LOOKUP_INFLIGHT.lock().await.remove(&normalized);
    }
}
