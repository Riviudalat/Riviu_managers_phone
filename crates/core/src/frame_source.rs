//! Read-only access to the device screen stream that is already running.
//!
//! The nurture engine has to watch the screen continuously to catch popups the
//! moment they appear. It must not do that with WDA `GET /screenshot`: polling
//! the control relay wedged the USB tunnel repeatedly in live tests #1 and #4–5.
//!
//! The MJPEG stream is a *separate* usbmux channel (device port 9100) that the
//! app already keeps open to draw the device tile, so reading frames from it
//! costs the control plane nothing. This trait is the seam that lets
//! `riviu-core` consume those frames without depending on `riviu-ios-driver`,
//! which depends on core: the driver implements it, and the composition root
//! (Tauri state, or the live-test harness) injects it.

use std::sync::Arc;

use async_trait::async_trait;

/// One encoded screen frame (JPEG) in native device pixels.
pub type Frame = Arc<Vec<u8>>;

/// A per-device subscription to newly published frames.
#[async_trait]
pub trait FrameStream: Send {
    /// The next frame for this device. `None` when the source has shut down.
    ///
    /// Implementations coalesce rather than queue: a slow consumer receives the
    /// most recent frame, never a backlog of stale ones.
    async fn next(&mut self) -> Option<Frame>;
}

/// A source of device screen frames, keyed by UDID.
pub trait FrameSource: Send + Sync {
    /// Subscribe to frames for one device. Each call returns an independent
    /// stream, so several watchers can read the same device without stealing
    /// frames from each other.
    fn subscribe(&self, udid: &str) -> Box<dyn FrameStream>;

    /// The most recently published frame, if any. Useful for a one-shot check
    /// where waiting for the next frame would add latency for no benefit.
    fn latest(&self, udid: &str) -> Option<Frame>;
}

/// One frame qualified by the exact stream generation that produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationFrame {
    pub generation: u64,
    pub bytes: Frame,
}

/// Observable outcome from a generation-qualified stream subscription.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenerationFrameEvent {
    Frame(GenerationFrame),
    Advanced { expected: u64, actual: u64 },
    Closed,
}

#[async_trait]
pub trait GenerationFrameStream: Send {
    async fn next(&mut self) -> GenerationFrameEvent;
}

/// Generation-qualified access for evidence that must not cross a stream restart.
pub trait GenerationFrameSource: FrameSource {
    fn subscribe_generation(&self, udid: &str, generation: u64) -> Box<dyn GenerationFrameStream>;

    fn latest_in_generation(&self, udid: &str, generation: u64) -> Option<GenerationFrame>;
}

/// A source that never produces frames — used when no stream is available so
/// callers can degrade instead of branching on `Option<Arc<dyn FrameSource>>`.
pub struct NullFrameSource;

struct NullStream;

#[async_trait]
impl FrameStream for NullStream {
    async fn next(&mut self) -> Option<Frame> {
        None
    }
}

impl FrameSource for NullFrameSource {
    fn subscribe(&self, _udid: &str) -> Box<dyn FrameStream> {
        Box::new(NullStream)
    }

    fn latest(&self, _udid: &str) -> Option<Frame> {
        None
    }
}
