//! Fan-out for the desktop **view** path. Not `StreamHub`.
//!
//! JPEG evidence stays on `StreamHub`. This hub carries H.264 samples from
//! scrcpy and JPEG bytes from the iOS preview loop, multiplexed onto one
//! loopback WebSocket so the WebView never base64-encodes a frame.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::SinkExt;
use parking_lot::Mutex;
use riviu_android_driver::{ViewKind, ViewPacket, ViewSink};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio_tungstenite::tungstenite::Message;

/// ASCII `RVU1`.
pub const VIEW_MAGIC: u32 = 0x5256_5531;
pub const VIEW_KIND_H264: u8 = 1;
pub const VIEW_KIND_JPEG: u8 = 2;
pub const VIEW_FLAG_KEY: u8 = 1;

/// Live view, not a DVR. 256 held ~8 s at 30 fps and the WebSocket then
/// drained the past — that is the lag the operator sees after the encoder
/// is already healthy. 8 trips `Lagged` in ~250 ms so we resync from the
/// last key instead of painting a backlog.
const BROADCAST_CAP: usize = 8;

pub struct ViewHub {
    generations: Mutex<HashMap<String, u64>>,
    last_jpeg: Mutex<HashMap<String, ViewPacket>>,
    last_h264: Mutex<HashMap<String, ViewPacket>>,
    last_packet_at: Mutex<HashMap<String, Instant>>,
    tx: broadcast::Sender<ViewPacket>,
    port: AtomicU16,
}

impl ViewHub {
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(BROADCAST_CAP);
        Arc::new(Self {
            generations: Mutex::new(HashMap::new()),
            last_jpeg: Mutex::new(HashMap::new()),
            last_h264: Mutex::new(HashMap::new()),
            last_packet_at: Mutex::new(HashMap::new()),
            tx,
            port: AtomicU16::new(0),
        })
    }

    pub fn endpoint(&self) -> Option<String> {
        let port = self.port.load(Ordering::Acquire);
        (port != 0).then(|| format!("ws://127.0.0.1:{port}"))
    }

    /// Bind 127.0.0.1:0 and accept WebSocket clients. Call once from the
    /// desktop background tasks.
    pub async fn listen(self: Arc<Self>) -> anyhow::Result<u16> {
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
        let port = listener.local_addr()?.port();
        self.port.store(port, Ordering::Release);
        let hub = Arc::clone(&self);
        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(error) => {
                        log::warn!("view websocket accept failed: {error}");
                        continue;
                    }
                };
                let hub = Arc::clone(&hub);
                tokio::spawn(async move {
                    if let Err(error) = serve_client(hub, stream).await {
                        log::debug!("view websocket client ended: {error}");
                    }
                });
            }
        });
        Ok(port)
    }

    /// iOS MJPEG preview: publish JPEG bytes without advancing generation.
    /// An Android UDID that already has H.264 must not receive these — the
    /// worker would paint a still over the live decode.
    pub fn publish_jpeg(&self, udid: &str, bytes: Vec<u8>) -> bool {
        if self.last_h264.lock().contains_key(udid) {
            return true;
        }
        let generation = {
            let mut gens = self.generations.lock();
            *gens.entry(udid.to_string()).or_insert(1)
        };
        self.publish(ViewPacket {
            udid: udid.to_string(),
            generation,
            kind: ViewKind::Jpeg,
            width: 0,
            height: 0,
            key: true,
            bytes,
        })
    }

    /// Age of the last sample that actually went on the wire. `None` after
    /// `advance` and before the first accepted publish — the keeper treats
    /// that as silent once the producer claims to be running.
    pub fn last_packet_age(&self, udid: &str) -> Option<Duration> {
        self.last_packet_at.lock().get(udid).map(Instant::elapsed)
    }
}

impl ViewSink for ViewHub {
    fn generation(&self, udid: &str) -> u64 {
        self.generations.lock().get(udid).copied().unwrap_or(0)
    }

    fn advance(&self, udid: &str) -> u64 {
        let mut gens = self.generations.lock();
        let next = gens.get(udid).copied().unwrap_or(0).saturating_add(1);
        gens.insert(udid.to_string(), next);
        self.last_h264.lock().remove(udid);
        self.last_packet_at.lock().remove(udid);
        next
    }

    fn publish(&self, packet: ViewPacket) -> bool {
        let current = self
            .generations
            .lock()
            .get(&packet.udid)
            .copied()
            .unwrap_or(0);
        if packet.kind == ViewKind::H264 && packet.generation != current {
            return false;
        }
        if packet.kind == ViewKind::Jpeg {
            self.last_jpeg
                .lock()
                .insert(packet.udid.clone(), packet.clone());
        }
        if packet.kind == ViewKind::H264 && packet.key {
            self.last_h264
                .lock()
                .insert(packet.udid.clone(), packet.clone());
        }
        self.last_packet_at
            .lock()
            .insert(packet.udid.clone(), Instant::now());
        let _ = self.tx.send(packet);
        true
    }
}

async fn serve_client(hub: Arc<ViewHub>, stream: tokio::net::TcpStream) -> anyhow::Result<()> {
    let _ = stream.set_nodelay(true);
    let mut ws = tokio_tungstenite::accept_async(stream).await?;
    replay_latest(&hub, &mut ws).await?;
    let mut rx = hub.tx.subscribe();
    loop {
        let first = match rx.recv().await {
            Ok(packet) => packet,
            Err(RecvError::Lagged(_)) => {
                // A stale last-key plus live deltas is a broken GOP: the
                // decoder freezes until the next IDR. Skip the backlog;
                // coalesce keeps a key from the live batch when one lands.
                continue;
            }
            Err(RecvError::Closed) => break,
        };
        let mut batch = vec![first];
        loop {
            match rx.try_recv() {
                Ok(packet) => batch.push(packet),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Lagged(_)) => break,
                Err(TryRecvError::Closed) => break,
            }
        }
        for packet in coalesce_for_live(batch) {
            ws.feed(Message::Binary(encode_packet(&packet).into()))
                .await?;
        }
        ws.flush().await?;
    }
    Ok(())
}

async fn replay_latest(
    hub: &ViewHub,
    ws: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) -> anyhow::Result<()> {
    let mut replay: Vec<ViewPacket> = hub.last_jpeg.lock().values().cloned().collect();
    replay.extend(hub.last_h264.lock().values().cloned());
    for packet in replay {
        ws.send(Message::Binary(encode_packet(&packet).into()))
            .await?;
    }
    Ok(())
}

/// Newest packet per UDID. If that newest is a delta and the same batch
/// still holds a key, emit the key first so a lagged decoder can resync
/// without waiting another `i-frame-interval`.
fn coalesce_for_live(packets: Vec<ViewPacket>) -> Vec<ViewPacket> {
    if packets.len() <= 1 {
        return packets;
    }
    struct Acc {
        last: ViewPacket,
        last_key: Option<ViewPacket>,
    }
    let mut last = HashMap::new();
    let mut order = Vec::new();
    for packet in packets {
        if !last.contains_key(&packet.udid) {
            order.push(packet.udid.clone());
        }
        let key = packet.key.then(|| packet.clone());
        last.entry(packet.udid.clone())
            .and_modify(|acc: &mut Acc| {
                if key.is_some() {
                    acc.last_key = key.clone();
                }
                acc.last = packet.clone();
            })
            .or_insert(Acc {
                last: packet,
                last_key: key,
            });
    }
    let mut out = Vec::with_capacity(order.len() * 2);
    for udid in order {
        let Some(acc) = last.remove(&udid) else {
            continue;
        };
        if let Some(key) = acc.last_key {
            if !acc.last.key {
                out.push(key);
            }
        }
        out.push(acc.last);
    }
    out
}

/// Binary envelope the worker parses. Layout:
/// `magic u32 BE | kind u8 | flags u8 | generation u64 BE | width u16 BE |
/// height u16 BE | udid_len u16 BE | payload_len u32 BE | udid | payload`.
pub fn encode_packet(packet: &ViewPacket) -> Vec<u8> {
    let kind = match packet.kind {
        ViewKind::H264 => VIEW_KIND_H264,
        ViewKind::Jpeg => VIEW_KIND_JPEG,
    };
    let flags = if packet.key { VIEW_FLAG_KEY } else { 0 };
    let udid = packet.udid.as_bytes();
    let mut out = Vec::with_capacity(24 + udid.len() + packet.bytes.len());
    out.extend_from_slice(&VIEW_MAGIC.to_be_bytes());
    out.push(kind);
    out.push(flags);
    out.extend_from_slice(&packet.generation.to_be_bytes());
    out.extend_from_slice(&(packet.width.min(u16::MAX as u32) as u16).to_be_bytes());
    out.extend_from_slice(&(packet.height.min(u16::MAX as u32) as u16).to_be_bytes());
    out.extend_from_slice(&(udid.len() as u16).to_be_bytes());
    out.extend_from_slice(&(packet.bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(udid);
    out.extend_from_slice(&packet.bytes);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_packet_round_trips_the_header_the_worker_will_read() {
        let packet = ViewPacket {
            udid: "ce06".into(),
            generation: 3,
            kind: ViewKind::H264,
            width: 176,
            height: 392,
            key: true,
            bytes: b"NAL".to_vec(),
        };
        let bytes = encode_packet(&packet);
        assert_eq!(&bytes[0..4], &VIEW_MAGIC.to_be_bytes());
        assert_eq!(bytes[4], VIEW_KIND_H264);
        assert_eq!(bytes[5], VIEW_FLAG_KEY);
        assert_eq!(&bytes[6..14], &3u64.to_be_bytes());
        assert_eq!(&bytes[14..16], &176u16.to_be_bytes());
        assert_eq!(&bytes[16..18], &392u16.to_be_bytes());
        assert_eq!(&bytes[18..20], &4u16.to_be_bytes());
        assert_eq!(&bytes[20..24], &3u32.to_be_bytes());
        assert_eq!(&bytes[24..28], b"ce06");
        assert_eq!(&bytes[28..], b"NAL");
    }

    #[test]
    fn a_stale_h264_generation_is_refused() {
        let hub = ViewHub::new();
        assert_eq!(hub.advance("a"), 1);
        assert_eq!(hub.advance("a"), 2);
        let stale = ViewPacket {
            udid: "a".into(),
            generation: 1,
            kind: ViewKind::H264,
            width: 10,
            height: 20,
            key: true,
            bytes: vec![1],
        };
        assert!(!hub.publish(stale));
        let current = ViewPacket {
            udid: "a".into(),
            generation: 2,
            kind: ViewKind::H264,
            width: 10,
            height: 20,
            key: true,
            bytes: vec![2],
        };
        assert!(hub.publish(current));
        assert_eq!(
            hub.last_h264.lock().get("a").map(|p| p.bytes.as_slice()),
            Some(&[2][..])
        );
        hub.advance("a");
        assert!(hub.last_h264.lock().get("a").is_none());
    }

    // Both operands are constants, so this is decided at compile time and belongs there.
    // As a `#[test]` it was a bound nobody learned about until the suite ran, and clippy
    // rejected it for exactly that reason; as an anonymous const it fails the build.
    const _: () = assert!(
        BROADCAST_CAP <= 16,
        "a large broadcast cap turns a slow WebSocket into delayed video"
    );

    #[test]
    fn a_backed_up_view_keeps_the_newest_packet_per_device() {
        let packets = vec![
            ViewPacket {
                udid: "a".into(),
                generation: 1,
                kind: ViewKind::H264,
                width: 10,
                height: 20,
                key: true,
                bytes: vec![1],
            },
            ViewPacket {
                udid: "b".into(),
                generation: 1,
                kind: ViewKind::H264,
                width: 10,
                height: 20,
                key: true,
                bytes: vec![2],
            },
            ViewPacket {
                udid: "a".into(),
                generation: 1,
                kind: ViewKind::H264,
                width: 10,
                height: 20,
                key: false,
                bytes: vec![3],
            },
        ];
        let live = coalesce_for_live(packets);
        assert_eq!(live.len(), 3);
        assert_eq!(live[0].udid, "a");
        assert!(live[0].key);
        assert_eq!(live[0].bytes, vec![1]);
        assert_eq!(live[1].udid, "a");
        assert_eq!(live[1].bytes, vec![3]);
        assert_eq!(live[2].udid, "b");
        assert_eq!(live[2].bytes, vec![2]);
    }

    #[test]
    fn jpeg_preview_does_not_overwrite_a_live_h264_device() {
        let hub = ViewHub::new();
        assert_eq!(hub.advance("a"), 1);
        assert!(hub.publish(ViewPacket {
            udid: "a".into(),
            generation: 1,
            kind: ViewKind::H264,
            width: 10,
            height: 20,
            key: true,
            bytes: vec![9],
        }));
        assert!(hub.publish_jpeg("a", vec![0xff, 0xd8]));
        assert!(hub.last_jpeg.lock().get("a").is_none());
        assert_eq!(
            hub.last_h264.lock().get("a").map(|p| p.bytes.as_slice()),
            Some(&[9][..])
        );
        assert!(hub.last_packet_age("a").is_some());
    }

    #[test]
    fn publish_records_last_packet_time_and_advance_clears_it() {
        let hub = ViewHub::new();
        hub.advance("a");
        assert!(hub.last_packet_age("a").is_none());
        assert!(hub.publish(ViewPacket {
            udid: "a".into(),
            generation: 1,
            kind: ViewKind::H264,
            width: 10,
            height: 20,
            key: true,
            bytes: vec![1],
        }));
        let age = hub.last_packet_age("a").expect("publish stamps the clock");
        assert!(age < Duration::from_secs(1));
        assert!(!hub.publish(ViewPacket {
            udid: "a".into(),
            generation: 0,
            kind: ViewKind::H264,
            width: 10,
            height: 20,
            key: true,
            bytes: vec![9],
        }));
        assert!(hub.last_packet_age("a").is_some());
        hub.advance("a");
        assert!(hub.last_packet_age("a").is_none());
    }
}
