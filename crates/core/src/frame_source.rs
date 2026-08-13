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
    /// Monotonic within one generation. A fresh evidence subscription uses
    /// this as a watermark so buffered pre-verification frames cannot prove a
    /// later device effect.
    pub sequence: u64,
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
    /// Subscribe after the source's current frame watermark. Implementations
    /// must never yield an older buffered frame as a newly published event.
    fn subscribe_generation(&self, udid: &str, generation: u64) -> Box<dyn GenerationFrameStream>;

    fn latest_in_generation(&self, udid: &str, generation: u64) -> Option<GenerationFrame>;
}

/// The publish side of the same seam.
///
/// A device backend produces frames; the composition root owns the hub that
/// orders them and fans them out. Without this trait a second backend would have
/// to depend on the crate that happens to hold the hub — `riviu-android-driver`
/// would import `riviu-ios-driver` — which is backwards.
///
/// Publishing is **generation-qualified on purpose**. A reader that lost its
/// stream can still be holding buffered bytes, and those bytes must never land in
/// the next stream's cache: they would look like fresh evidence for an action that
/// had not happened yet. So a producer records the generation it started at and
/// stops when [`Self::publish_if_current`] reports it is stale.
pub trait FrameSink: Send + Sync {
    /// The generation currently accepted for this device.
    fn generation(&self, udid: &str) -> u64;

    /// Drop the cached frame and start a new generation, returning the new one.
    /// Every producer for `udid` older than this is now stale.
    fn clear_and_advance(&self, udid: &str) -> u64;

    /// Advance the generation but **keep** the cached frame, returning the new one.
    ///
    /// This is what a *bounded* background stop wants: the producer must die and
    /// every frame it still holds must be rejected, but the operator's tile should
    /// keep showing the last image it had rather than going blank. A destructive
    /// stop uses [`Self::clear_and_advance`] instead.
    ///
    /// The default delegates to the destructive reset, because a sink with no
    /// parked state is still *correct* — only blanker. Overriding it is a display
    /// improvement, never a correctness fix.
    fn park_and_advance(&self, udid: &str) -> u64 {
        self.clear_and_advance(udid)
    }

    /// Publish a JPEG frame, but only while `generation` is still current.
    ///
    /// `false` means a newer stream superseded this producer: drop the frame and
    /// stop reading. It is not an error.
    fn publish_if_current(&self, udid: &str, generation: u64, jpeg: Vec<u8>) -> bool;
}

/// Whether these bytes are a JPEG that actually **decodes**.
///
/// Lives beside [`FrameSink`] because it is part of the same contract: a producer
/// claiming `first_frame_observed` is asserting that a real image reached the hub,
/// and the magic bytes alone do not support that claim — a length-prefixed blob of
/// the right size with the right two-byte prefix passes a header check and is not
/// an image. Cheap enough to run once per stream start; do **not** put it in the
/// per-frame path.
///
/// Here rather than in each driver so the two backends cannot disagree about what
/// counts as a frame, and so neither has to take an image decoder of its own.
pub fn decodes_as_jpeg(bytes: &[u8]) -> bool {
    // Check the marker first: `load_from_memory` sniffs every supported format, so
    // without this a PNG would pass a function named for JPEG.
    bytes.starts_with(&[0xFF, 0xD8, 0xFF]) && image::load_from_memory(bytes).is_ok()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn one_pixel_jpeg() -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(1, 1)
            .write_to(&mut buffer, image::ImageFormat::Jpeg)
            .expect("encode a 1x1 jpeg");
        buffer.into_inner()
    }

    #[test]
    fn a_real_jpeg_decodes() {
        let jpeg = one_pixel_jpeg();
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
        assert!(decodes_as_jpeg(&jpeg));
    }

    #[test]
    fn the_magic_bytes_alone_are_not_a_frame() {
        // The exact failure this function exists to stop: a blob that passes a
        // header check and is not an image. Reporting it as the first frame would
        // put a lie inside `StreamStartProof`.
        assert!(!decodes_as_jpeg(&[
            0xFF, 0xD8, 0xFF, 0x00, 0x00, 0x00, 0x00
        ]));
        assert!(!decodes_as_jpeg(&[0xFF, 0xD8, 0xFF]));
        assert!(!decodes_as_jpeg(&[]));
    }

    #[test]
    fn another_decodable_format_is_still_not_a_jpeg() {
        // `load_from_memory` sniffs every enabled format, so without the marker
        // check a PNG would satisfy a function named for JPEG.
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(1, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("encode a 1x1 png");
        let png = png.into_inner();
        assert!(image::load_from_memory(&png).is_ok());
        assert!(!decodes_as_jpeg(&png));
    }

    #[test]
    fn the_default_park_falls_back_to_a_destructive_reset() {
        // A sink with no parked state is still correct, only blanker. This pins
        // that the default is not accidentally a no-op, which would leave a dead
        // producer's frames publishable.
        struct CountingSink {
            cleared: std::sync::atomic::AtomicU64,
        }
        impl FrameSink for CountingSink {
            fn generation(&self, _udid: &str) -> u64 {
                self.cleared.load(std::sync::atomic::Ordering::Relaxed)
            }
            fn clear_and_advance(&self, _udid: &str) -> u64 {
                self.cleared
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    + 1
            }
            fn publish_if_current(&self, _udid: &str, _generation: u64, _jpeg: Vec<u8>) -> bool {
                true
            }
        }
        let sink = CountingSink {
            cleared: std::sync::atomic::AtomicU64::new(0),
        };
        assert_eq!(sink.park_and_advance("fixture"), 1);
        assert_eq!(sink.generation("fixture"), 1);
    }
}
