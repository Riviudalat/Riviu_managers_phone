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

use std::time::Duration;

use anyhow::{anyhow, Context};
use serde_json::{json, Value};

/// W3C returns the element id under this key; older servers use `ELEMENT`.
const W3C_ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

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
    /// Fully-qualified `resource-id`. Obfuscated in TikTok; use sparingly.
    ResourceId(String),
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
            Self::ResourceId(value) => json!({
                "strategy": "id",
                "selector": value,
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
            Self::ResourceId(value) => {
                format!("new UiSelector().resourceId({})", quote_java(&value))
            }
            Self::UiSelector(expression) => expression,
        };
        Self::UiSelector(format!("{inner}.focused(true)"))
    }
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

#[derive(Clone)]
pub struct AgentClient {
    http: reqwest::Client,
    base: String,
    session_id: String,
}

impl AgentClient {
    /// Open a session against an agent already listening on `base`.
    pub async fn connect(base: impl Into<String>) -> anyhow::Result<Self> {
        let base = base.into().trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
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
            session_id,
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
    async fn prime_session(&self) -> anyhow::Result<()> {
        self.send(
            reqwest::Method::POST,
            "/appium/settings",
            Some(json!({ "settings": { "waitForIdleTimeout": 0 } })),
        )
        .await
        .map(|_| ())
    }

    /// Is the agent listening at all? Cheap; used for liveness.
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
        format!("{}/session/{}{suffix}", self.base, self.session_id)
    }

    async fn send(
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
        let response = request
            .send()
            .await
            .with_context(|| format!("gọi agent {suffix}"))?;
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
    pub async fn tap(&self, x: f64, y: f64) -> anyhow::Result<()> {
        let actions = json!({
            "actions": [{
                "type": "pointer",
                "id": "finger1",
                "parameters": { "pointerType": "touch" },
                "actions": [
                    { "type": "pointerMove", "duration": 0, "x": x as i64, "y": y as i64 },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pause", "duration": 60 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        });
        self.send(reqwest::Method::POST, "/actions", Some(actions))
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
    let inner = value.get("value")?;
    if let Some(id) = inner.get(W3C_ELEMENT_KEY).and_then(Value::as_str) {
        return Some(id.to_string());
    }
    if let Some(id) = inner.get("ELEMENT").and_then(Value::as_str) {
        return Some(id.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
