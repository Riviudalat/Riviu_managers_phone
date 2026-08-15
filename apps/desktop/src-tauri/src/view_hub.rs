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

/// Live view, not a DVR — but 8 was paying for that with the operator's smoothness.
///
/// The original reasoning was sound and its fix landed elsewhere: 256 held ~8 s at 30 fps
/// and the WebSocket drained the past, so the operator watched history. What actually
/// prevents that is [`coalesce_for_live`], which collapses a drained batch to the newest
/// packet per device; the ring size stopped being what bounds the lag the moment that
/// existed.
///
/// What 8 does bound is how much host-side jitter it takes to lose the stream. This is ONE
/// channel for the whole fleet, so two phones at 24 fps put ~48 packets/s through it and 8
/// slots is ~166 ms. Any WebView2 hitch past that trips `Lagged`, and a `Lagged` costs far
/// more than a dropped frame: the backlog goes, keyframes included, and with
/// `i-frame-interval:int=1` the decoder has nothing to decode until the next IDR — up to a
/// full second of frozen picture, every time. Scrolling is when the encoder emits the most
/// data, which is exactly when the operator reported the stutter.
///
/// 128 is ~2.7 s of the same two-phone fleet, enough that ordinary jitter rides through,
/// and coalescing means a full ring is collapsed to one frame per device rather than
/// replayed.
const BROADCAST_CAP: usize = 128;

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
            Err(RecvError::Lagged(dropped)) => {
                // Skipping the backlog is right -- a stale last-key plus live deltas is a
                // broken GOP. Skipping it *silently* was not: the old code hoped a key would
                // turn up in some later batch, and until one did the decoder had nothing to
                // decode. With `i-frame-interval:int=1` that is up to a second of frozen
                // picture per lag event, which is the stutter the operator sees when
                // scrolling.
                //
                // The keyframe is already here. `last_h264` holds the newest one per device,
                // kept for exactly this shape of problem, so resync from it immediately
                // instead of waiting for the encoder to produce another.
                log::debug!(
                    "view subscriber lagged, dropped {dropped} packets; resyncing from last key"
                );
                let keys: Vec<ViewPacket> = hub.last_h264.lock().values().cloned().collect();
                for packet in keys {
                    ws.feed(Message::Binary(encode_packet(&packet).into()))
                        .await?;
                }
                ws.flush().await?;
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

/// How far behind one device may fall before its intermediate frames are discarded.
///
/// This is the difference between smooth motion and a slideshow, and the old policy had no
/// such threshold: ANY batch of more than one packet collapsed to the newest per device.
/// A batch grows whenever the WebSocket is a beat behind, which during a scroll -- the
/// moment the encoder produces the most data -- is constantly. So exactly when there was
/// the most motion to show, every intermediate frame was thrown away and the operator saw
/// the endpoints.
///
/// Dropping is still right when a device is genuinely far behind, because painting a
/// backlog is watching the past. Three frames is ~125 ms at 24 fps: short enough that
/// catching up is imperceptible, long enough that ordinary jitter no longer costs frames.
const COALESCE_AFTER_FRAMES: usize = 3;

/// Forward a drained batch, dropping intermediate frames only for a device that is more
/// than [`COALESCE_AFTER_FRAMES`] behind.
///
/// For a device at or under that, every packet is forwarded in order -- that is what makes
/// motion look like motion. Past it, the device collapses to its newest packet, preceded by
/// the newest key in the same batch if that newest is a delta, so a decoder that lost its
/// GOP can resync without waiting another `i-frame-interval`.
fn coalesce_for_live(packets: Vec<ViewPacket>) -> Vec<ViewPacket> {
    if packets.len() <= 1 {
        return packets;
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for packet in &packets {
        *counts.entry(packet.udid.as_str()).or_insert(0) += 1;
    }
    if counts.values().all(|count| *count <= COALESCE_AFTER_FRAMES) {
        // Nobody is meaningfully behind. Forwarding the batch untouched is the whole point:
        // these are consecutive frames of a moving screen.
        return packets;
    }
    let behind: std::collections::HashSet<String> = counts
        .iter()
        .filter(|(_, count)| **count > COALESCE_AFTER_FRAMES)
        .map(|(udid, _)| (*udid).to_string())
        .collect();
    // A device that is keeping up must not lose frames just because another device on the
    // same shared channel fell behind.
    let (packets, passthrough): (Vec<ViewPacket>, Vec<ViewPacket>) = packets
        .into_iter()
        .partition(|packet| behind.contains(&packet.udid));
    let collapsed = collapse_to_newest(packets);
    let mut out = passthrough;
    out.extend(collapsed);
    out
}

fn collapse_to_newest(packets: Vec<ViewPacket>) -> Vec<ViewPacket> {
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

    // Still decided at compile time, and still the same worry -- a ring that a slow
    // WebSocket can drain as history turns live video into delayed video. What changed is
    // what enforces it. `coalesce_for_live` collapses a drained batch to the newest packet
    // per device, so the ring cannot be replayed as a backlog whatever its size; the bound
    // that matters now is against the OTHER failure, where a ring too small to absorb host
    // jitter trips `Lagged` and costs a whole GOP. 8 was ~166 ms for two phones, which any
    // WebView2 hitch clears.
    //
    // The upper bound stays, well above the old one and well below a DVR: 128 packets is
    // ~2.7 s of that fleet, and it only ever materialises as one frame per device.
    const _: () = assert!(
        BROADCAST_CAP <= 512,
        "a broadcast cap this large is a DVR, not a live view, even with coalescing"
    );
    const _: () = assert!(
        BROADCAST_CAP >= 64,
        "a ring under ~1s of fleet traffic trips Lagged on ordinary jitter, and every Lagged costs a full i-frame-interval of frozen picture"
    );

    fn h264(udid: &str, key: bool, byte: u8) -> ViewPacket {
        ViewPacket {
            udid: udid.into(),
            generation: 1,
            kind: ViewKind::H264,
            width: 10,
            height: 20,
            key,
            bytes: vec![byte],
        }
    }

    #[test]
    fn a_batch_that_is_barely_behind_keeps_every_frame_in_order() {
        // The change that makes motion look like motion. Any batch of more than one packet
        // used to collapse to the newest per device, and a batch grows whenever the
        // WebSocket is one beat behind -- which during a scroll is constantly, because a
        // scroll is when the encoder emits the most data. The operator saw the endpoints of
        // every movement and nothing between them.
        let live = coalesce_for_live(vec![
            h264("a", true, 1),
            h264("b", true, 2),
            h264("a", false, 3),
        ]);
        assert_eq!(live.len(), 3);
        assert_eq!(live[0].bytes, vec![1]);
        assert_eq!(live[1].bytes, vec![2]);
        assert_eq!(live[2].bytes, vec![3]);
    }

    #[test]
    fn a_device_far_behind_collapses_to_its_newest_key_and_frame() {
        // Past the threshold, dropping is still right: painting a backlog is watching the
        // past. The newest key goes first so a decoder that lost its GOP can resync without
        // waiting another i-frame-interval.
        let mut packets = vec![h264("a", true, 1)];
        for byte in 2..=8 {
            packets.push(h264("a", false, byte));
        }
        let live = coalesce_for_live(packets);
        assert_eq!(live.len(), 2, "{live:?}");
        assert!(live[0].key);
        assert_eq!(live[0].bytes, vec![1]);
        assert_eq!(live[1].bytes, vec![8]);
    }

    #[test]
    fn a_device_keeping_up_does_not_lose_frames_because_another_fell_behind() {
        // One shared channel for the whole fleet, so this is the failure mode that matters
        // at scale: one slow phone must not turn every other phone into a slideshow.
        let mut packets = vec![h264("slow", true, 1)];
        for byte in 2..=8 {
            packets.push(h264("slow", false, byte));
        }
        packets.push(h264("fast", true, 100));
        packets.push(h264("fast", false, 101));

        let live = coalesce_for_live(packets);
        let fast: Vec<u8> = live
            .iter()
            .filter(|p| p.udid == "fast")
            .map(|p| p.bytes[0])
            .collect();
        assert_eq!(
            fast,
            vec![100, 101],
            "the device keeping up kept both frames"
        );
        let slow: Vec<u8> = live
            .iter()
            .filter(|p| p.udid == "slow")
            .map(|p| p.bytes[0])
            .collect();
        assert_eq!(
            slow,
            vec![1, 8],
            "the device far behind kept its key and newest"
        );
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
