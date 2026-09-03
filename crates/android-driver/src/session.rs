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
/// `KEYCODE_SLEEP` — screen off / lock, deterministically (POWER only toggles).
const KEYCODE_SLEEP: i64 = 223;
/// `KEYCODE_WAKEUP` — screen on, deterministically.
const KEYCODE_WAKEUP: i64 = 224;
/// `KEYCODE_MENU` — dismisses a swipe-only keyguard on many builds.
const KEYCODE_MENU: i64 = 82;

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
        // Anchored at the end, and the literal is escaped: the caller passes an id suffix, not a
        // pattern.
        riviu_core::ElementQuery::ResourceIdSuffix(value) => {
            Locator::ResourceIdMatches(format!(".*{}", crate::agent::escape_java_regex(value)))
        }
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

    /// Real key events, via `input text`.
    ///
    /// This exists because `set_text` is invisible to the app: `ACTION_SET_TEXT` changes the
    /// field's contents without any keystroke reaching TikTok, so its mention picker — which
    /// watches typing — never opens. Measured 24/08/2026 on `ce051715ac247a3f01`: setting the
    /// comment box to `@lt.gi` did nothing, while injecting the same characters here opened
    /// the picker and filtered it to four real accounts.
    ///
    /// **The character set is a whitelist, and that is a security boundary, not tidiness.**
    /// `adb shell` runs a real shell on the phone, so this string reaches it as code — the
    /// same hazard `validate_bundle_id` exists for. Handles are `[A-Za-z0-9._-]` with a
    /// leading `@`; anything else is refused rather than escaped, because the only caller
    /// needs nothing else and an escaping bug here is a remote shell.
    ///
    /// It is also ASCII-only for a second reason: `input text` is *killed* by diacritics,
    /// which is why `type_text` goes through accessibility instead.
    async fn type_keys(&self, text: &str) -> anyhow::Result<()> {
        let typed = keys_payload(text)?;
        self.adb
            .shell(&self.serial, &format!("input text {typed}"))
            .await?;
        Ok(())
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

    /// Sleep locks; wake then nudges past a swipe-only keyguard.
    ///
    /// A secure PIN is deliberately left alone — this is a screen on/off for the fleet, not
    /// a lock-screen bypass, so a PIN-protected phone wakes to its own lock screen.
    async fn set_locked(&self, locked: bool) -> anyhow::Result<()> {
        if locked {
            self.agent.press_key(KEYCODE_SLEEP).await
        } else {
            self.agent.press_key(KEYCODE_WAKEUP).await?;
            let _ = self.agent.press_key(KEYCODE_MENU).await;
            Ok(())
        }
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
                // **The three-way answer matters, and flattening it produced a false
                // sentence.** Measured 23/08/2026 on two locked phones: `mCurrentFocus` read
                // `Window{… StatusBar}`, which has no `package/activity` pair, so the old
                // code reported *"had no mCurrentFocus line"* — about a line that was right
                // there. The operator was then told the phone was "unreadable", which is not
                // a thing anybody can act on; "on the lock screen" is.
                Ok(stdout) => match crate::adb::parse_foreground_window(&stdout) {
                    crate::adb::ForegroundWindow::App(package) => return Ok(package),
                    // Names the window and stops there. It used to add "the phone is most
                    // likely on its lock screen", which is one possibility presented as the
                    // answer: measured 25/08/2026, a phone reporting `Select input method`
                    // was awake, unlocked, and holding Android's keyboard chooser over the
                    // app. The caller can act on a name; it cannot act on a guess.
                    crate::adb::ForegroundWindow::System(window) => tried.push(format!(
                        "`{source}` reported the system window {window}, not an app — a lock \
                         screen does this, and so does any system dialog standing over the \
                         app"
                    )),
                    crate::adb::ForegroundWindow::Unreadable => {
                        tried.push(format!("`{source}` had no readable mCurrentFocus line"))
                    }
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

    /// Bounds, label, and both armed flags, from one hierarchy query.
    ///
    /// The attribute read-backs are extra round trips rather than fields of the
    /// first response, because the agent's find reply carries only the element id.
    /// All three are still worth it: the comment control's own text is what tells two
    /// posts apart — that is what proves a swipe advanced — the Send button's
    /// `enabled` flag is what proves the comment is armed, and the image picker's
    /// `Next` moves `clickable` instead, which is what proves images were selected
    /// before anything is posted.
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
        let enabled = enabled_from_attribute(
            self.agent
                .attribute(&element, "enabled")
                .await
                .ok()
                .flatten(),
        );
        let clickable = clickable_from_attribute(
            self.agent
                .attribute(&element, "clickable")
                .await
                .ok()
                .flatten(),
        );
        Ok(Some(located_box(
            rect,
            description,
            ArmedFlags { enabled, clickable },
        )))
    }

    /// The stateful sibling of `locate`, used only by controls whose boolean state is itself
    /// the proof. Keeping it separate preserves the old locate path and its round-trip count.
    async fn locate_stateful(
        &self,
        query: riviu_core::ElementQuery<'_>,
    ) -> anyhow::Result<Option<riviu_core::driver::StatefulElementBox>> {
        let locator = to_agent_locator(query);
        let Some(element) = self.agent.find(&locator).await? else {
            return Ok(None);
        };
        let rect = self.agent.rect(&element).await?;
        let description = self
            .agent
            .attribute(&element, "content-desc")
            .await
            .ok()
            .flatten();
        let enabled = enabled_from_attribute(
            self.agent
                .attribute(&element, "enabled")
                .await
                .ok()
                .flatten(),
        );
        let clickable = clickable_from_attribute(
            self.agent
                .attribute(&element, "clickable")
                .await
                .ok()
                .flatten(),
        );
        let checked = state_from_attribute(
            self.agent
                .attribute(&element, "checked")
                .await
                .ok()
                .flatten(),
        );
        let selected = state_from_attribute(
            self.agent
                .attribute(&element, "selected")
                .await
                .ok()
                .flatten(),
        );
        Ok(Some(riviu_core::driver::StatefulElementBox {
            element: located_box(rect, description, ArmedFlags { enabled, clickable }),
            checked,
            selected,
        }))
    }

    /// Bounds for **every** match, geometry only.
    ///
    /// Deliberately skips the `content-desc`, `enabled` and `clickable` read-backs that
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
                    // Constant, like `enabled` above, and for the same reason: this
                    // path reads no attributes. `false` is the refusing direction.
                    clickable: false,
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
                clickable: false,
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

/// The exact argument `input text` will receive, or a refusal.
///
/// **Pure and separate because this is a security boundary.** The string ends up inside
/// `adb -s <serial> shell "input text …"`, which is executed by `/system/bin/sh` **on the
/// device** — so what the whitelist admits decides whether a handle can become a command. It
/// was only reachable through a method needing a live `Adb` and a phone, so nothing tested it.
///
/// Three properties, each load-bearing:
///
/// * the admitted set is `[A-Za-z0-9@._- ]`, so no metacharacter survives — not `;` `|` `&`
///   `$` backtick `(` `)` `<` `>` newline `*` `?` `[` `]` `{` `}` `~` `!` `#` `'` `"` `\`;
/// * `%` is **not** admitted, which is what makes the space handling safe rather than merely
///   convenient: a caller cannot forge the `%s` escape;
/// * space becomes `%s` — Android's own escape, `Input.java` does `text.replace("%s", " ")` —
///   so the emitted command has zero user-controlled spaces and `input text <token>` is always
///   exactly three shell words. Word-splitting is structurally impossible, not merely unlikely.
fn keys_payload(text: &str) -> anyhow::Result<String> {
    // **Refused, not silently accepted.** `Ok(())` on an empty string is a success that reports
    // "typed" about a device nothing was sent to — the same flattened answer the
    // `ForegroundWindow` split in this file exists to stop producing.
    anyhow::ensure!(!text.is_empty(), "typeKeys refuses an empty string");
    // A handle is tens of characters. `adb shell` has a transport-level limit on the command
    // string, so without a bound here a long one fails inside adb with an opaque message
    // instead of here with a clear one. Counted in characters, not bytes, because this runs
    // before the ASCII whitelist and a non-ASCII string would otherwise be refused for a
    // length it does not have.
    const MAX_KEYS: usize = 256;
    anyhow::ensure!(
        text.chars().count() <= MAX_KEYS,
        "typeKeys refuses {} characters: the limit is {MAX_KEYS}, and `adb shell` truncates a \
         longer command string rather than reporting it",
        text.chars().count()
    );
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '-' | ' ');
    anyhow::ensure!(
        text.chars().all(allowed),
        "typeKeys refuses {text:?}: only ASCII letters, digits, space and @._- reach the device \
         shell, and anything else would need escaping this deliberately does not do"
    );
    Ok(text.replace(' ', "%s"))
}

/// The two armed flags of one element, read and defaulted.
///
/// A named pair rather than two positional `bool`s, and that is the whole reason it exists:
/// the two are the same type and mean opposite things, so `located_box(rect, desc, a, b)`
/// with the arguments swapped compiles, runs, and — on the measured picker, where `enabled`
/// is always `"true"` — reports every element as clickable. `ElementBox.clickable` would then
/// say "images are selected" before any were.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArmedFlags {
    enabled: bool,
    clickable: bool,
}

/// Assemble the box, so the field assignment is testable without a device.
///
/// Split out because a source-scanning gate can only see that both attribute names and both
/// helper names appear in `locate`; it cannot see which read feeds which field. A review
/// named the exact mutation that stayed green: pass the `clickable` response to
/// `enabled_from_attribute` and vice versa. This function is where that becomes a unit test.
fn located_box(
    rect: crate::agent::Rect,
    description: Option<String>,
    flags: ArmedFlags,
) -> riviu_core::ElementBox {
    riviu_core::ElementBox {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        description,
        enabled: flags.enabled,
        clickable: flags.clickable,
    }
}

/// Read the `enabled` attribute, **defaulting to enabled** when it cannot be read.
///
/// The default is the whole content of this function. TikTok's comment Send button
/// exists in the drawer the entire time and flips `enabled=false` → `true` when the
/// field holds text, so this flag is what proves a comment is armed. Guessing
/// *disabled* on a failed attribute read would report a live button as unarmed and
/// silently drop every comment — a failure that costs nothing but looks like the app
/// simply not working. Guessing *enabled* costs at worst one tap that does nothing.
///
/// Anything that is not the literal `false` counts as enabled, matching the way the
/// hierarchy renders the attribute.
fn enabled_from_attribute(raw: Option<String>) -> bool {
    raw.map(|value| value != "false").unwrap_or(true)
}

/// Read the `clickable` attribute, **defaulting to not-clickable** when it cannot be read.
///
/// The mirror image of [`enabled_from_attribute`], and the asymmetry is the point.
/// Measured 29/08/2026 on `com.ss.android.ugc.trill` 38.3.2 — the build sixteen of the
/// twenty phones run — TikTok's image picker arms its `Next` button by moving
/// `clickable` while `enabled` stays `true` throughout:
///
/// ```text
///   nothing selected   clickable=false  enabled=true
///   one image selected clickable=true   enabled=true
/// ```
///
/// So this flag is the only evidence that images were actually selected, and what it
/// gates is a **post**. Guessing *armed* on a failed attribute read would advance out
/// of the picker with nothing chosen, and further down that path there is no delete
/// on Android to undo the result. Unknown therefore refuses.
///
/// And unlike `enabled`, only the literal `true` counts — an unreadable or unexpected
/// value is not permission to post.
fn clickable_from_attribute(raw: Option<String>) -> bool {
    raw.map(|value| value == "true").unwrap_or(false)
}

/// Parse an optional hierarchy boolean without turning absence or malformed XML into `false`.
fn state_from_attribute(raw: Option<String>) -> Option<bool> {
    match raw.as_deref() {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The two armed flags default in opposite directions, and that is deliberate.**
    ///
    /// Both read a boolean attribute off the same hierarchy, so the temptation to write
    /// one helper for the pair is real — and it would be wrong, because they guard
    /// different things. `enabled` guards sending a comment: refusing on doubt loses a
    /// message. `clickable` guards leaving the image picker: accepting on doubt posts a
    /// carousel that nothing in this project can take down again.
    ///
    /// If a later edit unifies them, this test is what fails.
    #[test]
    fn the_unreadable_case_refuses_for_a_post_and_permits_for_a_comment() {
        assert!(
            enabled_from_attribute(None),
            "an unreadable `enabled` must not report a live Send button as unarmed"
        );
        assert!(
            !clickable_from_attribute(None),
            "an unreadable `clickable` must never be read as `enough images are selected`"
        );

        // The measured values, both ways round.
        assert!(enabled_from_attribute(Some("true".into())));
        assert!(!enabled_from_attribute(Some("false".into())));
        assert!(clickable_from_attribute(Some("true".into())));
        assert!(!clickable_from_attribute(Some("false".into())));

        // A value neither helper was measured against. `enabled` is permissive by
        // design; `clickable` accepts nothing but the literal, because the cost of
        // being wrong is a published post.
        assert!(enabled_from_attribute(Some("TRUE".into())));
        assert!(!clickable_from_attribute(Some("TRUE".into())));
        assert!(!clickable_from_attribute(Some(String::new())));
    }

    #[test]
    fn checked_and_selected_xml_attributes_keep_true_false_and_absent_distinct() {
        assert_eq!(state_from_attribute(Some("true".to_owned())), Some(true));
        assert_eq!(state_from_attribute(Some("false".to_owned())), Some(false));
        assert_eq!(state_from_attribute(None), None);
        assert_eq!(state_from_attribute(Some("unexpected".to_owned())), None);
    }

    #[test]
    fn locate_stateful_reads_checked_and_selected_from_the_same_hierarchy_node() {
        let source = include_str!("session.rs").replace("\r\n", "\n");
        let start = source
            .find("    async fn locate_stateful(")
            .expect("Android must override `locate_stateful`");
        let body = &source[start
            ..source[start..]
                .find("    async fn locate_all(")
                .map(|offset| start + offset)
                .expect("the next method")];
        assert!(body.contains(r#"attribute(&element, "checked")"#));
        assert!(body.contains(r#"attribute(&element, "selected")"#));
        assert!(body.contains("state_from_attribute("));
    }

    /// **Each flag lands in its own field.**
    ///
    /// The gap a review named: the source-scanning gate below sees that both attribute names
    /// and both helpers appear in `locate`, and cannot see which read feeds which field.
    /// Swapping them compiles and passes every other test — and on the measured picker, where
    /// `enabled` is always `"true"`, it makes every element report `clickable`, which the
    /// composer reads as "images are selected" before any are.
    #[test]
    fn the_two_armed_flags_do_not_land_in_each_others_fields() {
        let rect = crate::agent::Rect {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        };
        // Deliberately opposite, so a swap cannot pass by coincidence.
        let element = located_box(
            rect,
            Some("Next".into()),
            ArmedFlags {
                enabled: false,
                clickable: true,
            },
        );
        assert!(!element.enabled, "the clickable flag landed in `enabled`");
        assert!(element.clickable, "the enabled flag landed in `clickable`");
        assert_eq!((element.x, element.y), (1.0, 2.0));
        assert_eq!((element.width, element.height), (3.0, 4.0));
        assert_eq!(element.description.as_deref(), Some("Next"));

        let mirrored = located_box(
            rect,
            None,
            ArmedFlags {
                enabled: true,
                clickable: false,
            },
        );
        assert!(mirrored.enabled);
        assert!(!mirrored.clickable);
    }

    /// `locate` must actually ask for both attributes, on the real element.
    ///
    /// The helpers above are pure and provable, but a caller that never calls them is
    /// what a pure test cannot see: deleting the `clickable` read-back would leave every
    /// box `false` and every picker permanently "not armed", which reads as a phone
    /// problem rather than a missing line. Only `locate` pays for the round trips —
    /// `locate_all` and `locate_all_described` skip both on purpose — so this counts
    /// sites rather than asking whether the string appears anywhere in the file.
    #[test]
    fn locate_reads_both_armed_flags_and_the_list_paths_still_read_neither() {
        let source = include_str!("session.rs").replace("\r\n", "\n");
        let body = {
            let start = source
                .find("    async fn locate(")
                .expect("`locate` is still called that");
            let open = start + source[start..].find('{').expect("a body");
            let mut depth = 0usize;
            let mut end = open;
            for (offset, character) in source[open..].char_indices() {
                match character {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + offset;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            &source[open..end]
        };
        assert!(
            body.contains(r#"attribute(&element, "enabled")"#),
            "`locate` stopped reading `enabled`; the comment path can no longer tell armed from not"
        );
        assert!(
            body.contains(r#"attribute(&element, "clickable")"#),
            "`locate` stopped reading `clickable`; the image picker can no longer prove a selection"
        );
        assert!(
            body.contains("enabled_from_attribute(") && body.contains("clickable_from_attribute("),
            "`locate` parses the attributes inline again, so the defaults tested above are dead"
        );

        // **And each attribute goes into the helper that belongs to it.** Presence alone was
        // the gate's whole content, and a review named the mutation that survived it: feed
        // the `clickable` response to `enabled_from_attribute` and vice versa. Both helpers
        // still appear, both attribute names still appear, every unit test still passes — and
        // on the measured picker, where `enabled` is always `"true"`, every element comes back
        // `clickable`, which reads as "images are selected" before any are.
        //
        // Matched by cutting each helper's call and requiring its own attribute name inside.
        for (helper, attribute) in [
            ("enabled_from_attribute(", "\"enabled\""),
            ("clickable_from_attribute(", "\"clickable\""),
        ] {
            let start = body
                .find(helper)
                .unwrap_or_else(|| panic!("{helper} is no longer called in `locate`"));
            let call = &body[start..];
            let end = call
                .find(");")
                .unwrap_or_else(|| panic!("{helper} call does not terminate"));
            assert!(
                call[..end].contains(attribute),
                "{helper} is fed something other than {attribute}; the two flags mean opposite \
                 things and swapping them is invisible on a build where `enabled` never moves"
            );
        }

        // The list paths deliberately pay for neither, and their boxes say so. Counted
        // over the module only — this test's own source is in the same file, and a gate
        // that counts its own assertion text passes no matter what the module does.
        let module = &source[..source.find("#[cfg(test)]").expect("a test module")];
        assert_eq!(
            module.matches("clickable: false,").count(),
            2,
            "`locate_all` and `locate_all_described` must keep declaring that they did not read it"
        );
    }

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

    /// The key payload is a security boundary, so it is tested like one.
    #[test]
    fn the_key_payload_lets_no_metacharacter_reach_the_device_shell() {
        // What the one caller actually sends: a leading space so the tag does not run into the
        // last word of the comment, and the space arrives as Android's own escape.
        assert_eq!(keys_payload(" @lt.gi").expect("a handle"), "%s@lt.gi");

        // `%` is not admitted, so the escape cannot be forged by a caller.
        assert!(keys_payload("%s").is_err(), "`%s` must not be forgeable");

        for hostile in [
            "a;id", "a|id", "a&id", "a$(id)", "a`id`", "a>f", "a<f", "a\nid", "a*", "a?", "a[b]",
            "a{b}", "a~b", "a!b", "a#b", "a'b", "a\"b", "a\\b", "tên",
        ] {
            assert!(
                keys_payload(hostile).is_err(),
                "{hostile:?} must not reach /system/bin/sh"
            );
        }

        // Doing nothing is not a success.
        assert!(keys_payload("").is_err());

        // And a command string too long to survive `adb shell` is refused here, where the
        // message says so, rather than inside adb where it does not.
        assert!(keys_payload(&"a".repeat(256)).is_ok());
        assert!(keys_payload(&"a".repeat(257)).is_err());
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
