use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use riviu_core::db::Database;
use riviu_core::{
    DeviceControlPlane, DeviceWorkOwner, FrameSource, NurtureEngine, NurtureSessionStatus,
    NurtureSettings, SessionLogBook, SessionLogEntry, SessionLogSummary,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::command_error::CommandError;
use crate::state::AppState;

fn err(e: impl std::fmt::Display) -> CommandError {
    CommandError::operation(e)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NurtureApiTestResult {
    pub udid: String,
    pub comment: String,
    pub caption: Option<String>,
    pub context_confidence: u8,
    pub relevance: u8,
    pub evidence_support: u8,
    pub frame_sha256: String,
    pub model: String,
    pub base_url_host: String,
    pub evidence_mode: String,
    /// How many *different* frames went into the picture the model read. The UI used to print
    /// a flat "3-frame vision" here, which is wrong on any still card: a photo post publishes
    /// byte-identical frames, so the three samples were one image three times.
    pub distinct_frames: u8,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

pub(crate) fn validate_nurture_settings(settings: &NurtureSettings) -> Result<(), String> {
    if !(1..=10_000).contains(&settings.num_videos) {
        return Err("num_videos phải nằm trong khoảng 1..=10000".into());
    }
    if !(1..=100).contains(&settings.num_rounds) {
        return Err("num_rounds phải nằm trong khoảng 1..=100".into());
    }
    for (label, value) in [
        ("like_prob", settings.like_prob),
        ("comment_prob", settings.comment_prob),
        ("save_prob", settings.save_prob),
        ("follow_prob", settings.follow_prob),
        ("frenzy_prob", settings.frenzy_prob),
    ] {
        if value > 100 {
            return Err(format!("{label} phải nằm trong khoảng 0..=100"));
        }
    }
    if !(4..=30).contains(&settings.max_comment_words) {
        return Err("max_comment_words phải nằm trong khoảng 4..=30".into());
    }
    if settings.comment_prob > 0 && settings.api_key.trim().is_empty() {
        return Err("Đã bật bình luận nhưng API key còn trống".into());
    }
    if settings.base_url.trim().is_empty() || settings.model.trim().is_empty() {
        return Err("Base URL và model AI không được để trống".into());
    }
    if !settings.watch_min.is_finite()
        || !settings.watch_max.is_finite()
        || settings.watch_min <= 0.0
        || settings.watch_max < settings.watch_min
        || settings.watch_max > 120.0
    {
        return Err("Khoảng thời gian xem video phải trong 0..120 giây và min <= max".into());
    }
    if !(15..=1_440).contains(&settings.schedule_every_minutes) {
        return Err("schedule_every_minutes phải nằm trong khoảng 15..=1440".into());
    }
    if !(15..=360).contains(&settings.schedule_duration_minutes) {
        return Err("schedule_duration_minutes phải nằm trong khoảng 15..=360".into());
    }
    // **A window is checked as hard as the panel, and its id is checked harder.**
    //
    // The id is not decoration: it becomes a settings key (`nurture.schedule.next_run_at.<id>`)
    // holding the mark for when that window is next due. An empty id would make two windows
    // share one mark and mute each other; a duplicate id does the same thing on purpose.
    let mut seen: Vec<&str> = Vec::new();
    for window in &settings.schedule_windows {
        let id = window.id.trim();
        if id.is_empty() {
            return Err("mỗi khung giờ phải có id".into());
        }
        if !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!(
                "id khung giờ {id:?} chỉ được gồm chữ, số, `-` và `_`"
            ));
        }
        if seen.contains(&id) {
            return Err(format!(
                "hai khung giờ dùng chung id {id:?}, nên chúng sẽ khoá lẫn nhau"
            ));
        }
        seen.push(id);
        // 1439 is 23:59. A start or end at 1440 would be a minute that no clock shows and
        // that `covers` could never match.
        if window.start_minute > 1_439 || window.end_minute > 1_439 {
            return Err("giờ bắt đầu và kết thúc của khung phải nằm trong một ngày".into());
        }
        if !(15..=1_440).contains(&window.every_minutes) {
            return Err(format!(
                "khung {id:?}: chu kỳ phải nằm trong khoảng 15..=1440 phút"
            ));
        }
        if !(15..=360).contains(&window.duration_minutes) {
            return Err(format!(
                "khung {id:?}: thời lượng phải nằm trong khoảng 15..=360 phút"
            ));
        }
        if let Some(behaviour) = &window.behaviour {
            if behaviour.num_videos == 0 || behaviour.num_videos > 10_000 {
                return Err(format!("khung {id:?}: số video phải trong 1..=10000"));
            }
            if behaviour.num_rounds == 0 || behaviour.num_rounds > 100 {
                return Err(format!("khung {id:?}: số vòng phải trong 1..=100"));
            }
            // Public actions are independent; each rate is bounded on its own.
            for (label, value) in [
                ("tỉ lệ tim", behaviour.like_prob),
                ("tỉ lệ bình luận", behaviour.comment_prob),
                ("tỉ lệ lưu", behaviour.save_prob),
                ("tỉ lệ follow", behaviour.follow_prob),
            ] {
                if value > 100 {
                    return Err(format!("khung {id:?}: {label} phải trong 0..=100"));
                }
            }
        }
    }
    Ok(())
}

/// What the operator typed to mean "leave the stored key alone".
///
/// The form has to show *something* in the API-key box, and it must not be the key: this value
/// crosses IPC into the WebView, and the panel is screenshotted constantly. So the command
/// hands back this sentinel instead, and treats it on the way back in as "unchanged". Any other
/// value — including empty — is taken literally, so clearing the key is still possible.
const API_KEY_UNCHANGED: &str = "__riviu_keep_stored_key__";

#[tauri::command]
pub fn nurture_get_settings(state: State<'_, AppState>) -> Result<NurtureSettings, CommandError> {
    let mut settings = state.db.get_nurture_settings().map_err(err)?;
    // The key never leaves the backend. `has_api_key` is what the form needs to know.
    settings.has_api_key = !settings.api_key.trim().is_empty();
    if settings.has_api_key {
        settings.api_key = API_KEY_UNCHANGED.to_string();
    }
    Ok(settings)
}

#[tauri::command]
pub fn nurture_save_settings(
    state: State<'_, AppState>,
    settings: NurtureSettings,
) -> Result<NurtureSettings, CommandError> {
    let mut settings = settings;
    let prev_for_key = state.db.get_nurture_settings().unwrap_or_default();
    if settings.api_key == API_KEY_UNCHANGED {
        settings.api_key = prev_for_key.api_key.clone();
    }
    let settings = settings;
    validate_nurture_settings(&settings)?;
    let _admission = state.ensure_accepting_work()?;
    let prev = prev_for_key;
    state.db.save_nurture_settings(&settings).map_err(err)?;
    // When schedule is (re)enabled, schedule the next tick from now.
    if settings.schedule_enabled
        && (!prev.schedule_enabled
            || prev.schedule_every_minutes != settings.schedule_every_minutes)
    {
        let every = settings.schedule_every_minutes.max(1) as i64;
        let next = (chrono::Utc::now() + chrono::Duration::minutes(every)).to_rfc3339();
        let _ = state.db.set_setting("nurture.schedule.next_run_at", &next);
    }
    if !settings.schedule_enabled {
        let _ = state.db.set_setting("nurture.schedule.next_run_at", "");
    }
    let _ = state.db.log_op("nurture.settings", &settings.model);
    // Answer with the same shape `nurture_get_settings` returns, so a save does not hand the
    // key back to the page that just stopped receiving it.
    let mut echoed = settings;
    echoed.has_api_key = !echoed.api_key.trim().is_empty();
    if echoed.has_api_key {
        echoed.api_key = API_KEY_UNCHANGED.to_string();
    }
    Ok(echoed)
}

/// Whether a byte string starts with the JPEG SOI marker.
///
/// The only check applied to caller-supplied frames, and enough: these bytes go straight
/// to a vision endpoint as `image/jpeg`, so what matters is that they are one. Same test
/// `save_view_snapshot` applies to the same source.
fn looks_like_jpeg(bytes: &[u8]) -> bool {
    bytes.len() > 3 && bytes[0] == 0xff && bytes[1] == 0xd8
}

/// The caller-supplied frames that may be used as evidence, at most three.
///
/// Separated from the command so the decision can be tested without an `AppState`. Anything
/// that is not a JPEG is dropped rather than refused: the WebView produces these from a
/// canvas, and a device whose canvas has not painted yet yields something unusable rather
/// than something malicious. Dropping it lands the caller in the hub fallback, which is the
/// same place it would have been without this parameter at all.
fn usable_supplied_frames(frames: Option<Vec<Vec<u8>>>) -> Vec<Vec<u8>> {
    frames
        .unwrap_or_default()
        .into_iter()
        .filter(|frame| looks_like_jpeg(frame))
        .take(3)
        .collect()
}

/// Run the same grounded vision pipeline as production comment preparation,
/// but stop after returning the prepared text. No device UI or comment sender
/// is opened by this command.
///
/// `frames` is how an Android phone gets here at all. The grid and the overlay stopped
/// showing minicap JPEGs when the H.264 view path landed, so `state.streams` — which this
/// command was reading — is empty for a phone whose live picture the operator is looking
/// at right now. Pressing Test API answered "Chưa có frame stream cho thiết bị …", which
/// was true about the hub and false about the phone.
///
/// So the caller may hand in the frames it already has decoded, which is exactly the
/// picture the button promises to test ("frame hiện tại"). No platform branch is needed on
/// either side: the WebView produces these only for devices it is decoding, and everything
/// else falls through to the hub as before.
#[tauri::command]
pub async fn nurture_test_api(
    state: State<'_, AppState>,
    udid: String,
    frames: Option<Vec<Vec<u8>>>,
) -> Result<NurtureApiTestResult, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let udid = udid.trim().to_string();
    if udid.is_empty() {
        return Err("Chọn một thiết bị để test API".into());
    }
    let settings = state.db.get_nurture_settings().map_err(err)?;
    if settings.api_key.trim().is_empty() {
        return Err("API key đang trống — lưu Cấu hình AI trước khi test".into());
    }
    if settings.base_url.trim().is_empty() || settings.model.trim().is_empty() {
        return Err("Base URL và model AI không được để trống".into());
    }

    let mut frames = usable_supplied_frames(frames);
    if frames.is_empty() {
        if let Some(frame) = state.streams.latest(&udid) {
            frames.push(frame.as_ref().clone());
        }
        let mut stream = FrameSource::subscribe(&state.streams, &udid);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
        while frames.len() < 3 {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, stream.next()).await {
                Ok(Some(frame)) => frames.push(frame.as_ref().clone()),
                Ok(None) | Err(_) => break,
            }
        }
    }
    if frames.is_empty() {
        return Err(err(format!(
            "Chưa có hình nào của thiết bị {udid} để test — mở stream của máy này rồi thử lại"
        )));
    }

    let direction = settings
        .ai_directions
        .split('|')
        .map(str::trim)
        .find(|value| !value.is_empty());
    let (result, evidence_mode) = riviu_core::openai_client::prepare_comment_for_frames(
        &settings,
        &frames,
        // This command re-runs one nurture card's evidence by hand, and that evidence is
        // sampled from a single card — the same moments the pixel engine collects.
        riviu_core::openai_client::EvidenceKind::Moments,
        direction,
        &crate::interaction_ocr::DesktopFrameTextSource,
        // This re-runs one nurture card, and that card was met by scrolling: no link, so
        // nothing to look a caption up by.
        Default::default(),
    )
    .await
    .map_err(err)?;

    Ok(NurtureApiTestResult {
        udid,
        comment: result.text,
        caption: result
            .caption
            .map(|caption| caption.chars().take(240).collect()),
        context_confidence: result.context_confidence,
        relevance: result.relevance,
        evidence_support: result.evidence_support,
        frame_sha256: result.frame_sha256,
        model: result.model,
        base_url_host: result.base_url_host,
        evidence_mode: evidence_mode.into(),
        distinct_frames: result.distinct_frames,
        prompt_tokens: result.prompt_tokens,
        completion_tokens: result.completion_tokens,
    })
}

#[tauri::command]
pub fn nurture_list_comment_attempts(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<riviu_core::NurtureCommentAttempt>, CommandError> {
    state
        .db
        .list_nurture_comment_attempts(limit.unwrap_or(100))
        .map_err(err)
}

#[tauri::command]
pub fn nurture_cost_summary(
    state: State<'_, AppState>,
) -> Result<riviu_core::NurtureCostSummary, CommandError> {
    state.db.nurture_cost_summary().map_err(err)
}

#[tauri::command]
pub fn nurture_session_status(
    state: State<'_, AppState>,
) -> Result<Vec<NurtureSessionStatus>, CommandError> {
    Ok(state.nurture.list_status())
}

/// One device's log, oldest line first.
///
/// Separate from `nurture_session_status` on purpose: the status list is polled and
/// pushed for every device continuously, and hanging a two-hundred-line history off each
/// row would multiply that traffic by the number of phones for a panel that shows one at
/// a time. This is fetched when a row is opened.
#[tauri::command]
pub fn nurture_session_log(
    state: State<'_, AppState>,
    udid: String,
) -> Result<Vec<SessionLogEntry>, CommandError> {
    Ok(state.nurture.log().entries(&udid))
}

/// Which phones have any history, and the last thing each one said.
///
/// The panel's rows used to be the live nurture statuses and nothing else. The idle sweep
/// produces neither a session nor a status, so a phone it had just unstuck had a full
/// history and nowhere to open it from — this is what gives it a row.
#[tauri::command]
pub fn nurture_session_log_summary(
    state: State<'_, AppState>,
) -> Result<Vec<SessionLogSummary>, CommandError> {
    Ok(state.nurture.log().summaries())
}

/// Throw away one device's history.
#[tauri::command]
pub fn nurture_clear_session_log(
    state: State<'_, AppState>,
    udid: String,
) -> Result<(), CommandError> {
    state.nurture.log().clear(&udid);
    Ok(())
}

#[tauri::command]
pub async fn nurture_start(
    app: AppHandle,
    state: State<'_, AppState>,
    udids: Vec<String>,
    duration_minutes: Option<u32>,
) -> Result<Vec<String>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if udids.is_empty() {
        return Err("Chưa chọn thiết bị".into());
    }
    let settings = state
        .db
        .get_nurture_settings()
        .map_err(CommandError::operation)?;
    validate_nurture_settings(&settings).map_err(CommandError::operation)?;
    let preflight = preflight_comment_job(&state.control, &udids, &settings).await;
    if preflight.ready.is_empty() {
        return Err(CommandError::operation(preflight.refusal()));
    }
    if !preflight.skipped.is_empty() {
        // Named in the log rather than swallowed. The command's answer is the list of
        // phones that started, so the caller can already see the shortfall; this is what
        // says which ones and why.
        log::warn!(
            "nuôi TT bỏ qua {} máy: {}",
            preflight.skipped.len(),
            preflight.skipped.join("; ")
        );
    }
    let udids = preflight.ready;
    // Manual starts get a varied 2–3 hour horizon so they do not all end on
    // the same fixed video count. Scheduled starts keep their explicit value.
    let run_duration = duration_minutes
        .map(|m| Duration::from_secs(m as u64 * 60))
        .or_else(|| {
            let jitter = chrono::Utc::now().timestamp_subsec_nanos() % 61;
            Some(Duration::from_secs((120 + jitter) as u64 * 60))
        });
    let started = state
        .nurture
        .start_many(
            app,
            state.nurture_engine.clone(),
            udids,
            settings,
            run_duration,
        )
        .await
        .map_err(err)?;
    Ok(started)
}

/// Which phones can take a text comment, and why the others cannot.
#[derive(Debug, Default, Clone)]
pub(crate) struct CommentPreflight {
    pub(crate) ready: Vec<String>,
    /// `udid: reason`, one line each.
    pub(crate) skipped: Vec<String>,
}

impl CommentPreflight {
    pub(crate) fn refusal(&self) -> String {
        format!(
            "Riviu Agent chưa sẵn sàng cho bình luận chữ: {}. Chạy Agent Repair rồi thử lại.",
            self.skipped.join("; ")
        )
    }
}

/// Check every phone, and let each one's answer be its own.
///
/// **One busy phone used to end the whole start.** Acquiring the lease was a `?`, so a
/// device already held by a job, a flow or the control overlay aborted the preflight before
/// the phones after it were even looked at — and the error the operator got was about a
/// lease, not about an agent. Twenty phones, one of them busy, nothing starts.
///
/// Same shape as the fix for `group_input`: record the failure, keep going, and let the
/// caller decide what a partial result means. A phone that cannot take a comment is a
/// reason to leave *that* phone out, not to cancel the other nineteen.
pub(crate) async fn preflight_comment_job(
    control: &DeviceControlPlane,
    udids: &[String],
    settings: &NurtureSettings,
) -> CommentPreflight {
    if settings.comment_prob == 0 {
        // Comments are off, so no phone needs an agent for them. Every device is eligible
        // and nothing is probed -- taking a lease per phone to answer a question nobody
        // asked would be its own way of blocking a start.
        return CommentPreflight {
            ready: udids.to_vec(),
            skipped: Vec::new(),
        };
    }

    let mut preflight = CommentPreflight::default();
    for udid in udids {
        let context = match control
            .try_acquire_exclusive(udid, DeviceWorkOwner::Nurture)
            .await
        {
            Ok(context) => context,
            Err(error) => {
                preflight.skipped.push(format!("{udid}: {error}"));
                continue;
            }
        };
        match control.preflight_agent(&context).await {
            Ok(status) if status.auth_ready => preflight.ready.push(udid.clone()),
            Ok(status) => preflight.skipped.push(format!(
                "{udid}: {}",
                status
                    .message
                    .unwrap_or_else(|| format!("trạng thái {:?}", status.state))
            )),
            Err(error) => preflight.skipped.push(format!("{udid}: {error}")),
        }
    }
    preflight
}

#[tauri::command]
pub fn nurture_stop(state: State<'_, AppState>, udids: Vec<String>) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if udids.is_empty() {
        state.nurture.stop_all();
    } else {
        for u in udids {
            state.nurture.stop(&u);
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct NurtureRuntime {
    inner: Arc<NurtureRuntimeInner>,
}

struct NurtureRuntimeInner {
    runs: Mutex<NurtureRuns>,
    status: Mutex<HashMap<String, NurtureSessionStatus>>,
    database: Option<Arc<Database>>,
    /// Every line these sessions ever said, per device.
    ///
    /// Shared with the idle sweeper, which writes into the same book — from the operator's
    /// side "what has this phone been doing" is one question, and answering it from two
    /// places would mean two panels showing two halves of the story.
    log: SessionLogBook,
}

struct NurtureRuns {
    accepting_starts: bool,
    stops: HashMap<String, Arc<AtomicBool>>,
}

impl NurtureRuntime {
    pub fn new() -> Self {
        Self::build(None)
    }

    pub fn with_database(database: Arc<Database>) -> Self {
        Self::build(Some(database))
    }

    fn build(database: Option<Arc<Database>>) -> Self {
        Self {
            inner: Arc::new(NurtureRuntimeInner {
                runs: Mutex::new(NurtureRuns {
                    accepting_starts: true,
                    stops: HashMap::new(),
                }),
                status: Mutex::new(HashMap::new()),
                database,
                log: SessionLogBook::new(),
            }),
        }
    }

    /// The shared log book, for the idle sweeper and the command that reads it.
    pub fn log(&self) -> SessionLogBook {
        self.inner.log.clone()
    }

    pub fn list_status(&self) -> Vec<NurtureSessionStatus> {
        self.inner.status.lock().values().cloned().collect()
    }

    /// Record a status change, and keep what it said.
    ///
    /// The log is written here rather than at the call sites because *every* status —
    /// the queued one, each update from the engine, the final one and the error one —
    /// already funnels through this method. A second write anywhere else would be the
    /// line that goes missing when somebody adds a third call site.
    pub fn set_status(&self, mut st: NurtureSessionStatus) {
        st.updated_at = Some(chrono::Utc::now());
        if st.run_id.is_some() {
            if let Some(database) = &self.inner.database {
                if let Err(error) = database.append_nurture_status(&st) {
                    // A public-action worker must not continue indefinitely after its durable
                    // history disappeared. The current callback cannot return an error through
                    // NurtureEngine, so signal its existing stop token and keep the in-memory
                    // row visible while the last durable row is reconciled on restart.
                    log::error!("không ghi được trạng thái Nuôi cho {}: {error:#}", st.udid);
                    self.stop(&st.udid);
                }
            }
        }
        self.store_live_status(st);
    }

    fn store_live_status(&self, st: NurtureSessionStatus) {
        self.inner.log.record(&st.udid, &st.last_message);
        self.inner.status.lock().insert(st.udid.clone(), st);
    }

    pub fn stop(&self, udid: &str) {
        if let Some(flag) = self.inner.runs.lock().stops.get(udid) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    pub fn stop_all(&self) {
        for flag in self.inner.runs.lock().stops.values() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    pub fn begin_shutdown(&self) {
        let mut runs = self.inner.runs.lock();
        runs.accepting_starts = false;
        for flag in runs.stops.values() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn reserve_start(&self, udid: &str) -> Option<Arc<AtomicBool>> {
        let mut runs = self.inner.runs.lock();
        if !runs.accepting_starts || runs.stops.contains_key(udid) {
            return None;
        }
        let stop = Arc::new(AtomicBool::new(false));
        runs.stops.insert(udid.to_string(), stop.clone());
        Some(stop)
    }

    fn finish_start(&self, udid: &str, stop: &Arc<AtomicBool>) {
        let mut runs = self.inner.runs.lock();
        if runs
            .stops
            .get(udid)
            .is_some_and(|current| Arc::ptr_eq(current, stop))
        {
            runs.stops.remove(udid);
        }
    }

    async fn wait_stagger_or_stop(stop: &AtomicBool, duration: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            if stop.load(Ordering::Relaxed) {
                return true;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return false;
            }
            tokio::time::sleep((deadline - now).min(Duration::from_millis(100))).await;
        }
    }

    pub async fn start_many(
        &self,
        app: AppHandle,
        engine: NurtureEngine,
        udids: Vec<String>,
        settings: NurtureSettings,
        max_duration: Option<Duration>,
    ) -> anyhow::Result<Vec<String>> {
        // **The identity of this run, and the only place it exists.**
        //
        // `set_status` inserts by udid and nothing ever removes an entry, so the status list
        // accumulates every phone that has run since the process started. A fleet total
        // summed over it therefore already includes finished phones from earlier runs, and
        // restarting one phone makes an overall bar go *backwards* — that row's counters
        // reset to zero while the others keep their finished values. Stamping a run id here
        // is what lets a reader ask "this run" instead of "everything ever". Flow runs
        // already carry one for the same reason.
        //
        // `run_size` is the count that were *asked for*, not the count that started: a phone
        // `reserve_start` turns away still occupies a slot in the operator's mind, and a
        // denominator that shrank when a phone failed to start would report 100% on a run
        // that was two phones short.
        let run_id = uuid::Uuid::new_v4();
        let run_size = udids.len() as u32;
        anyhow::ensure!(run_size > 0, "nurture target list is empty");
        let now = chrono::Utc::now();
        let deadline_at = max_duration.and_then(|window| {
            chrono::Duration::from_std(window)
                .ok()
                .map(|window| now + window)
        });
        let reservations = udids
            .iter()
            .map(|udid| (udid.clone(), self.reserve_start(udid)))
            .collect::<Vec<_>>();
        let initial_statuses = reservations
            .iter()
            .map(|(udid, reservation)| {
                let mut status = NurtureSessionStatus {
                    running: reservation.is_some(),
                    last_message: if reservation.is_some() {
                        "queued".into()
                    } else {
                        "Không bắt đầu: thiết bị đang chạy một phiên Nuôi khác".into()
                    },
                    run_id: Some(run_id),
                    run_size,
                    phase: riviu_core::NurturePhase::Queued,
                    video_target: settings.num_videos.max(1) * settings.num_rounds.max(1),
                    deadline_at,
                    updated_at: Some(now),
                    ..NurtureSessionStatus::new(udid)
                };
                if reservation.is_none() {
                    status.finish(riviu_core::Outcome::Failed);
                }
                status
            })
            .collect::<Vec<_>>();
        if let Some(database) = &self.inner.database {
            if let Err(error) = database.create_nurture_run(run_id, &udids, &initial_statuses) {
                for (udid, reservation) in &reservations {
                    if let Some(stop) = reservation {
                        self.finish_start(udid, stop);
                    }
                }
                return Err(error.context("persist nurture run before worker dispatch"));
            }
        }
        for status in &initial_statuses {
            self.store_live_status(status.clone());
        }
        let mut started = Vec::new();
        for (idx, (udid, reservation)) in reservations.into_iter().enumerate() {
            let Some(stop) = reservation else {
                continue;
            };

            let runtime = self.clone();
            let engine = engine.clone();
            let settings = settings.clone();
            let app2 = app.clone();
            let udid_clone = udid.clone();
            let task_stop = stop.clone();
            let min = settings.stagger_delay_min.min(settings.stagger_delay_max);
            let max = settings.stagger_delay_max.max(settings.stagger_delay_min);
            let stagger = if idx == 0 {
                0
            } else if max > min {
                min + (idx as u32 % (max - min + 1))
            } else {
                min
            };

            tauri::async_runtime::spawn(async move {
                // **The run's identity is the batch's, not the session's, so it is stamped
                // here.** `run_session` takes a udid and knows nothing about the other
                // thirteen phones; asking it to carry a run id would put a fact about the
                // caller into a signature five harnesses also call. Stamping on the way past
                // means every status this device ever emits — queued, mid-run, terminal, and
                // the error path below — carries it, and there is exactly one place to get
                // it wrong.
                let tag = move |mut st: NurtureSessionStatus| {
                    st.run_id = Some(run_id);
                    st.run_size = run_size;
                    st
                };
                let stopped_before_start =
                    Self::wait_stagger_or_stop(&task_stop, Duration::from_secs(stagger as u64))
                        .await;
                let final_status = if stopped_before_start || task_stop.load(Ordering::Acquire) {
                    let mut status = NurtureSessionStatus {
                        last_message: "stopped before start".to_string(),
                        ..NurtureSessionStatus::new(&udid_clone)
                    };
                    status.finish(riviu_core::Outcome::Stopped);
                    status
                } else {
                    match engine
                        .run_session(
                            &udid_clone,
                            settings,
                            task_stop.clone(),
                            max_duration,
                            |st| {
                                let st = tag(st);
                                runtime.set_status(st.clone());
                                let _ = app2.emit(
                                    "riviu://event",
                                    riviu_core::AppEvent::NurtureStatus { status: st },
                                );
                            },
                        )
                        .await
                    {
                        Ok(mut status) => {
                            // `run_session` already went through `finish`, so the verdict is
                            // on it. Belt and braces for the paths that return a row the
                            // engine built before its own terminal handling: a row that
                            // reaches here still `running` has no verdict, and a bar drawn
                            // from it would sit at whatever fraction it died at forever.
                            if status.outcome.is_none() {
                                status.finish(riviu_core::Outcome::Partial);
                            }
                            status
                        }
                        Err(error) => {
                            let mut status = NurtureSessionStatus {
                                last_message: format!("error: {error}"),
                                ..NurtureSessionStatus::new(&udid_clone)
                            };
                            status.finish(riviu_core::Outcome::Failed);
                            status
                        }
                    }
                };
                let final_status = tag(final_status);
                runtime.set_status(final_status.clone());
                runtime.finish_start(&udid_clone, &task_stop);
                let _ = app2.emit(
                    "riviu://event",
                    riviu_core::AppEvent::NurtureStatus {
                        status: final_status.clone(),
                    },
                );
            });
            started.push(udid);
        }
        Ok(started)
    }
}

impl Default for NurtureRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riviu_core::{
        AgentState, DeviceControlPlane, DeviceDriver, DeviceWorkCoordinator, StreamBudgetManager,
    };
    use riviu_ios_driver::MockIosDriver;

    fn database_fixture() -> (Arc<Database>, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "riviu-desktop-nurture-history-{}.db",
            uuid::Uuid::new_v4()
        ));
        (
            Arc::new(Database::open(&path).expect("open fixture database")),
            path,
        )
    }

    #[test]
    fn concurrent_starts_reserve_exactly_one_stop_token_per_device() {
        let runtime = NurtureRuntime::new();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let runtime = runtime.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                runtime.reserve_start("same-device")
            }));
        }
        barrier.wait();
        let reservations = workers
            .into_iter()
            .map(|worker| worker.join().expect("reservation worker"))
            .collect::<Vec<_>>();

        assert_eq!(
            reservations.iter().filter(|value| value.is_some()).count(),
            1
        );
    }

    #[test]
    fn shutdown_atomically_blocks_new_starts_and_signals_existing_tokens() {
        let runtime = NurtureRuntime::new();
        let active = runtime.reserve_start("active").expect("active token");

        runtime.begin_shutdown();

        assert!(active.load(Ordering::Relaxed));
        assert!(runtime.reserve_start("late").is_none());
    }

    #[test]
    fn accepting_a_status_stamps_its_latest_observed_time() {
        let runtime = NurtureRuntime::new();
        let before = chrono::Utc::now();

        runtime.set_status(NurtureSessionStatus::new("phone-a"));

        let status = runtime
            .list_status()
            .into_iter()
            .next()
            .expect("stored status");
        let updated = status.updated_at.expect("latest status timestamp");
        assert!(updated >= before);
        assert!(updated <= chrono::Utc::now());
    }

    #[test]
    fn runtime_status_transition_is_appended_to_the_durable_run() {
        let (database, path) = database_fixture();
        let runtime = NurtureRuntime::with_database(database.clone());
        let run_id = uuid::Uuid::new_v4();
        let targets = vec!["phone-a".to_string()];
        let initial = NurtureSessionStatus {
            running: true,
            run_id: Some(run_id),
            run_size: 1,
            last_message: "queued".into(),
            updated_at: Some(chrono::Utc::now()),
            ..NurtureSessionStatus::new("phone-a")
        };
        database
            .create_nurture_run(run_id, &targets, std::slice::from_ref(&initial))
            .expect("create durable run");
        runtime.store_live_status(initial);

        let mut watching = runtime.list_status().remove(0);
        watching.phase = riviu_core::NurturePhase::Watching;
        watching.videos_done = 4;
        watching.last_message = "watching".into();
        runtime.set_status(watching);

        let restored = database
            .get_nurture_run(run_id)
            .expect("read run")
            .expect("run exists")
            .statuses
            .into_iter()
            .next()
            .expect("device status");
        assert_eq!(restored.videos_done, 4);
        assert_eq!(restored.phase, riviu_core::NurturePhase::Watching);

        drop(runtime);
        drop(database);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[tokio::test]
    async fn stop_interrupts_stagger_before_a_device_session_can_start() {
        let stop = Arc::new(AtomicBool::new(false));
        let waiter_stop = stop.clone();
        let waiter = tokio::spawn(async move {
            NurtureRuntime::wait_stagger_or_stop(&waiter_stop, Duration::from_secs(30)).await
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        stop.store(true, Ordering::Relaxed);

        assert!(tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("stagger stop should be observed promptly")
            .expect("stagger waiter"));
    }

    #[tokio::test]
    async fn comment_job_with_unready_agent_is_rejected_before_it_is_reported_started() {
        let driver = MockIosDriver::new();
        for udid in ["needs-repair-a", "needs-repair-b"] {
            let mut status = driver.cached_agent_status(udid);
            status.state = AgentState::RepairRequired;
            status.message = Some("agent version does not match manifest".to_string());
            driver.set_mock_agent_status(status);
        }
        let control = DeviceControlPlane::new(
            Arc::new(driver.clone()),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::default()),
        );
        let runtime = NurtureRuntime::new();
        let settings = NurtureSettings {
            comment_prob: 1,
            ..Default::default()
        };

        let preflight = preflight_comment_job(
            &control,
            &["needs-repair-a".to_string(), "needs-repair-b".to_string()],
            &settings,
        )
        .await;

        assert!(preflight.ready.is_empty());
        let refusal = preflight.refusal();
        assert!(refusal.contains("needs-repair-a"));
        assert!(refusal.contains("needs-repair-b"));
        assert!(refusal.contains("Agent Repair"));
        assert!(runtime.list_status().is_empty());
        assert_eq!(
            driver.agent_preflight_calls(),
            0,
            "comment preflight must use install-only readiness"
        );
    }

    #[tokio::test]
    async fn one_busy_phone_no_longer_cancels_the_start_for_every_other_phone() {
        // Taking the lease was a `?`, so a device already held by a job, a flow or the
        // control overlay aborted the preflight before the phones after it were even
        // looked at -- and the error named a lease, not an agent. Twenty phones, one busy,
        // nothing starts.
        let driver = MockIosDriver::new();
        let work = Arc::new(DeviceWorkCoordinator::new());
        let control = DeviceControlPlane::new(
            Arc::new(driver.clone()),
            work.clone(),
            Arc::new(StreamBudgetManager::default()),
        );
        // Held by something else, exactly as a running job would hold it.
        let _busy = work
            .try_acquire("MOCK-IPHONE-02", DeviceWorkOwner::Script)
            .expect("hold the busy device");
        let settings = NurtureSettings {
            comment_prob: 1,
            ..Default::default()
        };

        let preflight = preflight_comment_job(
            &control,
            &["MOCK-IPHONE-02".to_string(), "MOCK-IPHONE-01".to_string()],
            &settings,
        )
        .await;

        // The busy phone is skipped with its reason; the healthy one behind it -- which the
        // old code never reached -- is ready to start.
        assert_eq!(preflight.ready, vec!["MOCK-IPHONE-01".to_string()]);
        assert_eq!(preflight.skipped.len(), 1);
        assert!(preflight.skipped[0].starts_with("MOCK-IPHONE-02:"));
    }

    #[tokio::test]
    async fn with_comments_switched_off_no_phone_is_probed_or_excluded() {
        // Nothing needs a text agent, so nothing is asked for a lease. Probing anyway would
        // be its own way of letting one busy phone hold up a start.
        let driver = MockIosDriver::new();
        let control = DeviceControlPlane::new(
            Arc::new(driver.clone()),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::default()),
        );
        let settings = NurtureSettings {
            comment_prob: 0,
            ..Default::default()
        };

        let preflight =
            preflight_comment_job(&control, &["MOCK-IPHONE-01".to_string()], &settings).await;

        assert_eq!(preflight.ready, vec!["MOCK-IPHONE-01".to_string()]);
        assert!(preflight.skipped.is_empty());
    }

    /// The scheduled path must ask the same question the button asks.
    ///
    /// The tick in `state.rs` went straight to `start_many`, so a scheduled run began on
    /// phones whose text agent was not ready and then failed every comment it attempted,
    /// once an hour, with nothing written down. The manual start had refused those phones;
    /// the schedule did not know to ask. Both now call `preflight_comment_job`, and this
    /// pins the source-level fact that they do -- there is no seam a unit test can drive
    /// the spawned scheduler through.
    #[test]
    fn the_scheduled_start_goes_through_the_same_comment_gate_as_the_button() {
        let scheduler = include_str!("state.rs");
        let tick = scheduler
            .split("// TikTok nurture schedule ticks")
            .nth(1)
            .expect("the nurture schedule tick");
        // `.start_many(` with the dot, not the bare name: the comment above the gate
        // explains what the code used to do and mentions `start_many` by name, so matching
        // the bare word finds the prose rather than the call.
        let start = tick.find(".start_many(").expect("the scheduled start");
        let gate = tick
            .find("preflight_comment_job")
            .expect("the scheduled start skipped the comment gate");
        assert!(
            gate < start,
            "the gate has to run before the start, not after it"
        );
        assert!(
            tick.contains("preflight.ready"),
            "the scheduled start must run only the phones the gate admitted"
        );
    }
    #[test]
    fn default_nurture_settings_pass_validation() {
        assert!(validate_nurture_settings(&NurtureSettings::default()).is_ok());
    }

    #[test]
    fn independent_public_action_probabilities_can_all_be_hundred_percent() {
        let settings = NurtureSettings {
            like_prob: 100,
            comment_prob: 100,
            save_prob: 100,
            follow_prob: 100,
            api_key: "fixture".into(),
            ..NurtureSettings::default()
        };
        assert!(validate_nurture_settings(&settings).is_ok());
    }

    #[test]
    fn nurture_validation_rejects_unbounded_session_values() {
        let settings = NurtureSettings {
            num_videos: 10_001,
            ..NurtureSettings::default()
        };
        assert!(validate_nurture_settings(&settings)
            .expect_err("video ceiling must be bounded")
            .contains("num_videos"));

        let settings = NurtureSettings {
            schedule_duration_minutes: 10,
            ..NurtureSettings::default()
        };
        assert!(validate_nurture_settings(&settings)
            .expect_err("schedule burst must be human-sized")
            .contains("schedule_duration_minutes"));

        let settings = NurtureSettings {
            watch_max: 121.0,
            ..NurtureSettings::default()
        };
        assert!(validate_nurture_settings(&settings)
            .expect_err("watch duration must be bounded")
            .contains("thời gian xem"));

        let settings = NurtureSettings {
            save_prob: 101,
            ..NurtureSettings::default()
        };
        assert!(validate_nurture_settings(&settings)
            .expect_err("save probability must be bounded independently")
            .contains("save_prob"));
    }

    /// The smallest thing that is a JPEG to every reader that matters.
    fn jpeg(marker: u8) -> Vec<u8> {
        vec![0xff, 0xd8, 0xff, marker]
    }

    #[test]
    fn frames_the_caller_already_decoded_are_what_gets_tested() {
        // Test API read only `state.streams`, the host's JPEG hub. Android phones stopped
        // publishing there when the H.264 view path landed, so pressing the button while
        // watching a phone's live picture answered "no frames for this device" -- true
        // about the hub, false about the phone. These are the frames the WebView already
        // has, which is exactly what the button promises ("frame hiện tại").
        let supplied = usable_supplied_frames(Some(vec![jpeg(1), jpeg(2)]));
        assert_eq!(supplied, vec![jpeg(1), jpeg(2)]);
    }

    #[test]
    fn a_caller_that_supplies_nothing_usable_falls_through_to_the_hub() {
        // Absent, empty, and present-but-unusable all have to reach the same place: an
        // iPhone supplies nothing because its frames live in the hub, and a canvas that
        // has not painted yet yields bytes that are not an image. Neither is an error --
        // both leave the caller exactly where it was before this parameter existed.
        assert!(usable_supplied_frames(None).is_empty());
        assert!(usable_supplied_frames(Some(Vec::new())).is_empty());
        assert!(
            usable_supplied_frames(Some(vec![Vec::new(), b"not-an-image".to_vec()])).is_empty()
        );
        // A PNG is a real image and still not one this path may send: the request declares
        // `image/jpeg`, so a mislabelled body is a provider error nobody could diagnose.
        let png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        assert!(usable_supplied_frames(Some(vec![png])).is_empty());
    }

    #[test]
    fn no_more_than_three_frames_are_ever_sent() {
        // The grounded pipeline reads three. A caller sending thirty would be billed for
        // thirty, silently, on a button whose whole purpose is to show what one costs.
        let many: Vec<Vec<u8>> = (0..30).map(|index| jpeg(index as u8)).collect();
        assert_eq!(usable_supplied_frames(Some(many)).len(), 3);
    }

    /// The sentinel is a contract between this file and the settings form, so it is pinned.
    ///
    /// If it ever equalled a plausible key, "leave it alone" would silently swallow a real one
    /// the operator had just typed; if the frontend's copy drifted from this one, every save
    /// would overwrite the stored key with the literal sentinel string.
    #[test]
    fn the_unchanged_sentinel_cannot_be_mistaken_for_a_key() {
        assert_eq!(API_KEY_UNCHANGED, "__riviu_keep_stored_key__");
        // Not something an API key could plausibly be: no provider issues keys with this shape.
        assert!(API_KEY_UNCHANGED.starts_with("__"));
        assert!(!API_KEY_UNCHANGED.starts_with("sk-"));
        // And the frontend must agree, byte for byte — a drifted copy would write the sentinel
        // into the credential store as if it were the key.
        let types_ts = include_str!("../../src/types.ts");
        assert!(
            types_ts.contains(API_KEY_UNCHANGED),
            "apps/desktop/src/types.ts no longer documents the sentinel {API_KEY_UNCHANGED}"
        );
    }

    /// The tag serde writes for one event, named beside the variant that produces it.
    ///
    /// Total on purpose: this is the half the compiler enforces. A variant added to
    /// `AppEvent` does not compile until it is listed here, which is what puts the frontend
    /// union in front of whoever adds it.
    fn tag_of(event: &riviu_core::AppEvent) -> &'static str {
        use riviu_core::AppEvent as E;
        match event {
            E::DevicesUpdated { .. } => "devicesUpdated",
            E::DeviceUpdated { .. } => "deviceUpdated",
            E::JobUpdated { .. } => "jobUpdated",
            E::FlowUpdated { .. } => "flowUpdated",
            E::FlowRunUpdated { .. } => "flowRunUpdated",
            E::InteractionUpdated { .. } => "interactionUpdated",
            E::PublishUpdated { .. } => "publishUpdated",
            E::WdaExpiryWarning { .. } => "wdaExpiryWarning",
            E::NurtureStatus { .. } => "nurtureStatus",
        }
    }

    /// Every tag `tag_of` can return.
    ///
    /// Hand-listed, but not hand-trusted: `tag_of` is total, so the names here are the ones
    /// the compiler already made someone write next to the variant, and
    /// `the_tag_names_are_the_ones_serde_writes` checks the naming convention against real
    /// serialised output rather than against this list.
    const EVERY_EVENT_TAG: [&str; 9] = [
        "devicesUpdated",
        "deviceUpdated",
        "jobUpdated",
        "flowUpdated",
        "flowRunUpdated",
        "interactionUpdated",
        "publishUpdated",
        "wdaExpiryWarning",
        "nurtureStatus",
    ];

    #[test]
    fn the_tag_names_are_the_ones_serde_writes() {
        // `tag_of` is only worth anything if its strings are what actually goes on the wire.
        // Four variants are cheap to build and that is enough: `rename_all` is a container
        // attribute, so the convention is on for every variant or for none.
        let samples = [
            riviu_core::AppEvent::FlowUpdated {
                flow_id: uuid::Uuid::nil(),
                revision: 1,
            },
            riviu_core::AppEvent::FlowRunUpdated {
                run_id: uuid::Uuid::nil(),
                revision: 1,
            },
            riviu_core::AppEvent::InteractionUpdated {
                campaign_id: String::new(),
                revision: 1,
            },
            riviu_core::AppEvent::WdaExpiryWarning {
                udid: String::new(),
                days_remaining: 0,
            },
        ];
        for event in &samples {
            let json = serde_json::to_value(event).expect("AppEvent serialises");
            let wire = json
                .get("type")
                .and_then(serde_json::Value::as_str)
                .expect("every AppEvent carries a type tag");
            assert_eq!(wire, tag_of(event), "serde and tag_of disagree: {json}");
            assert!(
                EVERY_EVENT_TAG.contains(&wire),
                "{wire} is not in EVERY_EVENT_TAG"
            );
        }
    }

    #[test]
    fn a_struct_variant_reaches_the_frontend_in_the_case_the_frontend_reads() {
        // The bug this pins is not a missing tag but a field spelling, which no tag check can
        // see. `rename_all` on an enum renames variants only -- the fields of a struct variant
        // keep their Rust spelling unless `rename_all_fields` says otherwise, and without it
        // this enum was the one payload in the app sending snake_case. `FlowRunMonitor`,
        // `FlowWorkspace` and `InteractionPopup` had all been written against camelCase and so
        // none of their guards ever matched. Nothing failed loudly; the run monitor just
        // looked slow, because a 750 ms poll was doing all the work.
        let json = serde_json::to_value(riviu_core::AppEvent::FlowRunUpdated {
            run_id: uuid::Uuid::nil(),
            revision: 4,
        })
        .expect("serialises");
        assert!(
            json.get("runId").is_some(),
            "FlowRunMonitor reads `runId`; the wire says {json}"
        );
        assert!(json.get("run_id").is_none(), "both spellings on the wire");
    }

    #[test]
    fn the_event_union_matches_the_variants_this_enum_sends() {
        // `types.ts` carries a hand-written union for this channel, and it is pinned both
        // ways: a tag the backend sends and the union omits is an event no subscriber can
        // ever see, and a tag the union lists and nothing sends is a dead branch that reads
        // like live code.
        let types_ts = include_str!("../../src/types.ts");
        let declared: Vec<String> = types_ts
            .lines()
            .skip_while(|line| !line.contains("export const APP_EVENT_TYPES"))
            .skip(1)
            .take_while(|line| !line.contains("] as const"))
            .filter_map(|line| {
                let t = line.trim().trim_end_matches(',');
                t.strip_prefix('"')?.strip_suffix('"').map(str::to_owned)
            })
            .collect();
        assert!(
            !declared.is_empty(),
            "types.ts no longer declares APP_EVENT_TYPES in a shape this test can read"
        );

        for tag in EVERY_EVENT_TAG {
            assert!(
                declared.iter().any(|d| d == tag),
                "the backend sends `{tag}` and the frontend union does not list it"
            );
        }
        for tag in &declared {
            assert!(
                EVERY_EVENT_TAG.contains(&tag.as_str()),
                "the frontend union lists `{tag}` and nothing sends it"
            );
        }

        // And the union must spell the payload fields the way the wire does, not just the
        // tags -- that was the actual defect, and it lived under a correct tag.
        assert!(
            types_ts.contains("runId: string")
                && types_ts.contains("flowId: string")
                && types_ts.contains("campaignId: string"),
            "the union stopped using the camelCase field names the wire sends"
        );
    }
}
