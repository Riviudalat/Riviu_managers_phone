use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use image::{ImageBuffer, Rgb};
use parking_lot::RwLock;
use riviu_core::{Frame, FrameSource, FrameStream};
use tokio::sync::broadcast;

pub fn jpeg_quality(quality: &riviu_core::StreamQuality) -> u8 {
    match quality {
        riviu_core::StreamQuality::Low => 40,
        riviu_core::StreamQuality::Medium => 60,
        riviu_core::StreamQuality::High => 80,
        riviu_core::StreamQuality::Extra => 92,
    }
}

pub fn encode_rgb_jpeg(
    img: &ImageBuffer<Rgb<u8>, Vec<u8>>,
    quality: u8,
) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, quality);
    encoder.encode(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(buf)
}

/// Fan-out of device screen frames to every consumer: the UI tile, and the
/// nurture engine's popup watcher.
///
/// Frames are shared as `Arc<Vec<u8>>` rather than cloned per subscriber — a
/// 750×1334 JPEG is ~100 kB and there are several subscribers at up to 24 FPS.
#[derive(Clone)]
pub struct StreamHub {
    state: Arc<RwLock<HubState>>,
    tx: broadcast::Sender<(String, Frame)>,
}

#[derive(Default)]
struct HubState {
    latest: HashMap<String, Frame>,
    generations: HashMap<String, u64>,
}

impl StreamHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            state: Arc::new(RwLock::new(HubState::default())),
            tx,
        }
    }

    pub fn publish(&self, udid: &str, jpeg: Vec<u8>) {
        let frame: Frame = Arc::new(jpeg);
        let mut state = self.state.write();
        state.latest.insert(udid.to_string(), frame.clone());
        let _ = self.tx.send((udid.to_string(), frame));
    }

    pub(crate) fn generation(&self, udid: &str) -> u64 {
        self.state
            .read()
            .generations
            .get(udid)
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn publish_if_current(&self, udid: &str, generation: u64, jpeg: Vec<u8>) -> bool {
        let frame: Frame = Arc::new(jpeg);
        let mut state = self.state.write();
        let current = state.generations.get(udid).copied().unwrap_or(0);
        if generation != current {
            return false;
        }
        state.latest.insert(udid.to_string(), frame.clone());
        let _ = self.tx.send((udid.to_string(), frame));
        true
    }

    pub fn latest(&self, udid: &str) -> Option<Frame> {
        self.state.read().latest.get(udid).cloned()
    }

    pub fn clear(&self, udid: &str) {
        self.clear_and_advance(udid);
    }

    pub(crate) fn clear_and_advance(&self, udid: &str) -> (u64, u64) {
        let mut state = self.state.write();
        state.latest.remove(udid);
        let generation = state.generations.entry(udid.to_string()).or_default();
        let old_generation = *generation;
        *generation = generation.checked_add(1).unwrap_or(1);
        (old_generation, *generation)
    }

    /// Raw subscription to every device's frames. Prefer [`FrameSource::subscribe`]
    /// when you only care about one device.
    pub fn subscribe(&self) -> broadcast::Receiver<(String, Frame)> {
        self.tx.subscribe()
    }
}

impl Default for StreamHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-device view of the hub's broadcast.
struct HubStream {
    udid: String,
    rx: broadcast::Receiver<(String, Frame)>,
}

#[async_trait]
impl FrameStream for HubStream {
    async fn next(&mut self) -> Option<Frame> {
        loop {
            match self.rx.recv().await {
                Ok((udid, frame)) if udid == self.udid => return Some(frame),
                Ok(_) => continue,
                // A slow consumer drops the backlog and resumes at the newest
                // frame. For a screen watcher that is exactly right: stale
                // frames describe a screen that has already changed.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

impl FrameSource for StreamHub {
    fn subscribe(&self, udid: &str) -> Box<dyn FrameStream> {
        Box::new(HubStream {
            udid: udid.to_string(),
            rx: self.tx.subscribe(),
        })
    }

    fn latest(&self, udid: &str) -> Option<Frame> {
        StreamHub::latest(self, udid)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn a_subscriber_only_sees_its_own_device() {
        let hub = StreamHub::new();
        let mut stream = FrameSource::subscribe(&hub, "udid-a");
        hub.publish("udid-b", vec![1, 2, 3]);
        hub.publish("udid-a", vec![9, 9]);
        let frame = stream.next().await.expect("frame");
        assert_eq!(
            &*frame,
            &[9, 9],
            "frames from another device leaked through"
        );
    }

    #[tokio::test]
    async fn latest_returns_the_most_recent_frame_per_device() {
        let hub = StreamHub::new();
        assert!(FrameSource::latest(&hub, "udid-a").is_none());
        hub.publish("udid-a", vec![1]);
        hub.publish("udid-a", vec![2]);
        hub.publish("udid-b", vec![7]);
        assert_eq!(&*FrameSource::latest(&hub, "udid-a").unwrap(), &[2]);
        assert_eq!(&*FrameSource::latest(&hub, "udid-b").unwrap(), &[7]);
    }

    #[test]
    fn clearing_one_device_drops_only_its_stale_latest_frame() {
        let hub = StreamHub::new();
        hub.publish("udid-a", vec![1]);
        hub.publish("udid-b", vec![2]);

        hub.clear("udid-a");

        assert!(FrameSource::latest(&hub, "udid-a").is_none());
        assert_eq!(&*FrameSource::latest(&hub, "udid-b").unwrap(), &[2]);
    }

    #[tokio::test]
    async fn a_stale_producer_cannot_repopulate_or_broadcast_frames_after_clear() {
        let hub = StreamHub::new();
        let old_generation = hub.generation("udid-a");
        assert!(hub.publish_if_current("udid-a", old_generation, vec![1]));

        let (cleared_generation, new_generation) = hub.clear_and_advance("udid-a");
        let mut broadcasts = hub.subscribe();

        assert!(!hub.publish_if_current("udid-a", old_generation, vec![2]));
        assert!(FrameSource::latest(&hub, "udid-a").is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(25), broadcasts.recv())
                .await
                .is_err(),
            "a buffered reader from the cleared generation must not broadcast"
        );
        assert_eq!(cleared_generation, old_generation);
        assert_eq!(hub.generation("udid-a"), new_generation);
        assert_ne!(old_generation, new_generation);
        assert!(hub.publish_if_current("udid-a", new_generation, vec![3]));
        let (broadcast_udid, broadcast_frame) = broadcasts.recv().await.expect("new frame");
        assert_eq!(broadcast_udid, "udid-a");
        assert_eq!(&*broadcast_frame, &[3]);
        assert_eq!(&*FrameSource::latest(&hub, "udid-a").unwrap(), &[3]);
    }

    /// Two watchers on one device must both receive frames, not steal them.
    #[tokio::test]
    async fn subscribers_are_independent() {
        let hub = StreamHub::new();
        let mut a = FrameSource::subscribe(&hub, "udid-a");
        let mut b = FrameSource::subscribe(&hub, "udid-a");
        hub.publish("udid-a", vec![5]);
        assert_eq!(&*a.next().await.unwrap(), &[5]);
        assert_eq!(&*b.next().await.unwrap(), &[5]);
    }

    #[tokio::test]
    async fn a_closed_hub_ends_the_stream() {
        let hub = StreamHub::new();
        let mut stream = FrameSource::subscribe(&hub, "udid-a");
        drop(hub);
        assert!(stream.next().await.is_none());
    }
}
