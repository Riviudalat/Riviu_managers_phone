use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::types::{DeviceInfo, SwipeGesture, TapPoint};

/// Why a UI command failed. The nurture engine's recovery ladder turns on this
/// distinction: a rejected command means the runner is alive and only the
/// command was wrong, while a broken socket means the relay itself needs
/// attention. Collapsing both into "WDA unhealthy" is what produced the
/// 2–3 minute recycle spirals in earlier live tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiErrorKind {
    /// The socket never completed: connection refused/reset, relay wedged.
    /// The only class that justifies touching the transport.
    Transport,
    /// Accepted but no answer inside the deadline.
    Timeout,
    /// The agent says this session is gone — recreate the session, nothing else.
    Session,
    /// The agent answered with an error status. The runner is healthy.
    Http,
    /// Anything not produced by the UI transport.
    Other,
}

impl UiErrorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            UiErrorKind::Transport => "transport",
            UiErrorKind::Timeout => "timeout",
            UiErrorKind::Session => "session",
            UiErrorKind::Http => "http",
            UiErrorKind::Other => "other",
        }
    }

    /// Does this class mean the command definitely did not reach the device?
    /// Retrying one of these is safe; retrying a timeout may double-apply it.
    pub fn is_safe_to_retry(&self) -> bool {
        matches!(self, UiErrorKind::Transport | UiErrorKind::Session)
    }
}

/// A classified UI transport failure.
#[derive(Debug, Clone)]
pub struct UiError {
    pub kind: UiErrorKind,
    /// The command that failed, e.g. `tap` or `actions.swipe`.
    pub op: String,
    pub message: String,
}

impl UiError {
    pub fn new(kind: UiErrorKind, op: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind,
            op: op.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for UiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} [{}]: {}", self.op, self.kind.as_str(), self.message)
    }
}

impl std::error::Error for UiError {}

/// Classify any error coming out of a [`UiSession`] call. Errors raised by the
/// driver carry a [`UiError`]; anything else is [`UiErrorKind::Other`].
pub fn ui_error_kind(err: &anyhow::Error) -> UiErrorKind {
    err.downcast_ref::<UiError>()
        .map(|e| e.kind)
        .unwrap_or(UiErrorKind::Other)
}

#[async_trait]
pub trait DeviceDriver: Send + Sync {
    async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>>;
    async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo>;
    async fn install_app(&self, udid: &str, path: &Path) -> anyhow::Result<()>;
    async fn uninstall_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()>;
    async fn screenshot(&self, udid: &str, dest: &Path) -> anyhow::Result<PathBuf>;
    async fn syslog_tail(&self, udid: &str, lines: usize) -> anyhow::Result<String>;
    async fn launch_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()>;
    async fn terminate_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()>;
    async fn reboot(&self, udid: &str) -> anyhow::Result<()>;
    async fn start_ui_session(&self, udid: &str) -> anyhow::Result<Box<dyn UiSession>>;
    /// Is a usable UI session already cached for this device? Callers use it
    /// only to report honestly whether they reused an agent or started one.
    async fn ui_session_cached(&self, _udid: &str) -> bool {
        false
    }
    /// Drop cached WDA session so the next `start_ui_session` opens a fresh one.
    async fn invalidate_ui_session(&self, _udid: &str) {}
    /// Hard-recycle USB relay + device WDA runner (wedged Agent recovery).
    async fn recycle_ui_transport(&self, _udid: &str) {}
    async fn ensure_stream(&self, udid: &str) -> anyhow::Result<String>;
    async fn prepare_device(&self, udid: &str) -> anyhow::Result<()>;
}

#[async_trait]
pub trait UiSession: Send + Sync {
    async fn tap(&self, point: TapPoint) -> anyhow::Result<()>;
    /// Tap the way a finger does, through the UI hierarchy rather than by
    /// synthesising HID events.
    ///
    /// Text fields need this: a W3C `/actions` tap posts touch events that iOS
    /// does not turn into `becomeFirstResponder`, so tapping TikTok's comment
    /// box with it opens no keyboard and everything typed afterwards is lost.
    /// Slower than [`Self::tap`], so it is only for focus-sensitive targets.
    async fn tap_native(&self, point: TapPoint) -> anyhow::Result<()> {
        self.tap(point).await
    }
    async fn swipe(&self, gesture: SwipeGesture) -> anyhow::Result<()>;
    /// Tap using coordinates in stream/screenshot pixel space.
    async fn tap_image(
        &self,
        x: f64,
        y: f64,
        image_w: f64,
        image_h: f64,
    ) -> anyhow::Result<()> {
        let _ = (image_w, image_h);
        self.tap(TapPoint { x, y }).await
    }
    /// Swipe using coordinates in stream/screenshot pixel space.
    async fn swipe_image(
        &self,
        from: TapPoint,
        to: TapPoint,
        image_w: f64,
        image_h: f64,
        duration_ms: u64,
    ) -> anyhow::Result<()> {
        let _ = (image_w, image_h);
        self.swipe(SwipeGesture {
            from,
            to,
            duration_ms,
        })
        .await
    }
    async fn type_text(&self, text: &str) -> anyhow::Result<()>;
    async fn home(&self) -> anyhow::Result<()>;
    async fn find_and_tap(&self, accessibility_id: &str) -> anyhow::Result<()>;
    async fn assert_visible(&self, accessibility_id: &str) -> anyhow::Result<()>;
    /// Dismiss a visible iOS system alert if present. Default: unsupported.
    async fn dismiss_alert(&self) -> anyhow::Result<()> {
        anyhow::bail!("dismiss_alert not supported")
    }
    /// Cheap liveness probe (WDA `/status`). Default: assume healthy.
    async fn healthy(&self) -> bool {
        true
    }
    /// Screen size in points. Default: unknown.
    async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        anyhow::bail!("window_size not supported")
    }
    /// Bring app to foreground (WDA). Default: unsupported.
    async fn launch_app_foreground(&self, _bundle_id: &str) -> anyhow::Result<()> {
        anyhow::bail!("launch_app_foreground not supported")
    }
    /// Bundle id of the frontmost app (`GET /wda/activeAppInfo`). Default: unsupported.
    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        anyhow::bail!("active_app_bundle not supported")
    }
    /// Raw screen capture via the UI channel (WDA `GET /screenshot`).
    /// Cheap (~0.3s over USB) unlike the pymobiledevice3 path.
    async fn screenshot_png(&self) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("screenshot_png not supported")
    }
    fn stream_url(&self) -> Option<String>;
}
