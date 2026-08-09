//! `UiSession` over the resident agent.

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use riviu_core::driver::UiSession;
use riviu_core::flow::model::{ElementLocatorStrategy, QualifiedElementLocator};
use riviu_core::{SwipeGesture, TapPoint};

use crate::adb::AdbProgram;
use crate::agent::{AgentClient, Locator};

/// `KEYCODE_HOME`.
const KEYCODE_HOME: i64 = 3;

pub struct AndroidUiSession {
    agent: AgentClient,
    adb: AdbProgram,
    serial: String,
    /// Rendered screen size in device pixels — the *override* size, which is
    /// what everything on screen is measured in.
    screen: (f64, f64),
}

impl AndroidUiSession {
    pub fn new(agent: AgentClient, adb: AdbProgram, serial: String, screen: (f64, f64)) -> Self {
        Self {
            agent,
            adb,
            serial,
            screen,
        }
    }

    pub fn agent(&self) -> &AgentClient {
        &self.agent
    }

    /// Resolve a locator to its on-screen rectangle.
    ///
    /// This is the bridge the engine actually wants. Nurture does not want
    /// "press like", it wants *where* like is, so the existing touch-jitter
    /// planner can pick a human-looking point inside it.
    pub async fn find_bounds(
        &self,
        locator: &Locator,
    ) -> anyhow::Result<Option<crate::agent::Rect>> {
        let Some(element) = self.agent.find(locator).await? else {
            return Ok(None);
        };
        self.agent.rect(&element).await.map(Some)
    }

    fn image_to_screen(&self, x: f64, y: f64, image_w: f64, image_h: f64) -> (f64, f64) {
        scale_to_screen(x, y, image_w, image_h, self.screen)
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

    async fn tap_image(&self, x: f64, y: f64, image_w: f64, image_h: f64) -> anyhow::Result<()> {
        let (x, y) = self.image_to_screen(x, y, image_w, image_h);
        self.agent.tap(x, y).await
    }

    async fn swipe_image(
        &self,
        from: TapPoint,
        to: TapPoint,
        image_w: f64,
        image_h: f64,
        duration_ms: u64,
    ) -> anyhow::Result<()> {
        let from = self.image_to_screen(from.x, from.y, image_w, image_h);
        let to = self.image_to_screen(to.x, to.y, image_w, image_h);
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

    /// True, and measured rather than assumed: the text is read back off the
    /// field before anything is sent.
    fn supports_text_input(&self) -> bool {
        true
    }

    async fn home(&self) -> anyhow::Result<()> {
        self.agent.press_key(KEYCODE_HOME).await
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

    async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        Ok(self.screen)
    }

    async fn launch_app_foreground(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.adb
            .shell(
                &self.serial,
                &format!("monkey -p {bundle_id} -c android.intent.category.LAUNCHER 1"),
            )
            .await
            .map(|_| ())
    }

    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        let stdout = self
            .adb
            .shell(&self.serial, "dumpsys window windows | grep mCurrentFocus")
            .await?;
        crate::adb::parse_current_focus_package(&stdout)
            .ok_or_else(|| anyhow!("could not read the foreground package"))
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

    fn stream_url(&self) -> Option<String> {
        // No MJPEG producer yet. Frames are deliberately deferred: with
        // hierarchy-based location they are corroboration, not the locator.
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
    fn a_degenerate_image_size_passes_the_point_through_instead_of_producing_nan() {
        let screen = (1080.0, 2220.0);
        assert_eq!(scale_to_screen(10.0, 20.0, 0.0, 0.0, screen), (10.0, 20.0));
        assert_eq!(scale_to_screen(10.0, 20.0, -1.0, 5.0, screen), (10.0, 20.0));
    }
}
