use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use image::{ImageBuffer, Rgb};
use parking_lot::RwLock;
use riviu_core::{
    Frame, FrameSource, FrameStream, GenerationFrame, GenerationFrameEvent, GenerationFrameSource,
    GenerationFrameStream,
};
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
    generation_tx: broadcast::Sender<HubGenerationEvent>,
}

#[derive(Clone)]
enum HubGenerationEvent {
    Frame {
        udid: String,
        generation: u64,
        bytes: Frame,
    },
    Advanced {
        udid: String,
        generation: u64,
    },
}

#[derive(Default)]
struct HubState {
    latest: HashMap<String, Frame>,
    generations: HashMap<String, u64>,
}

impl StreamHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        let (generation_tx, _) = broadcast::channel(256);
        Self {
            state: Arc::new(RwLock::new(HubState::default())),
            tx,
            generation_tx,
        }
    }

    pub fn publish(&self, udid: &str, jpeg: Vec<u8>) {
        let frame: Frame = Arc::new(jpeg);
        let mut state = self.state.write();
        let generation = state.generations.get(udid).copied().unwrap_or(0);
        state.latest.insert(udid.to_string(), frame.clone());
        let _ = self.tx.send((udid.to_string(), frame.clone()));
        let _ = self.generation_tx.send(HubGenerationEvent::Frame {
            udid: udid.to_string(),
            generation,
            bytes: frame,
        });
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
        let _ = self.tx.send((udid.to_string(), frame.clone()));
        let _ = self.generation_tx.send(HubGenerationEvent::Frame {
            udid: udid.to_string(),
            generation,
            bytes: frame,
        });
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
        let new_generation = *generation;
        let _ = self.generation_tx.send(HubGenerationEvent::Advanced {
            udid: udid.to_string(),
            generation: new_generation,
        });
        (old_generation, new_generation)
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

struct HubGenerationStream {
    udid: String,
    generation: u64,
    state: Arc<RwLock<HubState>>,
    rx: broadcast::Receiver<HubGenerationEvent>,
}

#[async_trait]
impl GenerationFrameStream for HubGenerationStream {
    async fn next(&mut self) -> GenerationFrameEvent {
        loop {
            let actual = self
                .state
                .read()
                .generations
                .get(&self.udid)
                .copied()
                .unwrap_or(0);
            if actual > self.generation {
                return GenerationFrameEvent::Advanced {
                    expected: self.generation,
                    actual,
                };
            }

            match self.rx.recv().await {
                Ok(HubGenerationEvent::Frame {
                    udid,
                    generation,
                    bytes,
                }) if udid == self.udid && generation == self.generation => {
                    let actual = self
                        .state
                        .read()
                        .generations
                        .get(&self.udid)
                        .copied()
                        .unwrap_or(0);
                    if actual > self.generation {
                        return GenerationFrameEvent::Advanced {
                            expected: self.generation,
                            actual,
                        };
                    }
                    return GenerationFrameEvent::Frame(GenerationFrame { generation, bytes });
                }
                Ok(HubGenerationEvent::Advanced { udid, generation })
                    if udid == self.udid && generation > self.generation =>
                {
                    return GenerationFrameEvent::Advanced {
                        expected: self.generation,
                        actual: generation,
                    };
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    return GenerationFrameEvent::Closed;
                }
            }
        }
    }
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

impl GenerationFrameSource for StreamHub {
    fn subscribe_generation(&self, udid: &str, generation: u64) -> Box<dyn GenerationFrameStream> {
        Box::new(HubGenerationStream {
            udid: udid.to_string(),
            generation,
            state: self.state.clone(),
            rx: self.generation_tx.subscribe(),
        })
    }

    fn latest_in_generation(&self, udid: &str, generation: u64) -> Option<GenerationFrame> {
        let state = self.state.read();
        (state.generations.get(udid).copied().unwrap_or(0) == generation)
            .then(|| state.latest.get(udid).cloned())
            .flatten()
            .map(|bytes| GenerationFrame { generation, bytes })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use riviu_core::{GenerationFrameEvent, GenerationFrameSource};

    use super::*;

    #[tokio::test]
    async fn generation_subscription_rejects_buffered_old_frames() {
        let hub = StreamHub::new();
        let old = hub.generation("fixture");
        let mut old_stream = hub.subscribe_generation("fixture", old);
        assert!(hub.publish_if_current("fixture", old, vec![1, 2, 3]));

        let (_, new) = hub.clear_and_advance("fixture");

        assert!(hub.latest_in_generation("fixture", old).is_none());
        assert_eq!(
            old_stream.next().await,
            GenerationFrameEvent::Advanced {
                expected: old,
                actual: new,
            }
        );
        assert!(hub.publish_if_current("fixture", new, vec![9, 8, 7]));
        let latest = hub.latest_in_generation("fixture", new).expect("new frame");
        assert_eq!(&*latest.bytes, &[9, 8, 7]);
    }

    #[tokio::test]
    async fn generation_subscription_reports_an_already_missed_advance() {
        let hub = StreamHub::new();
        let old = hub.generation("fixture");
        let (_, new) = hub.clear_and_advance("fixture");
        let mut old_stream = hub.subscribe_generation("fixture", old);

        assert_eq!(
            old_stream.next().await,
            GenerationFrameEvent::Advanced {
                expected: old,
                actual: new,
            }
        );
    }

    #[tokio::test]
    async fn generation_subscription_qualifies_raw_publish_with_current_generation() {
        let hub = StreamHub::new();
        let generation = hub.generation("fixture");
        let mut stream = hub.subscribe_generation("fixture", generation);

        hub.publish("fixture", vec![4, 5, 6]);

        let GenerationFrameEvent::Frame(frame) = stream.next().await else {
            panic!("expected a generation-qualified frame");
        };
        assert_eq!(frame.generation, generation);
        assert_eq!(&*frame.bytes, &[4, 5, 6]);
    }

    #[tokio::test]
    async fn generation_subscription_reports_closed_instead_of_hanging() {
        let hub = StreamHub::new();
        let generation = hub.generation("fixture");
        let mut stream = hub.subscribe_generation("fixture", generation);
        drop(hub);

        assert_eq!(stream.next().await, GenerationFrameEvent::Closed);
    }

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
