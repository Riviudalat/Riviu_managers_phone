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

pub fn encode_rgb_jpeg(img: &ImageBuffer<Rgb<u8>, Vec<u8>>, quality: u8) -> anyhow::Result<Vec<u8>> {
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
    latest: Arc<RwLock<HashMap<String, Frame>>>,
    tx: broadcast::Sender<(String, Frame)>,
}

impl StreamHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            latest: Arc::new(RwLock::new(HashMap::new())),
            tx,
        }
    }

    pub fn publish(&self, udid: &str, jpeg: Vec<u8>) {
        let frame: Frame = Arc::new(jpeg);
        self.latest.write().insert(udid.to_string(), frame.clone());
        let _ = self.tx.send((udid.to_string(), frame));
    }

    pub fn latest(&self, udid: &str) -> Option<Frame> {
        self.latest.read().get(udid).cloned()
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
    use super::*;

    #[tokio::test]
    async fn a_subscriber_only_sees_its_own_device() {
        let hub = StreamHub::new();
        let mut stream = FrameSource::subscribe(&hub, "udid-a");
        hub.publish("udid-b", vec![1, 2, 3]);
        hub.publish("udid-a", vec![9, 9]);
        let frame = stream.next().await.expect("frame");
        assert_eq!(&*frame, &[9, 9], "frames from another device leaked through");
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
