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

/// Live view, not a DVR — and the number has to be read as a rate, not a size.
///
/// This is ONE channel for the whole fleet, so its capacity in *time* shrinks linearly as
/// phones are added. That is the property every previous value here got wrong. At 24 fps:
///
/// | devices | cap 128 | cap 2048 |
/// |---|---|---|
/// | 2 | 2667 ms | 42667 ms |
/// | 20 | **267 ms** | 4267 ms |
/// | 50 | 107 ms | 1707 ms |
/// | 100 | **53 ms** | 853 ms |
///
/// 8 was chosen against a two-phone bench and 128 against the same bench; a 20-phone box
/// then reduced 128 to 267 ms, which any WebView2 hitch clears. What a lag costs is no longer
/// catastrophic — `serve_client` drains to the present and resyncs from the newest keyframe —
/// but it is still a visible hiccup, so headroom is worth buying.
///
/// The DVR worry that produced the original 8 is handled elsewhere and no longer bounds this:
/// [`coalesce_for_live`] collapses any device more than a few frames behind to its newest
/// packet, so a full ring materialises as one frame per device rather than as history. That
/// is what makes a large ring safe.
///
/// **The structural fix is a channel per device**, which would make the capacity independent
/// of fleet size instead of merely generous. Until then this is sized so a 100-device farm
/// still has most of a second: 2048 packets is ~4.3 s at twenty phones and ~0.85 s at a
/// hundred. Memory is bounded by what the ring holds, roughly 61 KB/s per device at 24 fps
/// with a 1 s i-frame interval, so ~5 MB for twenty phones.
const BROADCAST_CAP: usize = 2048;

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

    /// Which producer is current for this device.
    ///
    /// Same value [`ViewSink::generation`] returns, as an inherent method so callers that
    /// only need to date a piece of evidence do not have to import the sink trait. The
    /// watchdog uses it to decide whether a paint report describes the producer that is
    /// running or the one it replaced — counters from before a restart show arrivals far
    /// ahead of frames forever, and acting on them is a restart loop.
    pub fn current_generation(&self, udid: &str) -> u64 {
        self.generations.lock().get(udid).copied().unwrap_or(0)
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
    // Subscribe BEFORE replaying. The other order loses every packet published between the
    // replay's awaited writes and the subscribe -- silently, permanently, for that client --
    // and the window is one `ws.send().await` per device, which grew with the overlay's
    // larger encode. A duplicate frame from the overlap is harmless; a missing keyframe is
    // a black canvas.
    let mut rx = hub.tx.subscribe();
    replay_latest(&hub, &mut ws).await?;
    loop {
        let mut lagged_by = 0u64;
        let first = match rx.recv().await {
            Ok(packet) => packet,
            Err(RecvError::Lagged(dropped)) => {
                // Skipping the backlog is right -- a stale key plus live deltas is a broken
                // GOP. Resyncing here was NOT: at this point the receiver's cursor sits at
                // the oldest retained value, so a keyframe written now is followed by up to a
                // ring's worth of traffic that PREDATES it. Across a generation bump that is
                // a gen-N key followed by gen-N-1 packets, the worker flips generation and
                // closes its decoder, and nothing reopens it. Resync happens after the drain
                // below, once the cursor has actually reached the present.
                lagged_by = dropped;
                match rx.try_recv() {
                    Ok(packet) => packet,
                    Err(_) => {
                        resync_from_last_key(&hub, &mut ws, lagged_by).await?;
                        continue;
                    }
                }
            }
            Err(RecvError::Closed) => break,
        };
        let mut batch = vec![first];
        loop {
            match rx.try_recv() {
                Ok(packet) => batch.push(packet),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Lagged(dropped)) => {
                    // Do NOT break here. `try_recv` reporting Lagged has already advanced the
                    // cursor and CONSUMED the lag report, so breaking meant the outer
                    // `RecvError::Lagged` arm could never fire: no resync, no log, and any
                    // keyframe in the skipped span silently gone while every later delta was
                    // delivered normally. That is precisely "packets keep arriving and
                    // nothing paints". Record it and keep draining to the present.
                    lagged_by += dropped;
                    batch.clear();
                }
                Err(TryRecvError::Closed) => break,
            }
        }
        if lagged_by > 0 {
            // Everything drained after a lag is history relative to the newest keyframe, so
            // it is dropped rather than painted, and the client is resynced from the newest
            // key now that the cursor has reached the present.
            batch.clear();
            resync_from_last_key(&hub, &mut ws, lagged_by).await?;
            continue;
        }
        for packet in coalesce_for_live(batch) {
            ws.feed(Message::Binary(encode_packet(&packet).into()))
                .await?;
        }
        ws.flush().await?;
    }
    Ok(())
}

/// Hand the client the newest keyframe of every device, after a lag.
///
/// Only correct once the receiver's cursor has reached the present -- see the call sites.
async fn resync_from_last_key(
    hub: &ViewHub,
    ws: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    dropped: u64,
) -> anyhow::Result<()> {
    log::debug!("view subscriber lagged, dropped {dropped} packets; resyncing from last key");
    let keys: Vec<ViewPacket> = hub.last_h264.lock().values().cloned().collect();
    for packet in keys {
        ws.feed(Message::Binary(encode_packet(&packet).into()))
            .await?;
    }
    ws.flush().await?;
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

    // Both walls, still decided at compile time. The upper one is no longer about replaying
    // history — `coalesce_for_live` prevents that whatever the size — it is about how much
    // memory one channel may hold. The lower one is the real hazard now: this ring is shared
    // by the whole fleet, so a value that felt generous against two phones is a fraction of a
    // second against twenty.
    const _: () = assert!(
        BROADCAST_CAP <= 8192,
        "a ring this large holds tens of MB of video once a fleet fills it"
    );
    const _: () = assert!(
        BROADCAST_CAP >= 1024,
        "at 24 fps a shared ring under 1024 gives a 20-phone fleet well under a second, and          any host hitch past that costs a resync"
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
