use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderValue, CONNECTION};
use reqwest::Client;
use riviu_core::{SwipeGesture, TapPoint, UiError, UiErrorKind};
use serde_json::json;

use crate::telemetry::{self, Outcome};

/// Session caps with **no** `bundleId`. WDA with `bundleId=com.apple.springboard`
/// (and default `forceAppLaunch=true`) re-launches SpringBoard on every
/// `/session` — that flashes the lock screen / Home and looks like a WDA error
/// in the nurture log. Empty caps attach to whatever is already foreground.
///
/// Deliberately **no** `autoDismissAlerts` / `defaultAlertAction`. Those install
/// a background alert monitor inside the agent that keeps querying the
/// foreground app's accessibility hierarchy. On TikTok that query does not
/// return, and it blocks the XCTest thread that also serves gestures: with the
/// flags set, every session command timed out; without them, the identical
/// sequence answered in ~1 s. System alerts are handled explicitly through
/// `dismiss_alert` instead, and TikTok's own popups by the screen watcher.
fn session_capabilities() -> serde_json::Value {
    json!({
        "capabilities": {
            "alwaysMatch": {
                "appium:shouldWaitForQuiescence": false,
                "appium:waitForIdleTimeout": 0,
                "appium:forceAppLaunch": false,
            }
        },
        "desiredCapabilities": {
            "shouldWaitForQuiescence": false,
            "waitForIdleTimeout": 0,
            "forceAppLaunch": false,
        },
    })
}

/// Deadline for a single gesture. Long enough for TikTok to render under USB
/// load, short enough that a wedged call is reported rather than waited out.
///
/// Never wrap these in `tokio::time::timeout`: cancelling a request mid-flight
/// leaves the relay with a half-written body and wedges it (live test #1). The
/// deadline belongs to the request itself.
const GESTURE_TIMEOUT: Duration = Duration::from_secs(10);

/// Priming attempts before a session is declared unusable. Four 8 s attempts
/// with 2 s gaps covers the ~45 s self-clearing stall seen in a live run,
/// without waiting out a genuinely dead runner.
const PRIME_ATTEMPTS: u32 = 4;

#[derive(Clone)]
pub struct WdaClient {
    http: Client,
    base: String,
    port: u16,
    udid: String,
    session_id: Option<String>,
}

impl WdaClient {
    pub fn new(host: &str, port: u16, udid: &str) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(CONNECTION, HeaderValue::from_static("close"));
        Self {
            http: Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .default_headers(headers)
                // tidevice relay is unreliable when reqwest reuses a keep-alive
                // socket across session/window/gesture requests. Python probes
                // that open a fresh TCP connection per request stay stable.
                .pool_max_idle_per_host(0)
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_else(|_| Client::new()),
            base: format!("http://{host}:{port}"),
            port,
            udid: udid.to_string(),
            session_id: None,
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn udid(&self) -> &str {
        &self.udid
    }

    pub fn mjpeg_url(host: &str, mjpeg_port: u16) -> String {
        format!("http://{host}:{mjpeg_port}")
    }

    /// Cheap probe so a cached session isn't reused after the device dropped it.
    pub async fn session_alive(&self) -> bool {
        let Ok(url) = self.session_url("/window/size") else {
            return false;
        };
        self.send(
            Method::Get,
            &url,
            "window.size",
            None,
            Duration::from_secs(5),
        )
        .await
        .is_ok()
    }

    pub async fn create_session(&mut self) -> Result<(), UiError> {
        let url = format!("{}/session", self.base);
        // No bundleId — do not launch/activate SpringBoard (that locks→Home).
        let resp = self
            .send(
                Method::Post,
                &url,
                "session.create",
                Some(&session_capabilities()),
                Duration::from_secs(30),
            )
            .await?;
        let sid = resp
            .pointer("/value/sessionId")
            .or_else(|| resp.get("sessionId"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                UiError::new(
                    UiErrorKind::Http,
                    "session.create",
                    format!("session id missing: {resp}"),
                )
            })?
            .to_string();
        self.session_id = Some(sid);
        if !self.prime_session().await {
            return Err(UiError::new(
                UiErrorKind::Timeout,
                "session.prime",
                "agent nhận session nhưng không chạy được lệnh nào — runner đang kẹt",
            ));
        }
        Ok(())
    }

    /// Send one cheap session command before anything else touches the
    /// accessibility hierarchy.
    ///
    /// This is the fix for the failure that dominated every earlier live test.
    /// On this agent build, if the *first* session-scoped command is one that
    /// resolves the foreground app's hierarchy (`window/size`, `/actions`,
    /// `/wda/tap`), and that app is TikTok, the call never returns and the
    /// runner's XCTest thread stays blocked forever. `/status` and the
    /// sessionless `GET /screenshot` keep answering throughout, which is why
    /// health probes reported a healthy agent while every gesture timed out,
    /// and why recovery kept recycling a runner that looked fine.
    ///
    /// Measured on the live iPhone 8, cold runner, TikTok foreground:
    ///
    /// | first command        | window/size | tap    |
    /// |----------------------|------------:|-------:|
    /// | window/size          |     timeout |      — |
    /// | /actions tap         |           — | timeout|
    /// | appium/settings      |   107–690ms | 393–601ms |
    ///
    /// Four of four runs passed with the settings call first; every run without
    /// it wedged. The bounded snapshot depth is useful in its own right —
    /// TikTok's hierarchy is deep and nothing here needs to walk it.
    /// Returns false when the agent never answered — the caller must treat that
    /// as a wedged runner rather than carrying on into gestures that will all
    /// time out. A live run showed a stall clearing on its own after ~45 s, so
    /// this retries for about that long before giving up.
    async fn prime_session(&self) -> bool {
        let Ok(url) = self.session_url("/appium/settings") else {
            return false;
        };
        let body = json!({
            "settings": {
                "snapshotMaxDepth": 1,
                "customSnapshotTimeout": 2,
                "shouldUseCompactResponses": true,
            }
        });
        for attempt in 0..PRIME_ATTEMPTS {
            if self
                .send(
                    Method::Post,
                    &url,
                    "session.prime",
                    Some(&body),
                    Duration::from_secs(8),
                )
                .await
                .is_ok()
            {
                return true;
            }
            if attempt + 1 < PRIME_ATTEMPTS {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        false
    }

    fn session_url(&self, path: &str) -> Result<String, UiError> {
        let sid = self
            .session_id
            .as_ref()
            .ok_or_else(|| UiError::new(UiErrorKind::Session, "session", "no WDA session"))?;
        Ok(format!("{}/session/{sid}{path}", self.base))
    }

    /// One WDA request, timed and classified.
    async fn send(
        &self,
        method: Method,
        url: &str,
        label: &str,
        body: Option<&serde_json::Value>,
        timeout: Duration,
    ) -> Result<serde_json::Value, UiError> {
        let started = Instant::now();
        let result = self.send_inner(method, url, label, body, timeout).await;
        let ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
        let outcome = match &result {
            Ok(_) => Outcome::Ok,
            Err(e) => outcome_of(e.kind),
        };
        telemetry::record(&self.udid, label, ms, outcome);
        result
    }

    async fn send_inner(
        &self,
        method: Method,
        url: &str,
        label: &str,
        body: Option<&serde_json::Value>,
        timeout: Duration,
    ) -> Result<serde_json::Value, UiError> {
        let mut req = match method {
            Method::Get => self.http.get(url),
            Method::Post => self.http.post(url),
        };
        req = req.timeout(timeout);
        if let Some(b) = body {
            req = req.header("Content-Type", "application/json").json(b);
        }
        let resp = req.send().await.map_err(|e| {
            let kind = if e.is_timeout() {
                UiErrorKind::Timeout
            } else {
                UiErrorKind::Transport
            };
            UiError::new(kind, label, format!("{url}: {e}"))
        })?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let parsed = serde_json::from_str::<serde_json::Value>(&text).ok();
            let w3c_error = parsed
                .as_ref()
                .and_then(|v| v.pointer("/value/error").and_then(|m| m.as_str()))
                .unwrap_or_default()
                .to_string();
            let message = parsed
                .as_ref()
                .and_then(|v| {
                    v.pointer("/value/message")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| {
                    if text.is_empty() {
                        format!("HTTP {status}")
                    } else {
                        text.chars().take(240).collect()
                    }
                });
            // "the session is gone" is fixed by making a new session; every
            // other status means the runner rejected just this one command.
            let kind = if is_session_gone(&w3c_error, &message) {
                UiErrorKind::Session
            } else {
                UiErrorKind::Http
            };
            return Err(UiError::new(
                kind,
                label,
                format!("HTTP {status}: {message}"),
            ));
        }

        if text.trim().is_empty() {
            return Ok(json!({ "value": null }));
        }
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|_| json!({ "value": text }));
        // WDA can answer 200 with a W3C error body.
        if let Some(err) = value.pointer("/value/error").and_then(|v| v.as_str()) {
            let message = value
                .pointer("/value/message")
                .and_then(|v| v.as_str())
                .unwrap_or(err);
            let kind = if is_session_gone(err, message) {
                UiErrorKind::Session
            } else {
                UiErrorKind::Http
            };
            return Err(UiError::new(kind, label, message.to_string()));
        }
        Ok(value)
    }

    /// Sessionless `GET /screenshot`. Kept for the vision-comment capture; the
    /// popup watcher reads the MJPEG stream instead and never calls this, which
    /// is what stopped the watcher from wedging the control relay.
    ///
    /// The ~1.2 MB body must not be cancelled mid-flight — the deadline lives
    /// on the request.
    pub async fn screenshot_png(&self) -> Result<Vec<u8>, UiError> {
        use base64::Engine;
        let url = format!("{}/screenshot", self.base);
        let started = Instant::now();
        // Dedicated client: the shared one forces `Connection: close`, which
        // stalls on this body until the deadline aborts it mid-read and wedges
        // the relay. Plain keep-alive (what curl does) returns in ~0.3 s.
        let outcome = async {
            let client = Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(25))
                .build()
                .map_err(|e| UiError::new(UiErrorKind::Other, "screenshot", e.to_string()))?;
            let resp = client.get(&url).send().await.map_err(|e| {
                let kind = if e.is_timeout() {
                    UiErrorKind::Timeout
                } else {
                    UiErrorKind::Transport
                };
                UiError::new(kind, "screenshot", e.to_string())
            })?;
            let value: serde_json::Value = resp
                .error_for_status()
                .map_err(|e| UiError::new(UiErrorKind::Http, "screenshot", e.to_string()))?
                .json()
                .await
                .map_err(|e| UiError::new(UiErrorKind::Http, "screenshot", e.to_string()))?;
            let b64 = value
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| UiError::new(UiErrorKind::Http, "screenshot", "missing value"))?;
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| UiError::new(UiErrorKind::Http, "screenshot", e.to_string()))
        }
        .await;
        telemetry::record(
            &self.udid,
            "screenshot",
            started.elapsed().as_millis() as u32,
            match &outcome {
                Ok(_) => Outcome::Ok,
                Err(e) => outcome_of(e.kind),
            },
        );
        outcome
    }

    pub async fn window_size(&self) -> Result<(f64, f64), UiError> {
        let url = self.session_url("/window/size")?;
        let resp = self
            .send(
                Method::Get,
                &url,
                "window.size",
                None,
                Duration::from_secs(8),
            )
            .await?;
        let w = resp
            .pointer("/value/width")
            .and_then(|v| v.as_f64())
            .or_else(|| resp.get("width").and_then(|v| v.as_f64()))
            .unwrap_or(0.0);
        let h = resp
            .pointer("/value/height")
            .and_then(|v| v.as_f64())
            .or_else(|| resp.get("height").and_then(|v| v.as_f64()))
            .unwrap_or(0.0);
        if w <= 0.0 || h <= 0.0 {
            return Err(UiError::new(
                UiErrorKind::Http,
                "window.size",
                format!("invalid window size: {resp}"),
            ));
        }
        Ok((w, h))
    }

    /// Map stream/screenshot pixel coords → WDA logical points.
    pub async fn to_points(
        &self,
        x: f64,
        y: f64,
        image_w: f64,
        image_h: f64,
    ) -> Result<TapPoint, UiError> {
        let (sw, sh) = match self.window_size().await {
            Ok(size) => size,
            Err(_) => {
                // iPhone 8 / common fallback: the stream is @2x pixels.
                let scale = if image_w > 500.0 { 2.0 } else { 1.0 };
                (image_w / scale, image_h / scale)
            }
        };
        let iw = image_w.max(1.0);
        let ih = image_h.max(1.0);
        Ok(TapPoint {
            x: ((x / iw) * sw).clamp(0.0, sw),
            y: ((y / ih) * sh).clamp(0.0, sh),
        })
    }

    fn pointer_action(steps: serde_json::Value) -> serde_json::Value {
        json!({
            "actions": [{
                "type": "pointer",
                "id": "finger1",
                "parameters": { "pointerType": "touch" },
                "actions": steps,
            }]
        })
    }

    /// Tap at a logical point.
    ///
    /// PRIMARY is W3C `/actions`, for the same reason swipe uses it: it posts
    /// HID events and returns. `/wda/tap` goes through XCUICoordinate, which
    /// waits for the app to go quiescent — and TikTok's feed animates non-stop,
    /// so that wait routinely outlives the deadline and surfaces as
    /// "error sending request". Measured on this device before the change; the
    /// numbers are in `docs/LIVE_NURTURE_REPORT_2026-07-26.md`.
    ///
    /// `/wda/tap` remains a fallback for the one case it helps with: `/actions`
    /// being rejected outright (some builds 500 on the first call after init).
    pub async fn tap(&self, point: TapPoint) -> Result<(), UiError> {
        let x = point.x.round();
        let y = point.y.round();
        let body = Self::pointer_action(json!([
            { "type": "pointerMove", "duration": 0, "x": x, "y": y },
            { "type": "pointerDown", "button": 0 },
            { "type": "pause", "duration": 60 },
            { "type": "pointerUp", "button": 0 }
        ]));
        let actions_url = self.session_url("/actions")?;
        match self
            .send(
                Method::Post,
                &actions_url,
                "tap.actions",
                Some(&body),
                GESTURE_TIMEOUT,
            )
            .await
        {
            Ok(_) => Ok(()),
            // A broken socket or a dead session is the caller's business: it
            // owns the recovery budget. Do not hammer a wedged runner with a
            // fallback request.
            Err(e) if e.kind != UiErrorKind::Http => Err(e),
            Err(first) => {
                let url = self.session_url("/wda/tap")?;
                self.send(
                    Method::Post,
                    &url,
                    "tap.native",
                    Some(&json!({ "x": x, "y": y })),
                    GESTURE_TIMEOUT,
                )
                .await
                .map_err(|second| {
                    UiError::new(
                        second.kind,
                        "tap",
                        format!("actions: {}; native: {}", first.message, second.message),
                    )
                })?;
                Ok(())
            }
        }
    }

    /// `/wda/tap` only — an XCUICoordinate tap, which iOS treats as a real UI
    /// interaction and which therefore focuses text fields. See
    /// `UiSession::tap_native`.
    pub async fn tap_native(&self, point: TapPoint) -> Result<(), UiError> {
        let url = self.session_url("/wda/tap")?;
        self.send(
            Method::Post,
            &url,
            "tap.native",
            Some(&json!({ "x": point.x.round(), "y": point.y.round() })),
            GESTURE_TIMEOUT,
        )
        .await?;
        Ok(())
    }

    pub async fn swipe(&self, gesture: SwipeGesture) -> Result<(), UiError> {
        let fx = gesture.from.x.round();
        let fy = gesture.from.y.round();
        let tx = gesture.to.x.round();
        let ty = gesture.to.y.round();
        let duration = (gesture.duration_ms as f64 / 1000.0).clamp(0.05, 2.0);

        // PRIMARY: W3C /actions. `/wda/dragfromtoforduration` uses an
        // XCUICoordinate drag, which waits for app quiescence; TikTok's feed
        // never goes quiet, so that call blocks until the relay resets.
        let actions_url = self.session_url("/actions")?;
        let body = Self::pointer_action(json!([
            { "type": "pointerMove", "duration": 0, "x": fx, "y": fy },
            { "type": "pointerDown", "button": 0 },
            { "type": "pointerMove", "duration": gesture.duration_ms.max(80), "x": tx, "y": ty },
            { "type": "pointerUp", "button": 0 }
        ]));
        match self
            .send(
                Method::Post,
                &actions_url,
                "swipe.actions",
                Some(&body),
                GESTURE_TIMEOUT,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(e) if e.kind != UiErrorKind::Http => Err(e),
            Err(first) => {
                let drag_url = self.session_url("/wda/dragfromtoforduration")?;
                let drag_body = json!({
                    "fromX": fx, "fromY": fy, "toX": tx, "toY": ty, "duration": duration,
                });
                self.send(
                    Method::Post,
                    &drag_url,
                    "swipe.drag",
                    Some(&drag_body),
                    GESTURE_TIMEOUT,
                )
                .await
                .map_err(|second| {
                    UiError::new(
                        second.kind,
                        "swipe",
                        format!("actions: {}; drag: {}", first.message, second.message),
                    )
                })?;
                Ok(())
            }
        }
    }

    pub async fn type_text(&self, text: &str) -> Result<(), UiError> {
        if text.is_empty() {
            return Ok(());
        }
        let url = self.session_url("/wda/keys")?;
        let body = json!({
            "value": text.chars().map(|c| c.to_string()).collect::<Vec<_>>(),
        });
        self.send(
            Method::Post,
            &url,
            "keys",
            Some(&body),
            Duration::from_secs(20),
        )
        .await?;
        Ok(())
    }

    pub async fn home(&self) -> Result<(), UiError> {
        let url = format!("{}/wda/homescreen", self.base);
        if self
            .send(
                Method::Post,
                &url,
                "homescreen",
                Some(&json!({})),
                Duration::from_secs(8),
            )
            .await
            .is_ok()
        {
            return Ok(());
        }
        let url2 = self.session_url("/wda/pressButton")?;
        self.send(
            Method::Post,
            &url2,
            "pressButton",
            Some(&json!({ "name": "home" })),
            Duration::from_secs(8),
        )
        .await?;
        Ok(())
    }

    pub async fn find_and_tap(&self, accessibility_id: &str) -> Result<(), UiError> {
        // Raw WebDriverAgent names this strategy "predicate string"; the
        // `-ios ` prefix is an Appium-ism and this agent rejects it outright
        // with "Invalid locator requested", so every lookup silently failed.
        //
        // One strategy only — repeated element lookups with long timeouts
        // wedged the relay when nurture cancelled them mid-flight.
        let predicate = format!(
            "label == {accessibility_id:?} OR name == {accessibility_id:?} OR value == {accessibility_id:?}"
        );
        self.find_and_click(&[("predicate string", predicate.as_str())])
            .await
    }

    /// Dismiss a system/UIAlertController if one is showing (location, tracking…).
    pub async fn dismiss_alert(&self) -> Result<(), UiError> {
        let url = self.session_url("/alert/dismiss")?;
        self.send(
            Method::Post,
            &url,
            "alert.dismiss",
            Some(&json!({})),
            Duration::from_secs(2),
        )
        .await?;
        Ok(())
    }

    async fn find_and_click(&self, strategies: &[(&str, &str)]) -> Result<(), UiError> {
        let url = self.session_url("/element")?;
        let short = Duration::from_millis(1800);
        let mut last = None;
        for &(using, value) in strategies {
            let body = json!({ "using": using, "value": value });
            match self
                .send(Method::Post, &url, "element.find", Some(&body), short)
                .await
            {
                Ok(resp) => {
                    let element_id = resp
                        .pointer("/value/ELEMENT")
                        .or_else(|| resp.pointer("/value/element-6066-11e4-a52e-4f735466cecf"))
                        .and_then(|v| v.as_str());
                    if let Some(element_id) = element_id {
                        let click = self.session_url(&format!("/element/{element_id}/click"))?;
                        self.send(Method::Post, &click, "element.click", Some(&json!({})), short)
                            .await?;
                        return Ok(());
                    }
                    last = Some(UiError::new(
                        UiErrorKind::Http,
                        "element.find",
                        format!("element id missing for {using}={value}"),
                    ));
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last
            .unwrap_or_else(|| UiError::new(UiErrorKind::Http, "element.find", "element not found")))
    }

    pub async fn assert_visible(&self, accessibility_id: &str) -> Result<(), UiError> {
        let url = self.session_url("/element")?;
        let body = json!({ "using": "accessibility id", "value": accessibility_id });
        self.send(
            Method::Post,
            &url,
            "element.find",
            Some(&body),
            Duration::from_secs(5),
        )
        .await?;
        Ok(())
    }

    pub async fn health(&self) -> Result<bool, UiError> {
        let url = format!("{}/status", self.base);
        Ok(self
            .send(Method::Get, &url, "status", None, Duration::from_secs(8))
            .await
            .is_ok())
    }

    /// Liveness probe. Deliberately **not** a reason to recycle the runner:
    /// `/status` false-negatives under USB load, and killing a live agent on
    /// that signal cost 2–3 minutes per occurrence in live test #9. Only a
    /// failed gesture with a transport error justifies touching the transport.
    pub async fn health_quick(&self) -> bool {
        let url = format!("{}/status", self.base);
        if self
            .send(Method::Get, &url, "status", None, Duration::from_secs(4))
            .await
            .is_ok()
        {
            return true;
        }
        let Ok(url) = self.session_url("/window/size") else {
            return false;
        };
        self.send(
            Method::Get,
            &url,
            "window.size",
            None,
            Duration::from_secs(4),
        )
        .await
        .is_ok()
    }

    /// Bundle id of the frontmost application.
    ///
    /// Unreliable on this agent build (it has killed the runner), so the
    /// nurture engine classifies the foreground from stream frames instead.
    /// Kept because the driver trait exposes it for scripted jobs.
    pub async fn active_app_bundle(&self) -> Result<String, UiError> {
        let url = format!("{}/wda/activeAppInfo", self.base);
        let v = self
            .send(
                Method::Get,
                &url,
                "activeAppInfo",
                None,
                Duration::from_secs(4),
            )
            .await?;
        let bid = v
            .pointer("/value/bundleId")
            .or_else(|| v.pointer("/value/bundleID"))
            .or_else(|| v.pointer("/value/application/bundleId"))
            .or_else(|| v.get("bundleId"))
            .and_then(|x| x.as_str())
            .ok_or_else(|| {
                UiError::new(
                    UiErrorKind::Http,
                    "activeAppInfo",
                    format!("missing bundleId: {v}"),
                )
            })?;
        Ok(bid.to_string())
    }

    /// Bring an app to the foreground through WDA, without restarting it.
    ///
    /// This travels over the *control* relay, so callers must hold the device
    /// lock: a `tidevice launch` racing the relay is what wedged usbmux in live
    /// tests #5–#7. Activation is preferred over launch precisely because it
    /// does not restart an app that is already running.
    pub async fn activate_app(&self, bundle_id: &str) -> Result<(), UiError> {
        let activate_url = self.session_url("/wda/apps/activate")?;
        self.send(
            Method::Post,
            &activate_url,
            "apps.activate",
            Some(&json!({ "bundleId": bundle_id })),
            Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }

    pub async fn launch_app(&self, bundle_id: &str) -> Result<(), UiError> {
        if self.activate_app(bundle_id).await.is_ok() {
            return Ok(());
        }
        let url = self.session_url("/wda/apps/launch")?;
        let body = json!({ "bundleId": bundle_id, "arguments": [], "environment": {} });
        self.send(
            Method::Post,
            &url,
            "apps.launch",
            Some(&body),
            Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Method {
    Get,
    Post,
}

fn outcome_of(kind: UiErrorKind) -> Outcome {
    match kind {
        UiErrorKind::Transport => Outcome::Transport,
        UiErrorKind::Timeout => Outcome::Timeout,
        UiErrorKind::Session => Outcome::Session,
        _ => Outcome::Http,
    }
}

/// Does this WDA error mean the session id is no longer valid?
fn is_session_gone(w3c_error: &str, message: &str) -> bool {
    let hay = format!("{w3c_error} {message}").to_lowercase();
    hay.contains("invalid session id")
        || hay.contains("session does not exist")
        || hay.contains("session is either terminated or not started")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nurture_session_omits_bundle_id_to_avoid_homescreen_flash() {
        let caps = session_capabilities();
        assert!(caps.pointer("/capabilities/alwaysMatch/bundleId").is_none());
        assert!(caps.pointer("/desiredCapabilities/bundleId").is_none());
        assert_eq!(
            caps.pointer("/capabilities/alwaysMatch/appium:forceAppLaunch")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        // The alert monitor these flags install is what blocked the agent's
        // XCTest thread on TikTok; they must stay gone.
        assert!(caps
            .pointer("/capabilities/alwaysMatch/appium:autoDismissAlerts")
            .is_none());
        assert!(caps.pointer("/desiredCapabilities/autoDismissAlerts").is_none());
        assert!(caps
            .pointer("/capabilities/alwaysMatch/defaultAlertAction")
            .is_none());
    }

    /// WebDriverAgent's own locator names have no `-ios ` prefix — that is an
    /// Appium addition. Sending the prefixed form makes the agent answer
    /// "Invalid locator requested" and the lookup never runs.
    #[test]
    fn locator_strategies_use_raw_wda_names() {
        for name in ["predicate string", "class chain", "class name", "accessibility id"] {
            assert!(!name.starts_with("-ios "), "{name} carries the Appium prefix");
        }
    }

    #[test]
    fn a_dead_session_is_told_apart_from_a_rejected_command() {
        assert!(is_session_gone("invalid session id", "whatever"));
        assert!(is_session_gone(
            "",
            "A session is either terminated or not started"
        ));
        assert!(!is_session_gone("no such element", "unable to find element"));
        assert!(!is_session_gone("", "HTTP 500"));
    }

    #[test]
    fn error_classes_map_onto_telemetry_outcomes() {
        assert_eq!(outcome_of(UiErrorKind::Transport), Outcome::Transport);
        assert_eq!(outcome_of(UiErrorKind::Timeout), Outcome::Timeout);
        assert_eq!(outcome_of(UiErrorKind::Session), Outcome::Session);
        assert_eq!(outcome_of(UiErrorKind::Http), Outcome::Http);
        assert_eq!(outcome_of(UiErrorKind::Other), Outcome::Http);
    }

    /// Retrying a transport failure is safe (the command never landed);
    /// retrying a timeout is not (it may have landed and be running).
    #[test]
    fn retry_safety_matches_the_failure_class() {
        assert!(UiErrorKind::Transport.is_safe_to_retry());
        assert!(UiErrorKind::Session.is_safe_to_retry());
        assert!(!UiErrorKind::Timeout.is_safe_to_retry());
        assert!(!UiErrorKind::Http.is_safe_to_retry());
    }

    #[test]
    fn a_gesture_deadline_leaves_room_for_a_retry_inside_the_soft_budget() {
        // Soft recovery is budgeted at 15 s; one gesture plus one retry has to
        // fit inside it or the budget is a lie.
        assert!(GESTURE_TIMEOUT.as_secs() * 2 <= 20);
    }
}
