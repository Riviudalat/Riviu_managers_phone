//! HTTP client for the resident on-device agent (`appium-uiautomator2-server`,
//! Apache-2.0), reached over `adb forward`.
//!
//! Everything in a control loop belongs here rather than in [`crate::adb`].
//! Measured on the same Galaxy S8+, same operations
//! (`docs/ANDROID_PROBE_REPORT_2026-08-09.md`):
//!
//! | operation      | adb CLI   | this client |
//! |----------------|-----------|-------------|
//! | click          | 1502 ms   | 130–280 ms  |
//! | find element   | ~2700 ms  | 609 ms      |
//! | read attribute | ~2700 ms  | 241 ms      |
//! | type Vietnamese| impossible| 741–1156 ms |
//!
//! One thing the agent does **not** make cheap is dumping the whole hierarchy
//! (3403 ms via the agent against 2693–4239 ms via the CLI), because that cost
//! is traversing and serialising the accessibility tree rather than starting a
//! tool. So: never walk the tree in a loop, ask for the element you want. That
//! is already the shape of `UiSession` — `find_and_tap`, `assert_visible` and
//! `read_text` are all targeted queries.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use parking_lot::Mutex;
use serde_json::{json, Value};

/// W3C returns the element id under this key; older servers use `ELEMENT`.
const W3C_ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

/// How long one UiAutomator2 request may take before the driver gives up on it.
///
/// Thirty rather than `adb.rs`'s sixty because this is one HTTP call to a server already
/// running on the phone, not a command that may have to start one. A request still pending
/// after this is not slow, it is a server that stopped answering, and waiting longer only
/// delays the restart that fixes it.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// How an element is addressed on screen.
///
/// `Description` is the primary strategy and `ResourceId` deliberately is not.
/// TikTok's resource ids are R8-obfuscated (`a1p`, `ty9`, `ebz`) and move
/// between builds, while `content-desc` is semantic, stable, and in English
/// regardless of the UI language — measured `Like`, `Video liked`,
/// `Follow <name>`, `Read or add comments. 15 comments`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Locator {
    /// `content-desc`, exact match. Appium calls this "accessibility id".
    Description(String),
    /// `content-desc`, substring match. Needed because several of TikTok's
    /// labels embed a count (`Read or add comments. 15 comments`).
    DescriptionContains(String),
    ClassName(String),
    /// The rendered `text`, exact match.
    ///
    /// Not interchangeable with [`Self::Description`]: the Reply button in TikTok's
    /// comment drawer has an **empty** `content-desc` and carries `Trả lời` here, and
    /// a comment body's content *is* its text.
    Text(String),
    /// The rendered `text`, substring match.
    TextContains(String),
    /// Fully-qualified `resource-id`. Obfuscated in TikTok; use sparingly.
    ResourceId(String),
    /// A Java regex over the fully-qualified `resource-id`.
    ///
    /// The way to reach a node whose *class* is obfuscated but whose id is not — see
    /// [`riviu_core::ElementQuery::ResourceIdSuffix`] for the measurement that needs it.
    ResourceIdMatches(String),
    /// Raw `UiSelector` expression, for the cases the above cannot express.
    UiSelector(String),
}

impl Locator {
    fn to_body(&self) -> Value {
        match self {
            Self::Description(value) => json!({
                "strategy": "accessibility id",
                "selector": value,
            }),
            Self::DescriptionContains(value) => json!({
                "strategy": "-android uiautomator",
                "selector": format!(
                    "new UiSelector().descriptionContains({})",
                    quote_java(value)
                ),
            }),
            Self::ClassName(value) => json!({
                "strategy": "class name",
                "selector": value,
            }),
            // `quote_java` is not optional here: a comment body can contain a double
            // quote, and an unescaped one turns the selector into a parse error at
            // best and a different selector at worst.
            Self::Text(value) => json!({
                "strategy": "-android uiautomator",
                "selector": format!("new UiSelector().text({})", quote_java(value)),
            }),
            Self::TextContains(value) => json!({
                "strategy": "-android uiautomator",
                "selector": format!("new UiSelector().textContains({})", quote_java(value)),
            }),
            Self::ResourceId(value) => json!({
                "strategy": "id",
                "selector": value,
            }),
            Self::ResourceIdMatches(pattern) => json!({
                "strategy": "-android uiautomator",
                "selector": format!(
                    "new UiSelector().resourceIdMatches({})",
                    quote_java(pattern)
                ),
            }),
            Self::UiSelector(expression) => json!({
                "strategy": "-android uiautomator",
                "selector": expression,
            }),
        }
    }

    /// Narrow this locator to the element that currently holds focus.
    ///
    /// Required for typing. With TikTok's comment drawer open there are **two**
    /// `android.widget.EditText` nodes: the collapsed bar behind the drawer
    /// (`focused=false`) and the real field inside it (`focused=true`). A
    /// class-name lookup returns the collapsed one, and setting text on it
    /// succeeds at the API level while the screen stays empty — measured, and
    /// the most expensive wrong turn of the probe session.
    pub fn focused(self) -> Self {
        let inner = match self {
            Self::Description(value) => {
                format!("new UiSelector().description({})", quote_java(&value))
            }
            Self::DescriptionContains(value) => format!(
                "new UiSelector().descriptionContains({})",
                quote_java(&value)
            ),
            Self::ClassName(value) => {
                format!("new UiSelector().className({})", quote_java(&value))
            }
            Self::Text(value) => format!("new UiSelector().text({})", quote_java(&value)),
            Self::TextContains(value) => {
                format!("new UiSelector().textContains({})", quote_java(&value))
            }
            Self::ResourceId(value) => {
                format!("new UiSelector().resourceId({})", quote_java(&value))
            }
            Self::ResourceIdMatches(pattern) => {
                format!(
                    "new UiSelector().resourceIdMatches({})",
                    quote_java(&pattern)
                )
            }
            Self::UiSelector(expression) => expression,
        };
        Self::UiSelector(format!("{inner}.focused(true)"))
    }
}

/// Escape the regex metacharacters in a literal, for `resourceIdMatches`.
///
/// Separate from [`quote_java`] because the two protect against different things: that one stops
/// a quote from ending the Java string, this one stops a `.` from matching any character. A
/// resource-id suffix like `:id/desc` needs neither today, and the next one will.
/// Public because the examples need the *same* escaping the production path uses.
///
/// A resource-id suffix is a literal and the agent takes a Java regex, so a probe that
/// interpolated it raw would be testing a different locator than the one the app sends.
pub fn escape_java_regex(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(
            ch,
            '.' | '\\' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Quote a Java string literal for embedding in a `UiSelector` expression.
fn quote_java(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Element bounds in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn centre(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

/// The server's own words when its accessibility connection has gone bad.
///
/// Matched as a substring because the message carries a varying millisecond count.
const STALE_TREE_MARKER: &str = "waiting for the root AccessibilityNodeInfo";

/// Whether a failed accessibility request may be repeated after replacing the agent session.
///
/// UiAutomator2 models element lookup as `POST`, so HTTP method alone cannot separate reads from
/// effects. Keep the allowlist deliberately narrow: a repeated lookup/source read observes the
/// screen again, while repeating `/actions`, element click/value/clear, or a key press can emit a
/// second public effect after the first response was lost.
fn stale_tree_retry_is_safe(method: &reqwest::Method, suffix: &str) -> bool {
    (*method == reqwest::Method::POST && matches!(suffix, "/element" | "/elements"))
        || (*method == reqwest::Method::GET
            && (suffix == "/source" || suffix.starts_with("/element/")))
}

/// Past this, an **operator gesture** is something they can feel.
///
/// A tap is one `/actions` round trip. Measured on this fleet's own release log: 227 `/actions`
/// calls crossed this line, p50 624 ms, worst 1 539 ms, and **none** reached 5 s — so at this
/// budget the route reports real sluggishness and nothing else.
const SLOW_AGENT_CALL: Duration = Duration::from_millis(500);

/// Past this, reading the **accessibility tree** is something worth a line.
///
/// **The number this replaces was measured against the wrong operation, and the log proves it.**
/// `SLOW_AGENT_CALL`'s own comment used to claim it "catches control got sluggish without
/// printing a line for every hierarchy read". Counted over one release log — 13 221 warnings in
/// total — it printed a line for almost exactly that:
///
/// | route | lines | p50 | p90 | worst | ≥ 5 s |
/// |---|---|---|---|---|---|
/// | `/element` | **9 059** | 888 ms | 2 520 ms | 19 938 ms | 545 |
/// | `/elements` | 323 | 1 494 ms | 4 403 ms | 11 682 ms | 18 |
/// | `/actions` | 227 | 624 ms | 901 ms | 1 539 ms | 0 |
///
/// A tree read on this fleet **routinely** takes ~900 ms; it is not sluggish, it is the cost of
/// the operation. So 10 914 of 13 221 warning lines — 83% of the log — were one sentence about
/// a normal thing, and the real signal underneath (475 lost-accessibility-tree restarts, 223 adb
/// slot starvations, 143 refused view tokens, 60 failed scrcpy pushes) was unreadable. A log
/// nobody can read is why every incident on this project starts by guessing.
///
/// Five seconds is above the p90 of both tree routes and far below their worst cases, so what
/// remains is the tail that actually hurts: ~563 lines instead of 9 382. **Re-measure before
/// moving it** — the table above is how, and the same script prints it.
const SLOW_TREE_READ: Duration = Duration::from_secs(5);

/// How long this route is allowed to take before it is worth a warning.
///
/// Pure and keyed on the route rather than the caller, because the suffix is the only thing the
/// logging site knows — and because a table of shapes is checkable, where a threshold passed in
/// by every caller is a thing to get wrong at each one.
fn slow_call_budget(route: &str) -> Duration {
    // `/element`, `/elements`, and every `/element/<uuid>/…` attribute and rect read: all of
    // them walk the accessibility tree, and all of them were being held to a tap's budget.
    if route.starts_with("/element") || route.starts_with("/source") {
        return SLOW_TREE_READ;
    }
    SLOW_AGENT_CALL
}

#[derive(Clone)]
pub struct AgentClient {
    http: reqwest::Client,
    base: String,
    /// Carried only so a slow or failing call can name the phone it was talking to.
    ///
    /// `base=http://127.0.0.1:6795` is technically the same fact and practically useless:
    /// nobody holds the port-to-serial map in their head while reading a log.
    serial: String,
    /// Shared and swappable, so recycling a degraded session fixes **every** clone —
    /// including the `AndroidUiSession` already handed to a running loop.
    session_id: Arc<Mutex<String>>,
}

impl AgentClient {
    /// Open a session against an agent already listening on `base`.
    pub async fn connect(
        serial: impl Into<String>,
        base: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let base = base.into().trim_end_matches('/').to_string();
        // 30 s, from measurements rather than from caution. The slowest *legitimate*
        // element query recorded against this server is 10,2–10,5 s (the S8+ fleet under a
        // playing feed, `docs/ANDROID_PROBE_REPORT_2026-08-09.md`), and a rotten session
        // answers at 10,1 s before erroring; `/source` is 3,4 s. Nothing measured comes
        // near 30 s, and nothing slow goes through this client at all — APK pushes and
        // installs are adb.
        //
        // The old 120 s was not a limit anyone would notice, it was a hang: an agent that
        // had lost `UiAutomation` made `locate` — four round trips — stall for eight
        // minutes, which is how a probe run got killed at its 600 s cap on 12/08/2026.
        // A timeout is only useful if it fires before the operator gives up.
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("dựng HTTP client cho agent Android")?;
        let response: Value = http
            .post(format!("{base}/session"))
            .json(&json!({ "capabilities": { "firstMatch": [{}], "alwaysMatch": {} } }))
            .send()
            .await
            .context("mở session với agent Android")?
            .json()
            .await
            .context("đọc phản hồi mở session")?;
        let session_id = response
            .pointer("/value/sessionId")
            .and_then(Value::as_str)
            .or_else(|| response.get("sessionId").and_then(Value::as_str))
            .ok_or_else(|| anyhow!("agent không trả sessionId: {response}"))?
            .to_string();
        let client = Self {
            http,
            base,
            serial: serial.into(),
            session_id: Arc::new(Mutex::new(session_id)),
        };
        client.prime_session().await?;
        Ok(client)
    }

    /// Configure the session before anything else uses it.
    ///
    /// Same shape as the iOS rule in AGENTS.md 2.2, where a stock WDA session
    /// must be primed immediately. Zero means read the tree as it is now: a
    /// feed of autoplaying video is never idle, so waiting for idle is waiting
    /// for something that will not happen.
    ///
    /// **This does not fix the big problem, and must not be read as if it
    /// did.** Under a playing TikTok feed on the Galaxy S8+ fleet, every
    /// element query costs about 10.2–10.5 s: the server waits on a *hardcoded*
    /// root-`AccessibilityNodeInfo` timeout that no setting reaches.
    /// `waitForIdleTimeout: 0` was verified applied and changed nothing, and
    /// neither did `enableTopmostWindowFromActivePackage` or
    /// `deferAccessibilityCacheReset`. Measured 20/20 queries at p50 10531 ms.
    /// See `docs/ANDROID_PROBE_REPORT_2026-08-09.md`.
    /// Uses [`Self::send_once`] deliberately: this runs against a session that was
    /// *just* created, so a recycle here would be a loop, and a fresh session having
    /// a rotten tree is not a case that recycling can fix.
    async fn prime_session(&self) -> anyhow::Result<()> {
        self.send_once(
            reqwest::Method::POST,
            "/appium/settings",
            Some(json!({ "settings": { "waitForIdleTimeout": 0 } })),
        )
        .await
        .map(|_| ())
    }

    /// Delete this session on the device.
    ///
    /// **Not hygiene — a measured fix.** `connect` POSTs `/session` and nothing used
    /// to remove it, so every `open_session` left one behind. Measured on a Redmi
    /// Note 12 (11/08/2026) after roughly ten accumulated sessions in one afternoon:
    /// every element query went from ~150 ms to the server's hardcoded
    /// root-`AccessibilityNodeInfo` timeout — 10 000+ ms, then `absent` — and a
    /// force-stop of the instrumentation restored 118–425 ms immediately. The
    /// symptom looks exactly like a wrong locator, which is what makes it expensive:
    /// AGENTS.md §9 records the same 10 s regime for the S8+ fleet as if it were a
    /// property of a playing feed.
    pub async fn close(&self) -> anyhow::Result<()> {
        self.send(reqwest::Method::DELETE, "", None)
            .await
            .map(|_| ())
    }

    /// Whether this session can still **read the screen**, used before reuse.
    ///
    /// It queries an element, and that is the whole point — `window_size` is not
    /// evidence and used to be what this asked. Measured on an SM-N950F on
    /// 12/08/2026, on an agent whose `UiAutomation` connection had been taken away by
    /// an out-of-band `adb shell uiautomator dump`: the server process was alive,
    /// `/status` answered, `window_size` answered in **0 ms**, and every element query
    /// blocked until the HTTP timeout. `is_alive` said yes, so
    /// [`AndroidDriver::ensure_agent`] reused the session, and the nurture loop then
    /// timed out waiting for a feed that was plainly on screen.
    ///
    /// `FrameLayout` because it is present on every Android screen, so a healthy agent
    /// answers from the first node it walks — a locator that is *absent* would instead
    /// wait out the server's hardcoded root-node timeout and make a healthy agent look
    /// broken. `find` maps a genuine absence to `Ok(None)`, which counts as alive: this
    /// asks whether the tree is readable, not what is in it.
    /// What one element query costs against an agent that has lost `UiAutomation`.
    ///
    /// Not a timeout of ours — it is the server's own hardcoded root-`AccessibilityNodeInfo`
    /// deadline, and there is no setting that reaches it. Measured twice on this fleet at
    /// 10 116 ms and 10 132 ms; rounded up so a derivation built on it cannot come out
    /// short. Callers use it to reason about how long a *failing* agent takes to admit it
    /// is failing, which is the number that sizes recovery windows.
    pub const BLIND_QUERY_COST: Duration = Duration::from_secs(11);

    pub async fn is_alive(&self) -> bool {
        self.find(&Locator::ClassName("android.widget.FrameLayout".into()))
            .await
            .is_ok()
    }

    /// Is the agent **listening**? Cheap, and deliberately weaker than
    /// [`Self::is_alive`]: `/status` says a server is bound to the port, not that it can
    /// read the screen. Good enough for the operator's tile, not good enough to hand a
    /// session to a loop.
    pub async fn is_ready(base: &str) -> bool {
        let Ok(http) = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
        else {
            return false;
        };
        let url = format!("{}/status", base.trim_end_matches('/'));
        match http.get(url).send().await {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }

    fn url(&self, suffix: &str) -> String {
        format!("{}/session/{}{suffix}", self.base, self.session_id.lock())
    }

    /// Replace a degraded session with a fresh one, in place.
    ///
    /// **Measured, and it corrects an earlier wrong conclusion.** On a Redmi Note 12
    /// (11/08/2026) a session that had been in use for a while answered every element
    /// query with the server's hardcoded root-`AccessibilityNodeInfo` timeout — 10 116 ms
    /// and then an error, which reads exactly like a wrong locator. `GET /sessions`
    /// showed **one** session, so sessions were not accumulating. Deleting that session
    /// and creating another, **without restarting the agent**, dropped the same query to
    /// **7 ms**. So it is a long-lived session that rots, not a pile of them.
    ///
    /// That is why holding one session forever is the wrong fix: the desktop app runs
    /// for hours. Recycling on the server's own error message is self-healing and needs
    /// no timing heuristic.
    async fn recreate_session(&self) -> anyhow::Result<()> {
        let previous = self.session_id.lock().clone();
        // Best effort: the point is the new session, and the old one is already broken.
        let _ = self
            .http
            .delete(format!("{}/session/{previous}", self.base))
            .send()
            .await;
        let response: Value = self
            .http
            .post(format!("{}/session", self.base))
            .json(&json!({ "capabilities": { "firstMatch": [{}], "alwaysMatch": {} } }))
            .send()
            .await
            .context("mở lại session sau khi cây accessibility hỏng")?
            .json()
            .await
            .context("đọc phản hồi mở lại session")?;
        let fresh = response
            .pointer("/value/sessionId")
            .and_then(Value::as_str)
            .or_else(|| response.get("sessionId").and_then(Value::as_str))
            .ok_or_else(|| anyhow!("agent không trả sessionId khi mở lại: {response}"))?
            .to_string();
        tracing::warn!(
            previous = %previous,
            fresh = %fresh,
            "recycled a degraded agent session"
        );
        *self.session_id.lock() = fresh;
        self.prime_session().await
    }

    async fn send(
        &self,
        method: reqwest::Method,
        suffix: &str,
        body: Option<Value>,
    ) -> anyhow::Result<Value> {
        match self.send_once(method.clone(), suffix, body.clone()).await {
            Err(error)
                if stale_tree_retry_is_safe(&method, suffix)
                    && error.to_string().contains(STALE_TREE_MARKER) =>
            {
                // One retry, on one specific server message and only for an allowlisted read.
                // Effectful routes propagate the first error because the device may have acted.
                self.recreate_session().await?;
                self.send_once(method, suffix, body).await
            }
            other => other,
        }
    }

    async fn send_once(
        &self,
        method: reqwest::Method,
        suffix: &str,
        body: Option<Value>,
    ) -> anyhow::Result<Value> {
        let url = self.url(suffix);
        let mut request = self.http.request(method, &url);
        if let Some(body) = body {
            request = request.json(&body);
        }
        // Every operator gesture ends here, so this is where "control feels sluggish" either
        // is or is not true — and until now nothing measured it. A tap is one `/actions`
        // round trip over the adb forward; measured on this fleet it should be 130–280 ms,
        // so anything past half a second is the thing the operator is complaining about and
        // it should be in the log with the device and the route on it.
        let started = std::time::Instant::now();
        let response = request
            .send()
            .await
            .with_context(|| format!("gọi agent {suffix}"))?;
        let elapsed = started.elapsed();
        if elapsed >= slow_call_budget(suffix) {
            tracing::warn!(
                serial = %self.serial,
                route = suffix,
                ms = elapsed.as_millis() as u64,
                "agent call was slow"
            );
        }
        let status = response.status();
        // Read as bytes and decode UTF-8 ourselves. The server answers without a
        // charset for some routes, and letting a client guess is exactly how
        // Vietnamese turns into `Xin chÃ o`.
        let bytes = response
            .bytes()
            .await
            .with_context(|| format!("đọc phản hồi agent {suffix}"))?;
        let text = String::from_utf8_lossy(&bytes);
        let value: Value = serde_json::from_str(&text)
            .with_context(|| format!("agent {suffix} trả về không phải JSON: {text}"))?;
        if !status.is_success() {
            let message = value
                .pointer("/value/message")
                .and_then(Value::as_str)
                .unwrap_or(text.as_ref());
            return Err(anyhow!("agent {suffix} lỗi {status}: {message}"));
        }
        Ok(value)
    }

    /// Find one element. `Ok(None)` when it is simply not on screen — an absent
    /// element is an observation, not a transport failure, and callers branch
    /// on it (a LIVE post genuinely has no like button).
    pub async fn find(&self, locator: &Locator) -> anyhow::Result<Option<String>> {
        match self
            .send(reqwest::Method::POST, "/element", Some(locator.to_body()))
            .await
        {
            Ok(value) => Ok(element_id(&value)),
            Err(error) => {
                let text = error.to_string();
                if text.contains("no such element") || text.contains("could not be located") {
                    Ok(None)
                } else {
                    Err(error)
                }
            }
        }
    }

    /// Every element matching, in the order the server walks the tree.
    ///
    /// An empty vector is an observation, not a failure — the same rule
    /// [`Self::find`] follows.
    pub async fn find_all(&self, locator: &Locator) -> anyhow::Result<Vec<String>> {
        let value = match self
            .send(reqwest::Method::POST, "/elements", Some(locator.to_body()))
            .await
        {
            Ok(value) => value,
            Err(error) => {
                let text = error.to_string();
                if text.contains("no such element") || text.contains("could not be located") {
                    return Ok(Vec::new());
                }
                return Err(error);
            }
        };
        let entries = value
            .get("value")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(entries.iter().filter_map(element_id_from).collect())
    }

    pub async fn require(&self, locator: &Locator) -> anyhow::Result<String> {
        self.find(locator)
            .await?
            .ok_or_else(|| anyhow!("không thấy phần tử {locator:?} trên màn hình"))
    }

    pub async fn rect(&self, element: &str) -> anyhow::Result<Rect> {
        let value = self
            .send(
                reqwest::Method::GET,
                &format!("/element/{element}/rect"),
                None,
            )
            .await?;
        let rect = value
            .get("value")
            .ok_or_else(|| anyhow!("phản hồi rect thiếu value"))?;
        let number = |key: &str| -> anyhow::Result<f64> {
            rect.get(key)
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow!("rect thiếu {key}"))
        };
        Ok(Rect {
            x: number("x")?,
            y: number("y")?,
            width: number("width")?,
            height: number("height")?,
        })
    }

    pub async fn click(&self, element: &str) -> anyhow::Result<()> {
        self.send(
            reqwest::Method::POST,
            &format!("/element/{element}/click"),
            Some(json!({})),
        )
        .await
        .map(|_| ())
    }

    pub async fn text(&self, element: &str) -> anyhow::Result<String> {
        let value = self
            .send(
                reqwest::Method::GET,
                &format!("/element/{element}/text"),
                None,
            )
            .await?;
        Ok(value
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    pub async fn attribute(&self, element: &str, name: &str) -> anyhow::Result<Option<String>> {
        let value = self
            .send(
                reqwest::Method::GET,
                &format!("/element/{element}/attribute/{name}"),
                None,
            )
            .await?;
        Ok(value
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    /// Set an element's text through accessibility `ACTION_SET_TEXT`.
    ///
    /// This is the whole answer to Vietnamese input, and it removes the custom
    /// IME the plan had budgeted a phase for. `adb shell input text` cannot do
    /// it at all — with diacritics the process is killed outright.
    pub async fn set_text(&self, element: &str, text: &str) -> anyhow::Result<()> {
        self.send(
            reqwest::Method::POST,
            &format!("/element/{element}/value"),
            Some(json!({ "text": text })),
        )
        .await
        .map(|_| ())
    }

    pub async fn clear(&self, element: &str) -> anyhow::Result<()> {
        self.send(
            reqwest::Method::POST,
            &format!("/element/{element}/clear"),
            Some(json!({})),
        )
        .await
        .map(|_| ())
    }

    /// A real touch at a device-pixel point, via W3C pointer actions.
    ///
    /// Deliberately not `ACTION_CLICK`: an accessibility click is trivially
    /// distinguishable from a person and bypasses the gesture layer the
    /// touch-jitter work depends on.
    /// One tap, with a contact time and a drift rather than a fixed 60 ms hold.
    ///
    /// The old version held for exactly 60 ms and never moved while down, on every tap, on
    /// every device, forever. A finger does neither: contact is tens of milliseconds and
    /// varies, and the pad rolls a pixel or two against the glass before it lifts. Both are
    /// in the touch stream the app receives, and both were constant.
    ///
    /// The drift is deliberately smaller than the jitter
    /// [`TouchPointPlanner`](riviu_core::nurture) already applies to *where* the tap lands —
    /// this is the finger settling, not a different target, so it must never carry the point
    /// out of the control that was located.
    pub async fn tap(&self, x: f64, y: f64) -> anyhow::Result<()> {
        let (contact_ms, drift_x, drift_y) = {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            (
                rng.gen_range(45..=130),
                rng.gen_range(-2..=2),
                rng.gen_range(-2..=2),
            )
        };
        let actions = json!({
            "actions": [{
                "type": "pointer",
                "id": "finger1",
                "parameters": { "pointerType": "touch" },
                "actions": [
                    { "type": "pointerMove", "duration": 0, "x": x as i64, "y": y as i64 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pause", "duration": contact_ms },
                    // The roll happens under contact, so it is part of the same touch.
                    { "type": "pointerMove", "duration": 12,
                      "x": x as i64 + drift_x, "y": y as i64 + drift_y },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        });
        self.send(reqwest::Method::POST, "/actions", Some(actions))
            .await
            .map(|_| ())
    }

    /// Overlay tap: one short contact, no drift. Nurture must keep [`Self::tap`].
    pub async fn tap_direct(&self, x: f64, y: f64) -> anyhow::Result<()> {
        let actions = json!({
            "actions": [{
                "type": "pointer",
                "id": "finger1",
                "parameters": { "pointerType": "touch" },
                "actions": [
                    { "type": "pointerMove", "duration": 0, "x": x as i64, "y": y as i64 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pause", "duration": 16 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        });
        self.send(reqwest::Method::POST, "/actions", Some(actions))
            .await
            .map(|_| ())
    }

    /// A swipe as the planned path: one `pointerMove` per leg, then a settle before the lift.
    ///
    /// The whole reason [`riviu_core::types::SwipePath`] exists. [`Self::swipe`] sends a
    /// single move, which the framework reconstructs as a straight line at constant
    /// velocity; this sends the curve and the velocity profile that were planned, in the
    /// same one round trip.
    pub async fn swipe_path(&self, path: &riviu_core::types::SwipePath) -> anyhow::Result<()> {
        let mut actions = Vec::with_capacity(path.steps.len() + 3);
        actions.push(json!({
            "type": "pointerMove", "duration": 0,
            "x": path.start.x as i64, "y": path.start.y as i64
        }));
        actions.push(json!({ "type": "pointerDown", "button": 0 }));
        for step in &path.steps {
            actions.push(json!({
                "type": "pointerMove",
                "duration": step.duration_ms.max(1) as i64,
                "x": step.point.x as i64,
                "y": step.point.y as i64
            }));
        }
        if path.settle_ms > 0 {
            actions.push(json!({ "type": "pause", "duration": path.settle_ms as i64 }));
        }
        actions.push(json!({ "type": "pointerUp", "button": 0 }));
        let body = json!({
            "actions": [{
                "type": "pointer",
                "id": "finger1",
                "parameters": { "pointerType": "touch" },
                "actions": actions
            }]
        });
        self.send(reqwest::Method::POST, "/actions", Some(body))
            .await
            .map(|_| ())
    }

    pub async fn swipe(
        &self,
        from: (f64, f64),
        to: (f64, f64),
        duration_ms: u64,
    ) -> anyhow::Result<()> {
        let actions = json!({
            "actions": [{
                "type": "pointer",
                "id": "finger1",
                "parameters": { "pointerType": "touch" },
                "actions": [
                    { "type": "pointerMove", "duration": 0, "x": from.0 as i64, "y": from.1 as i64 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerMove", "duration": duration_ms.max(1) as i64,
                      "x": to.0 as i64, "y": to.1 as i64 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        });
        self.send(reqwest::Method::POST, "/actions", Some(actions))
            .await
            .map(|_| ())
    }

    pub async fn press_key(&self, keycode: i64) -> anyhow::Result<()> {
        self.send(
            reqwest::Method::POST,
            "/appium/device/press_keycode",
            Some(json!({ "keycode": keycode })),
        )
        .await
        .map(|_| ())
    }

    pub async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        let value = self
            .send(reqwest::Method::GET, "/window/current/size", None)
            .await?;
        let size = value
            .get("value")
            .ok_or_else(|| anyhow!("phản hồi window size thiếu value"))?;
        let width = size
            .get("width")
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow!("window size thiếu width"))?;
        let height = size
            .get("height")
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow!("window size thiếu height"))?;
        Ok((width, height))
    }

    /// Full hierarchy XML. **Measured at 3403 ms** — for diagnosis and evidence
    /// capture only. Never call this from a control loop; use [`Self::find`].
    pub async fn source(&self) -> anyhow::Result<String> {
        let value = self.send(reqwest::Method::GET, "/source", None).await?;
        Ok(value
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }
}

fn element_id(value: &Value) -> Option<String> {
    element_id_from(value.get("value")?)
}

/// The id out of one element object, W3C key or the legacy one.
///
/// Split out because `/elements` returns an **array of these**, unwrapped, while
/// `/element` wraps a single one under `value`.
fn element_id_from(element: &Value) -> Option<String> {
    if let Some(id) = element.get(W3C_ELEMENT_KEY).and_then(Value::as_str) {
        return Some(id.to_string());
    }
    if let Some(id) = element.get("ELEMENT").and_then(Value::as_str) {
        return Some(id.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_tree_retry_allowlist_contains_reads_and_excludes_effects() {
        for (method, route) in [
            (reqwest::Method::POST, "/element"),
            (reqwest::Method::POST, "/elements"),
            (reqwest::Method::GET, "/source"),
            (
                reqwest::Method::GET,
                "/element/fixture/attribute/content-desc",
            ),
            (reqwest::Method::GET, "/element/fixture/rect"),
            (reqwest::Method::GET, "/element/fixture/text"),
        ] {
            assert!(
                stale_tree_retry_is_safe(&method, route),
                "read-only route {method} {route} lost stale-tree recovery"
            );
        }
        for (method, route) in [
            (reqwest::Method::POST, "/actions"),
            (reqwest::Method::POST, "/element/fixture/click"),
            (reqwest::Method::POST, "/element/fixture/value"),
            (reqwest::Method::POST, "/element/fixture/clear"),
            (reqwest::Method::POST, "/appium/device/press_keycode"),
            (reqwest::Method::DELETE, ""),
        ] {
            assert!(
                !stale_tree_retry_is_safe(&method, route),
                "effectful route {method} {route} must never be reissued"
            );
        }
    }

    #[tokio::test]
    async fn stale_tree_action_error_sends_exactly_one_http_request() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind fixture agent");
        let base = format!("http://{}", listener.local_addr().expect("fixture address"));
        let calls = Arc::new(AtomicUsize::new(0));
        let server_calls = Arc::clone(&calls);
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                server_calls.fetch_add(1, Ordering::SeqCst);
                let mut request = [0_u8; 8_192];
                let _ = socket.read(&mut request).await;
                let body =
                    format!(r#"{{"value":{{"message":"{STALE_TREE_MARKER} after 10000 ms"}}}}"#);
                let response = format!(
                    "HTTP/1.1 500 Internal Server Error\r\n\
                     Content-Type: application/json\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        let client = AgentClient {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .expect("fixture client"),
            base,
            serial: "fixture-device".to_owned(),
            session_id: Arc::new(Mutex::new("fixture-session".to_owned())),
        };

        let error = client
            .send(
                reqwest::Method::POST,
                "/actions",
                Some(json!({ "actions": [] })),
            )
            .await
            .expect_err("stale-tree gesture must remain uncertain");
        assert!(error.to_string().contains(STALE_TREE_MARKER));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "one action intent must issue exactly one HTTP request"
        );
        server.abort();
        let _ = server.await;
    }

    #[test]
    fn description_uses_the_accessibility_id_strategy() {
        let body = Locator::Description("Like".into()).to_body();
        assert_eq!(body["strategy"], "accessibility id");
        assert_eq!(body["selector"], "Like");
    }

    #[test]
    fn contains_is_needed_because_tiktok_embeds_counts_in_labels() {
        // The real label is "Read or add comments. 15 comments", so an exact
        // match on "comments" would never hit.
        let body = Locator::DescriptionContains("comments".into()).to_body();
        assert_eq!(body["strategy"], "-android uiautomator");
        assert_eq!(
            body["selector"],
            "new UiSelector().descriptionContains(\"comments\")"
        );
    }

    #[test]
    fn focused_narrows_to_the_field_that_actually_has_focus() {
        // Two EditTexts exist while the comment drawer is open; without this
        // the hidden one wins and typing lands nowhere visible.
        let body = Locator::ClassName("android.widget.EditText".into())
            .focused()
            .to_body();
        assert_eq!(body["strategy"], "-android uiautomator");
        assert_eq!(
            body["selector"],
            "new UiSelector().className(\"android.widget.EditText\").focused(true)"
        );
    }

    #[test]
    fn focused_does_not_double_wrap_a_raw_selector() {
        let body = Locator::UiSelector("new UiSelector().index(3)".into())
            .focused()
            .to_body();
        assert_eq!(body["selector"], "new UiSelector().index(3).focused(true)");
    }

    #[test]
    fn java_quoting_escapes_quotes_and_backslashes() {
        assert_eq!(quote_java(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_java(r"a\b"), r#""a\\b""#);
        assert_eq!(quote_java("Xin chào"), "\"Xin chào\"");
    }

    #[test]
    fn element_id_accepts_both_w3c_and_legacy_shapes() {
        let w3c = json!({ "value": { W3C_ELEMENT_KEY: "abc" } });
        assert_eq!(element_id(&w3c).as_deref(), Some("abc"));
        let legacy = json!({ "value": { "ELEMENT": "def" } });
        assert_eq!(element_id(&legacy).as_deref(), Some("def"));
        assert_eq!(element_id(&json!({ "value": {} })), None);
    }

    #[test]
    fn rect_centre_is_the_middle() {
        let rect = Rect {
            x: 189.0,
            y: 919.0,
            width: 822.0,
            height: 137.0,
        };
        assert_eq!(rect.centre(), (600.0, 987.5));
    }
}

#[cfg(test)]
mod slow_call_budget_tests {
    use super::*;

    /// **A tree read is not a sluggish tap, and the log paid for confusing them.**
    ///
    /// Measured over one release log: `/element` produced **9 059** warning lines at p50 888 ms
    /// against a 500 ms budget set for a tap. Every route that walks the accessibility tree gets
    /// the tree's budget — including the `/element/<uuid>/…` attribute and rect reads, which are
    /// the same walk with a longer path and were the easiest ones to miss.
    #[test]
    fn every_tree_route_gets_the_tree_budget() {
        for route in [
            "/element",
            "/elements",
            "/source",
            "/element/00000000-0000-0114-ffff-ffff00000000/rect",
            "/element/00000000-0000-014c-ffff-ffff00000000/attribute/content-desc",
        ] {
            assert_eq!(
                slow_call_budget(route),
                SLOW_TREE_READ,
                "{route} walks the accessibility tree"
            );
        }
    }

    /// A gesture keeps the tight budget, because that is the one the number was measured for.
    #[test]
    fn a_gesture_keeps_the_gesture_budget() {
        for route in ["/actions", "", "/window/current/size", "/session"] {
            assert_eq!(
                slow_call_budget(route),
                SLOW_AGENT_CALL,
                "{route} is not a tree read"
            );
        }
    }

    /// **The budgets must not converge**, or the split above is decoration and the 9 059 lines
    /// come straight back the next time somebody "tidies" one of the two constants.
    #[test]
    fn the_tree_budget_is_far_above_the_gesture_budget() {
        assert!(
            SLOW_TREE_READ >= SLOW_AGENT_CALL * 4,
            "a tree read routinely costs ~900 ms on this fleet; a budget near the gesture's \
             prints a line for normal work"
        );
    }

    /// The measured p90s, as a line-count budget rather than a hope.
    ///
    /// Replays the distribution the release log actually held — `/element` p50 888 ms and p90
    /// 2 520 ms, `/actions` p50 624 ms — and asserts the new budgets keep the tree routes quiet
    /// at their p90 while still reporting the tail that hurts.
    #[test]
    fn the_measured_p90_of_a_tree_read_no_longer_prints() {
        let element_p90 = Duration::from_millis(2_520);
        let elements_p90 = Duration::from_millis(4_403);
        assert!(element_p90 < slow_call_budget("/element"));
        assert!(elements_p90 < slow_call_budget("/elements"));
        // The tail still does.
        assert!(Duration::from_millis(19_938) >= slow_call_budget("/element"));
        // And a tap that got slow still does.
        assert!(Duration::from_millis(624) >= slow_call_budget("/actions"));
    }
}
