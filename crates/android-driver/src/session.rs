//! `UiSession` over the resident agent.

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use riviu_core::driver::UiSession;
use riviu_core::flow::model::{ElementLocatorStrategy, QualifiedElementLocator};
use riviu_core::{HardwareKey, SwipeGesture, TapPoint};

use crate::adb::AdbProgram;
use crate::agent::{AgentClient, Locator};
use crate::riviu_agent::HelperClient;

/// `KEYCODE_HOME`.
const KEYCODE_HOME: i64 = 3;
/// `KEYCODE_BACK`.
const KEYCODE_BACK: i64 = 4;
/// `KEYCODE_VOLUME_UP`.
const KEYCODE_VOLUME_UP: i64 = 24;
/// `KEYCODE_VOLUME_DOWN`.
const KEYCODE_VOLUME_DOWN: i64 = 25;
/// `KEYCODE_POWER` — locks the screen; reboot stays a separate confirm.
const KEYCODE_POWER: i64 = 26;
/// `KEYCODE_NOTIFICATION`.
const KEYCODE_NOTIFICATION: i64 = 83;
/// `KEYCODE_APP_SWITCH` — Recents.
const KEYCODE_APP_SWITCH: i64 = 187;

pub(crate) fn hardware_keycode(key: HardwareKey) -> i64 {
    match key {
        HardwareKey::Home => KEYCODE_HOME,
        HardwareKey::Back => KEYCODE_BACK,
        HardwareKey::Recents => KEYCODE_APP_SWITCH,
        HardwareKey::VolumeUp => KEYCODE_VOLUME_UP,
        HardwareKey::VolumeDown => KEYCODE_VOLUME_DOWN,
        HardwareKey::Power => KEYCODE_POWER,
        HardwareKey::Notification => KEYCODE_NOTIFICATION,
    }
}

/// One serial's current screen size, shared by every session handed out for it.
///
/// A plain `(f64, f64)` field was wrong in a way that only shows up on a rotated phone: it
/// was captured once from `wm size` when the session opened and never refreshed, so every
/// coordinate scaled after a rotation went to the wrong place — and silently, because a tap
/// that lands somewhere unintended looks exactly like a tap the app ignored.
///
/// **`wm size` cannot be the refresh source**, which is the load-bearing part of this design.
/// Measured 16/08/2026 on SM-G955F, screen turned to landscape with Settings in front:
///
/// ```text
/// wm size            Override size: 1080x2220   ->  Override size: 1080x2220   (unchanged)
/// dumpsys display    real 1080 x 2220           ->  real 2220 x 1080           (swapped)
/// ```
///
/// `wm size` reports the display's base/override configuration, which has no orientation in
/// it at all — the same reason `frames.rs` feeds that tuple to minicap as `real=WxH` while
/// passing rotation as a separate argument. Re-running it after a rotation returns the
/// identical numbers and fixes nothing. The refresh therefore goes through the agent's
/// `/window/current/size`, which is the current window and rides the already-open forward.
///
/// Shared rather than copied so that invalidating reaches sessions already handed out — the
/// same reason `AgentClient` memoises its session id per serial.
#[derive(Clone, Default)]
pub(crate) struct ScreenCache(std::sync::Arc<parking_lot::Mutex<Option<(f64, f64)>>>);

impl ScreenCache {
    pub(crate) fn seeded(size: (f64, f64)) -> Self {
        Self(std::sync::Arc::new(parking_lot::Mutex::new(Some(size))))
    }

    pub(crate) fn peek(&self) -> Option<(f64, f64)> {
        *self.0.lock()
    }

    pub(crate) fn store(&self, size: (f64, f64)) {
        *self.0.lock() = Some(size);
    }

    pub(crate) fn invalidate(&self) {
        *self.0.lock() = None;
    }

    /// Whether the frame being pointed at proves the cached size is stale.
    ///
    /// The overlay hands in the size of the frame the operator actually clicked on, so a
    /// landscape frame against a portrait cache is not a suspicion — it is a contradiction,
    /// and it needs no clock and no timeout to detect. That is what catches the rotations
    /// nobody asked us for: auto-rotate, a physical turn, an app that comes up landscape.
    ///
    /// A square or degenerate frame answers `false`, so this can never add a round trip to
    /// the ordinary path.
    pub(crate) fn contradicted_by(&self, image_w: f64, image_h: f64) -> bool {
        let Some((width, height)) = self.peek() else {
            return false;
        };
        if image_w <= 0.0 || image_h <= 0.0 || width <= 0.0 || height <= 0.0 {
            return false;
        }
        let frame_landscape = image_w > image_h;
        let cache_landscape = width > height;
        image_w != image_h && width != height && frame_landscape != cache_landscape
    }
}

pub struct AndroidUiSession {
    agent: AgentClient,
    adb: AdbProgram,
    serial: String,
    /// The live screen size, refreshed rather than captured once. See [`ScreenCache`].
    screen: ScreenCache,
    /// Present only when `com.riviu.agent` answered `/status`. Missing means
    /// clipboard stays the trait default (`unsupported`) — never the empty
    /// uiautomator2 body.
    helper: Option<HelperClient>,
}

impl AndroidUiSession {
    pub fn new(agent: AgentClient, adb: AdbProgram, serial: String, screen: (f64, f64)) -> Self {
        Self {
            agent,
            adb,
            serial,
            screen: ScreenCache::seeded(screen),
            helper: None,
        }
    }

    /// Share the driver's per-serial cache instead of this session's private one, so an
    /// invalidation reaches a session that was handed out before the rotation.
    pub(crate) fn with_screen_cache(mut self, screen: ScreenCache) -> Self {
        self.screen = screen;
        self
    }

    pub(crate) fn with_helper(mut self, helper: Option<HelperClient>) -> Self {
        self.helper = helper;
        self
    }

    pub fn agent(&self) -> &AgentClient {
        &self.agent
    }

    /// Resolve a locator to its on-screen rectangle.
    ///
    /// This is the bridge the engine actually wants. Nurture does not want
    /// "press like", it wants *where* like is, so the existing touch-jitter
    /// planner can pick a human-looking point inside it.
    /// The UI language, for picking a measured TikTok label set.
    ///
    /// A device property rather than flow configuration, and it exists at all
    /// because label text is per-language: an English locator finds nothing on a
    /// Vietnamese UI (`riviu_core::tiktok_labels`).
    pub async fn ui_locale(&self) -> Option<String> {
        let property = self
            .adb
            .shell(&self.serial, "getprop persist.sys.locale")
            .await
            .unwrap_or_default();
        let setting = self
            .adb
            .shell(&self.serial, "settings get system system_locales")
            .await
            .unwrap_or_default();
        crate::adb::parse_locale(&property, &setting)
    }

    /// The installed `versionName` of a package.
    ///
    /// Keyed on the same reasoning as [`Self::ui_locale`]: some measured labels are
    /// unresolved resource ids, which move when the app is rebuilt
    /// (`riviu_core::tiktok_labels`). `dumpsys package` measured 1–2 s on this fleet,
    /// which is why callers read it once per session and not per locate.
    pub async fn app_version_name(&self, bundle_id: &str) -> Option<String> {
        // Same rule as everywhere else in this crate: this string reaches a real shell
        // on the phone, so it is validated as code rather than trusted as data.
        let package = crate::adb::validate_package_name(bundle_id).ok()?;
        let dumpsys = self
            .adb
            .shell(&self.serial, &format!("dumpsys package {package}"))
            .await
            .ok()?;
        riviu_core::tiktok_labels::parse_version_name(&dumpsys).map(str::to_string)
    }

    pub async fn find_bounds(
        &self,
        locator: &Locator,
    ) -> anyhow::Result<Option<crate::agent::Rect>> {
        let Some(element) = self.agent.find(locator).await? else {
            return Ok(None);
        };
        self.agent.rect(&element).await.map(Some)
    }

    /// The screen size to scale this frame against, refreshing it if it is missing or if the
    /// frame contradicts it.
    ///
    /// Resolved **once per gesture**, never per point: `/actions` takes a whole path in one
    /// round trip, and doing this inside the per-point closure would turn a fifty-point drag
    /// into fifty screen reads.
    ///
    /// A failed read falls back to the last known size rather than propagating. A stale
    /// scale still puts a finger on the phone; an `Err` loses the gesture outright, and for
    /// the nurture engine it aborts the whole session.
    async fn resolve_screen(&self, image_w: f64, image_h: f64) -> (f64, f64) {
        let cached = self.screen.peek();
        if let Some(size) = cached {
            if !self.screen.contradicted_by(image_w, image_h) {
                return size;
            }
        }
        match self.agent.window_size().await {
            Ok(size) => {
                self.screen.store(size);
                size
            }
            Err(error) => match cached {
                Some(size) => {
                    tracing::warn!(
                        serial = %self.serial,
                        %error,
                        "could not re-read the screen size; scaling against the last known one"
                    );
                    size
                }
                None => {
                    tracing::warn!(
                        serial = %self.serial,
                        %error,
                        "no screen size available; passing image coordinates through unscaled"
                    );
                    (image_w, image_h)
                }
            },
        }
    }
}

/// Scale a point given in image space onto the live screen.
///
/// A degenerate image size passes the point through rather than dividing by
/// zero: the caller already has a coordinate, and silently producing `NaN`
/// would tap somewhere unpredictable.
fn scale_to_screen(x: f64, y: f64, image_w: f64, image_h: f64, screen: (f64, f64)) -> (f64, f64) {
    if image_w <= 0.0 || image_h <= 0.0 {
        return (x, y);
    }
    (x * screen.0 / image_w, y * screen.1 / image_h)
}

/// Translate a core query into the agent's locator vocabulary.
fn to_agent_locator(query: riviu_core::ElementQuery<'_>) -> Locator {
    match query {
        riviu_core::ElementQuery::Description { value, exact: true } => {
            Locator::Description(value.to_string())
        }
        riviu_core::ElementQuery::Description {
            value,
            exact: false,
        } => Locator::DescriptionContains(value.to_string()),
        riviu_core::ElementQuery::ClassName(value) => Locator::ClassName(value.to_string()),
        riviu_core::ElementQuery::Text { value, exact: true } => Locator::Text(value.to_string()),
        riviu_core::ElementQuery::Text {
            value,
            exact: false,
        } => Locator::TextContains(value.to_string()),
    }
}

fn to_locator(locator: &QualifiedElementLocator) -> Locator {
    match locator.strategy {
        ElementLocatorStrategy::AccessibilityId => Locator::Description(locator.value.clone()),
        ElementLocatorStrategy::ClassName => Locator::ClassName(locator.value.clone()),
    }
}

#[async_trait]
impl UiSession for AndroidUiSession {
    async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
        self.agent.tap(point.x, point.y).await
    }

    async fn swipe(&self, gesture: SwipeGesture) -> anyhow::Result<()> {
        self.agent
            .swipe(
                (gesture.from.x, gesture.from.y),
                (gesture.to.x, gesture.to.y),
                gesture.duration_ms,
            )
            .await
    }

    /// The real thing: one `pointerMove` per leg, each with its own duration.
    ///
    /// This is why the path type exists. `swipe` sends a single move, which the framework
    /// receives as a straight line at constant speed; here the phone sees a curve whose
    /// velocity builds and eases, then a few milliseconds of contact before the lift.
    async fn swipe_path(&self, path: riviu_core::types::SwipePath) -> anyhow::Result<()> {
        self.agent.swipe_path(&path).await
    }

    /// Scale every point of the path, then send the whole curve in one round trip.
    ///
    /// One request, not one per step: the agent's `/actions` takes an arbitrary number of
    /// `pointerMove`s with individual durations, so a fifty-point drag costs the same trip
    /// as a two-point one. That is the whole reason the overlay can afford to send what the
    /// finger actually did instead of a straight line between the endpoints.
    async fn swipe_path_image(
        &self,
        path: riviu_core::types::SwipePath,
        image_w: f64,
        image_h: f64,
    ) -> anyhow::Result<()> {
        // Once for the whole path, before the closure. See `resolve_screen`.
        let screen = self.resolve_screen(image_w, image_h).await;
        let scale = |point: &riviu_core::types::TapPoint| {
            let (x, y) = scale_to_screen(point.x, point.y, image_w, image_h, screen);
            riviu_core::types::TapPoint { x, y }
        };
        let scaled = riviu_core::types::SwipePath {
            start: scale(&path.start),
            steps: path
                .steps
                .iter()
                .map(|step| riviu_core::types::SwipeStep {
                    point: scale(&step.point),
                    duration_ms: step.duration_ms,
                })
                .collect(),
            settle_ms: path.settle_ms,
        };
        self.agent.swipe_path(&scaled).await
    }

    async fn tap_image(&self, x: f64, y: f64, image_w: f64, image_h: f64) -> anyhow::Result<()> {
        let screen = self.resolve_screen(image_w, image_h).await;
        let (x, y) = scale_to_screen(x, y, image_w, image_h, screen);
        // Overlay / Open-on-Device: a 16 ms contact, no nurture drift.
        // `tap()` keeps the 45–130 ms human contact for the farm loop.
        self.agent.tap_direct(x, y).await
    }

    async fn swipe_image(
        &self,
        from: TapPoint,
        to: TapPoint,
        image_w: f64,
        image_h: f64,
        duration_ms: u64,
    ) -> anyhow::Result<()> {
        // One resolve for both endpoints -- they are the same gesture on the same screen.
        let screen = self.resolve_screen(image_w, image_h).await;
        let from = scale_to_screen(from.x, from.y, image_w, image_h, screen);
        let to = scale_to_screen(to.x, to.y, image_w, image_h, screen);
        self.agent.swipe(from, to, duration_ms).await
    }

    /// Type into whatever field currently holds focus.
    ///
    /// Goes through accessibility `ACTION_SET_TEXT`, which carries full
    /// Unicode: measured typing a fully accented Vietnamese sentence into
    /// TikTok's comment box with every diacritic intact, after which TikTok
    /// armed its own send button. `adb shell input text` cannot do this — with
    /// diacritics the process is killed outright.
    async fn type_text(&self, text: &str) -> anyhow::Result<()> {
        let locator = Locator::ClassName("android.widget.EditText".into()).focused();
        let element = self
            .agent
            .find(&locator)
            .await?
            .ok_or_else(|| anyhow!("no focused text field to type into"))?;
        self.agent.set_text(&element, text).await
    }

    async fn set_clipboard(&self, content_type: &str, bytes: &[u8]) -> anyhow::Result<()> {
        let helper = self
            .helper
            .as_ref()
            .ok_or_else(|| anyhow!(crate::riviu_agent::clipboard_unavailable(&self.serial)))?;
        helper.set_clipboard(content_type, bytes).await
    }

    async fn get_clipboard(
        &self,
        maximum_decoded_bytes: usize,
    ) -> anyhow::Result<(String, Vec<u8>)> {
        let helper = self
            .helper
            .as_ref()
            .ok_or_else(|| anyhow!(crate::riviu_agent::clipboard_unavailable(&self.serial)))?;
        helper.get_clipboard(maximum_decoded_bytes).await
    }

    /// True, and measured rather than assumed: the text is read back off the
    /// field before anything is sent.
    fn supports_text_input(&self) -> bool {
        true
    }

    async fn home(&self) -> anyhow::Result<()> {
        self.agent.press_key(KEYCODE_HOME).await
    }

    async fn back(&self) -> anyhow::Result<()> {
        self.agent.press_key(KEYCODE_BACK).await
    }

    async fn press_hardware_key(&self, key: HardwareKey) -> anyhow::Result<()> {
        self.agent.press_key(hardware_keycode(key)).await
    }

    /// Find by `content-desc`, then touch it like a finger would.
    ///
    /// Resolves bounds first and taps inside them instead of issuing an
    /// accessibility click, so the gesture goes through the same input path a
    /// person's does.
    async fn find_and_tap(&self, accessibility_id: &str) -> anyhow::Result<()> {
        let locator = Locator::Description(accessibility_id.to_string());
        let rect = self
            .find_bounds(&locator)
            .await?
            .ok_or_else(|| anyhow!("element '{accessibility_id}' is not on screen"))?;
        let (x, y) = rect.centre();
        self.agent.tap(x, y).await
    }

    async fn assert_visible(&self, accessibility_id: &str) -> anyhow::Result<()> {
        let locator = Locator::Description(accessibility_id.to_string());
        if self.agent.find(&locator).await?.is_some() {
            Ok(())
        } else {
            Err(anyhow!("element '{accessibility_id}' is not visible"))
        }
    }

    async fn healthy(&self) -> bool {
        self.agent.window_size().await.is_ok()
    }

    /// The cached size, read from the device only when there is nothing cached.
    ///
    /// Deliberately not a device call every time. Several callers treat this as free — the
    /// nurture planner reads it before each action, and `interaction_commands` reads it
    /// mid-run — and the G1 probe's "window_size 0 ms" is this cached read, not a round
    /// trip. `healthy()` exists to be the live probe.
    async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        if let Some(size) = self.screen.peek() {
            return Ok(size);
        }
        let size = self.agent.window_size().await?;
        self.screen.store(size);
        Ok(size)
    }

    async fn launch_app_foreground(&self, bundle_id: &str) -> anyhow::Result<()> {
        let bundle_id = crate::adb::validate_package_name(bundle_id)?;
        self.adb
            .shell(
                &self.serial,
                &format!("monkey -p {bundle_id} -c android.intent.category.LAUNCHER 1"),
            )
            .await
            .map(|_| ())
    }

    /// `am force-stop` and then launch, which on this platform is a real restart.
    ///
    /// The default would only re-raise a process that is already in front, and that is
    /// exactly the case this exists for: a TikTok whose feed has run dry shows the same
    /// card no matter how many times it is launched, and comes back with a new one after
    /// being stopped. Measured on ce051715081fe20f03, 18/08/2026.
    async fn restart_app(&self, bundle_id: &str) -> anyhow::Result<()> {
        let package = crate::adb::validate_package_name(bundle_id)?;
        // A force-stop on a package that is not running is not an error, so this needs no
        // check first.
        self.adb
            .shell(&self.serial, &format!("am force-stop {package}"))
            .await?;
        self.launch_app_foreground(bundle_id).await
    }

    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        // `dumpsys window windows` no longer carries `mCurrentFocus`: measured
        // empty on Android 15 (Redmi Note 12, HyperOS `OS2.0.207.0`) while it is
        // the line that does work on the Android 9 S8+ fleet. So ask both instead
        // of betting on one, and treat grep's non-zero exit for "no match" as an
        // answer rather than an error — that exit is what made the G1 probe fail
        // here with an empty message.
        // Measured on both phones in the fleet, 12/08/2026, and the split is total:
        //
        // | form                      | Note 8 / Android 8 | Redmi Note 12 / Android 15 |
        // |---------------------------|--------------------|----------------------------|
        // | `dumpsys window windows`  | works, 88–148 ms   | **empty, always**          |
        // | `dumpsys window displays` | **empty, always**  | works, 105–107 ms          |
        // | `dumpsys window`          | works, 84–97 ms    | works, 129–172 ms          |
        //
        // So the subcommand-free form is the only one that answers on both, and it costs
        // the same — the `grep` runs on the device, so one line comes back either way.
        // It goes first for that reason: with `windows` first, every call on the Android
        // 15 phone spent a wasted round trip (122–167 ms) before the one that works.
        // The other two stay as fallbacks rather than being deleted, because a phone
        // that answers only one of them is exactly what this list is for.
        const SOURCES: [&str; 3] = [
            "dumpsys window | grep mCurrentFocus",
            "dumpsys window windows | grep mCurrentFocus",
            "dumpsys window displays | grep mCurrentFocus",
        ];
        let mut tried: Vec<String> = Vec::new();
        for source in SOURCES {
            match self.adb.shell(&self.serial, source).await {
                Ok(stdout) => match crate::adb::parse_current_focus_package(&stdout) {
                    Some(package) => return Ok(package),
                    None => tried.push(format!("`{source}` had no mCurrentFocus line")),
                },
                Err(error) => tried.push(format!("`{source}` failed: {error}")),
            }
        }
        Err(anyhow!(
            "could not read the foreground package. Tried: {}",
            tried.join("; ")
        ))
    }

    /// Android has a first-class intent for this, so unlike the iOS side it
    /// needs no capability negotiation.
    async fn open_url(&self, url: &str) -> anyhow::Result<()> {
        self.adb
            .shell(
                &self.serial,
                &format!(
                    "am start -a android.intent.action.VIEW -d {}",
                    shell_quote(url)
                ),
            )
            .await
            .map(|_| ())
    }

    /// The same intent, but pinned to one app and carrying `BROWSABLE`.
    ///
    /// Measured on a Redmi Note 12, 11/08/2026, with `cmd package resolve-activity`:
    /// the intent [`Self::open_url`] builds resolves to
    /// `com.android.intentresolver.ResolverActivity` — the **app chooser** — because both
    /// TikTok and Chrome have `www.tiktok.com` verified. So a link opened that way reaches
    /// a dialog, not the post. Naming the package resolves to
    /// `com.ss.android.ugc.aweme.deeplink.AppLinkHandlerV2`.
    ///
    /// `BROWSABLE` is included because that is the category an app-link filter declares;
    /// without it the intent is matched only by the filter's implicit `DEFAULT`, which is
    /// how a chooser gets involved in the first place.
    ///
    /// The package is validated, not trusted: it is interpolated into a command a real
    /// shell on the phone runs, the same rule as everywhere else in this crate.
    async fn open_url_in_app(&self, url: &str, bundle_id: &str) -> anyhow::Result<()> {
        let package = crate::adb::validate_package_name(bundle_id)?;
        self.adb
            .shell(
                &self.serial,
                &format!(
                    "am start -a android.intent.action.VIEW \
                     -c android.intent.category.BROWSABLE -d {} -p {package}",
                    shell_quote(url)
                ),
            )
            .await
            .map(|_| ())
    }

    async fn read_text(
        &self,
        locator: &QualifiedElementLocator,
        _request_timeout: std::time::Duration,
    ) -> anyhow::Result<String> {
        let element = self.agent.require(&to_locator(locator)).await?;
        self.agent.text(&element).await
    }

    /// The first backend in this project that can honestly say yes. On iOS
    /// `snapshotMaxDepth` must stay at 1, so element reads are unavailable
    /// there by construction (AGENTS.md 2.3).
    fn supports_accessibility_readback(&self) -> bool {
        true
    }

    async fn screenshot_png(&self) -> anyhow::Result<Vec<u8>> {
        // Raw bytes, never text. A `String` round trip replaces every invalid
        // UTF-8 byte with U+FFFD and hands back something PNG-sized that is no
        // longer a PNG — caught by the G1 probe rather than by review.
        let png = self
            .adb
            .device_bytes(
                &self.serial,
                &["exec-out", "screencap", "-p"],
                std::time::Duration::from_secs(60),
            )
            .await
            .context("capture an Android screenshot")?;
        anyhow::ensure!(
            png.starts_with(&[0x89, b'P', b'N', b'G']),
            "screencap returned {} bytes that are not a PNG",
            png.len()
        );
        Ok(png)
    }

    /// Bounds, label, and enabled state, from one hierarchy query.
    ///
    /// The attribute read-backs are extra round trips rather than fields of the
    /// first response, because the agent's find reply carries only the element id.
    /// Both are still worth it: the comment control's own text is what tells two
    /// posts apart — that is what proves a swipe advanced — and the Send button's
    /// `enabled` flag is what proves the comment is armed.
    async fn locate(
        &self,
        query: riviu_core::ElementQuery<'_>,
    ) -> anyhow::Result<Option<riviu_core::ElementBox>> {
        let locator = to_agent_locator(query);
        let Some(element) = self.agent.find(&locator).await? else {
            return Ok(None);
        };
        let rect = self.agent.rect(&element).await?;
        // A missing label is not a failure: absent is a legitimate answer for an
        // element found by substring or by class, and the caller only needs it for
        // the fingerprint.
        let description = self
            .agent
            .attribute(&element, "content-desc")
            .await
            .ok()
            .flatten();
        // Default to enabled when the attribute cannot be read. The alternative —
        // defaulting to disabled — would report a live Send button as unarmed and
        // silently drop every comment.
        let enabled = self
            .agent
            .attribute(&element, "enabled")
            .await
            .ok()
            .flatten()
            .map(|value| value != "false")
            .unwrap_or(true);
        Ok(Some(riviu_core::ElementBox {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            description,
            enabled,
        }))
    }

    /// Bounds for **every** match, geometry only.
    ///
    /// Deliberately skips the `content-desc` and `enabled` read-backs that
    /// [`Self::locate`] performs. Those are two extra HTTP round trips *per element*,
    /// and this is called against a comment list: on a drawer with a dozen rows the
    /// attribute reads would dominate, and the caller that needs them
    /// (`interaction_hierarchy`, choosing a reply control by position) needs
    /// rectangles, not labels. `description` therefore comes back `None` here — ask
    /// [`Self::locate`] for a specific element when the label matters.
    async fn locate_all(
        &self,
        query: riviu_core::ElementQuery<'_>,
    ) -> anyhow::Result<Vec<riviu_core::ElementBox>> {
        let locator = to_agent_locator(query);
        let ids = self.agent.find_all(&locator).await?;
        let mut found = Vec::with_capacity(ids.len());
        for id in ids {
            // One stale element in a scrolling list must not lose the others: the
            // list can move between the find and the rect read.
            match self.agent.rect(&id).await {
                Ok(rect) => found.push(riviu_core::ElementBox {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                    description: None,
                    enabled: true,
                }),
                Err(error) => {
                    tracing::debug!(%error, "skipping an element whose rect could not be read")
                }
            }
        }
        Ok(found)
    }

    /// Bounds **and** the rendered `text` for every match.
    ///
    /// The expensive sibling of [`Self::locate_all`], and it reads `text` rather than
    /// `content-desc`: the caller is reading comment rows, whose author and body labels
    /// were measured to live in `text` while `content-desc` is empty (AGENTS.md §9.5).
    /// Asking for `content-desc` here would return `None` for every row and the caller
    /// would conclude the drawer has no comments in it.
    ///
    /// One extra round trip per element on top of the rectangle. Measured cost for the
    /// rectangles alone was 684 ms for 4 elements and 1172 ms for 13; this roughly
    /// doubles that. Acceptable once per send, never in a poll loop.
    ///
    /// An element whose text cannot be read is kept with `description: None` rather
    /// than dropped — losing a row silently would turn "the author is unreadable" into
    /// "this comment is not on screen", and the second one makes a reply land somewhere
    /// else.
    async fn locate_all_described(
        &self,
        query: riviu_core::ElementQuery<'_>,
    ) -> anyhow::Result<Vec<riviu_core::ElementBox>> {
        let locator = to_agent_locator(query);
        let ids = self.agent.find_all(&locator).await?;
        let mut found = Vec::with_capacity(ids.len());
        for id in ids {
            // Same reasoning as `locate_all`: one stale element in a scrolling list
            // must not lose the others.
            let Ok(rect) = self.agent.rect(&id).await else {
                continue;
            };
            let description = self
                .agent
                .attribute(&id, "text")
                .await
                .ok()
                .flatten()
                .filter(|value| !value.is_empty());
            found.push(riviu_core::ElementBox {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                description,
                enabled: true,
            });
        }
        Ok(found)
    }

    /// True — this is the backend the primitive exists for.
    fn supports_element_bounds(&self) -> bool {
        true
    }

    async fn ui_language(&self) -> Option<String> {
        self.ui_locale().await
    }

    async fn app_version(&self, bundle_id: &str) -> Option<String> {
        self.app_version_name(bundle_id).await
    }

    fn stream_url(&self) -> Option<String> {
        // Frames are published straight into the shared `StreamHub` by the
        // driver's minicap producer (`crate::frames`), so there is no per-session
        // URL to hand out. Hierarchy queries locate controls; frames are the
        // operator's view, not the locator.
        None
    }
}

/// Single-quote a value for `adb shell`.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessibility_id_maps_to_content_desc() {
        let locator = QualifiedElementLocator {
            strategy: ElementLocatorStrategy::AccessibilityId,
            value: "Like".into(),
        };
        assert_eq!(to_locator(&locator), Locator::Description("Like".into()));
    }

    #[test]
    fn class_name_maps_across_unchanged() {
        let locator = QualifiedElementLocator {
            strategy: ElementLocatorStrategy::ClassName,
            value: "android.widget.EditText".into(),
        };
        assert_eq!(
            to_locator(&locator),
            Locator::ClassName("android.widget.EditText".into())
        );
    }

    #[test]
    fn shell_quoting_survives_an_embedded_quote() {
        assert_eq!(shell_quote("https://a/b"), "'https://a/b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn image_coordinates_scale_onto_the_live_screen() {
        let screen = (1080.0, 2220.0);
        // A half-scale screenshot maps back to full device pixels.
        assert_eq!(
            scale_to_screen(270.0, 555.0, 540.0, 1110.0, screen),
            (540.0, 1110.0)
        );
        // A full-resolution screenshot is already in screen space.
        assert_eq!(
            scale_to_screen(996.0, 1250.0, 1080.0, 2220.0, screen),
            (996.0, 1250.0)
        );
    }

    #[test]
    fn hardware_keys_map_to_the_android_keycodes_genfarmer_uses() {
        assert_eq!(hardware_keycode(HardwareKey::Home), 3);
        assert_eq!(hardware_keycode(HardwareKey::Back), 4);
        assert_eq!(hardware_keycode(HardwareKey::Recents), 187);
        assert_eq!(hardware_keycode(HardwareKey::VolumeUp), 24);
        assert_eq!(hardware_keycode(HardwareKey::VolumeDown), 25);
        assert_eq!(hardware_keycode(HardwareKey::Power), 26);
        assert_eq!(hardware_keycode(HardwareKey::Notification), 83);
    }

    #[test]
    fn a_degenerate_image_size_passes_the_point_through_instead_of_producing_nan() {
        let screen = (1080.0, 2220.0);
        assert_eq!(scale_to_screen(10.0, 20.0, 0.0, 0.0, screen), (10.0, 20.0));
        assert_eq!(scale_to_screen(10.0, 20.0, -1.0, 5.0, screen), (10.0, 20.0));
    }

    #[test]
    fn a_rotated_phone_scaled_against_the_stale_screen_lands_off_the_display() {
        // The bug, in numbers. The phone is turned to landscape, so the frame the operator
        // clicks is 2220x1080 -- but the session captured 1080x2220 from `wm size` when it
        // opened and `wm size` does not follow rotation (measured; see `ScreenCache`).
        //
        // A tap in the middle of that frame, scaled against the stale portrait size, does
        // not merely land somewhere else. It lands off the screen entirely, which is why the
        // failure is silent: nothing happens, and nothing can report that nothing happened.
        let stale = (1080.0, 2220.0);
        let (x, y) = scale_to_screen(1110.0, 540.0, 2220.0, 1080.0, stale);
        assert!(
            x > 2220.0 || y > 1080.0,
            "a centre tap scaled against the stale size should be off the rotated display, \
             got ({x}, {y})"
        );

        // Against the refreshed size it is the centre, exactly.
        let live = (2220.0, 1080.0);
        assert_eq!(
            scale_to_screen(1110.0, 540.0, 2220.0, 1080.0, live),
            (1110.0, 540.0)
        );
    }

    #[test]
    fn a_landscape_frame_contradicts_a_portrait_cache() {
        let cache = ScreenCache::seeded((1080.0, 2220.0));
        assert!(
            cache.contradicted_by(2220.0, 1080.0),
            "landscape frame, portrait cache"
        );
        // The half-scale portrait frame pinned by the scaling test above is NOT a
        // contradiction -- it is the ordinary case and must not cost a round trip.
        assert!(!cache.contradicted_by(540.0, 1110.0));
        assert!(!cache.contradicted_by(1080.0, 2220.0));
    }

    #[test]
    fn a_square_or_degenerate_frame_never_forces_a_refresh() {
        // Otherwise this would add an agent read to gestures that carry no orientation
        // information at all, on every device, forever.
        let cache = ScreenCache::seeded((1080.0, 2220.0));
        assert!(!cache.contradicted_by(600.0, 600.0));
        assert!(!cache.contradicted_by(0.0, 0.0));
        assert!(!cache.contradicted_by(-1.0, 5.0));
        // And an empty cache cannot be contradicted -- there is nothing to disagree with.
        let empty = ScreenCache::default();
        assert!(!empty.contradicted_by(2220.0, 1080.0));
    }

    #[test]
    fn invalidating_one_handle_invalidates_the_session_already_handed_out() {
        // The property the whole fix rests on. A session opened before the rotate button was
        // pressed must see the invalidation, or the fix only helps sessions opened later --
        // which is to say, not the one the operator is using.
        let held_by_a_session = ScreenCache::seeded((1080.0, 2220.0));
        let held_by_the_driver = held_by_a_session.clone();
        assert_eq!(held_by_a_session.peek(), Some((1080.0, 2220.0)));

        held_by_the_driver.invalidate();
        assert_eq!(
            held_by_a_session.peek(),
            None,
            "the session is still scaling against a size the driver has thrown away"
        );

        held_by_the_driver.store((2220.0, 1080.0));
        assert_eq!(held_by_a_session.peek(), Some((2220.0, 1080.0)));
    }
}
