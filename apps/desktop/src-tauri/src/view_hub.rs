//! Fan-out for the desktop **view** path. Not `StreamHub`.
//!
//! JPEG evidence stays on `StreamHub`. This hub carries H.264 samples from
//! scrcpy and JPEG bytes from the iOS preview loop, multiplexed onto one
//! loopback WebSocket so the WebView never base64-encodes a frame.
//!
//! **One channel per device, one socket for the fleet.** Those are different axes and the
//! file used to conflate them: a single shared ring meant a device's buffer *in time* shrank
//! linearly as phones were added, and a lag could not be attributed to the phone that caused
//! it. The socket stays shared because the wire protocol is one-way and the worker
//! demultiplexes by udid; what is now per-device is the buffering behind it.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::SinkExt;
use parking_lot::Mutex;
use riviu_android_driver::{ViewKind, ViewPacket, ViewSink};
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinSet;
use tokio_tungstenite::tungstenite::Message;

/// ASCII `RVU1`.
pub const VIEW_MAGIC: u32 = 0x5256_5531;
pub const VIEW_KIND_H264: u8 = 1;
pub const VIEW_KIND_JPEG: u8 = 2;
pub const VIEW_FLAG_KEY: u8 = 1;

/// Live view, not a DVR — and now a **per-device** rate, which is the whole point.
///
/// This used to be one ring for the entire fleet, and that made its capacity in *time* a
/// function of how many phones were plugged in. Every value it ever held got that wrong: at
/// 24 fps a shared 128 was 2667 ms against two phones and **267 ms** against twenty, and the
/// 2048 that replaced it was 4267 ms at twenty but 853 ms at a hundred. The number kept
/// being re-tuned because the shape was wrong, not the size.
///
/// Per device, 128 slots is **5.3 s at any fleet size** — twenty phones, a hundred, one. A
/// lag also became attributable: a device that falls behind exhausts its own ring and gets
/// its own resync, instead of a slow phone spending the fleet's shared budget and every
/// client resyncing every device (see [`replay_device`]).
///
/// **The tradeoff inverts, and that is why this number went DOWN rather than staying put.**
/// Time per device is now constant; memory now scales with fleet size. At the ~2.6 KB
/// average packet this path carries (61 KB/s per device at 24 fps), 128 slots is ~325 KB per
/// device, ~6.5 MB across twenty phones. Keeping 2048 here would have been ~5.2 MB per
/// device and **~104 MB at twenty** — and tokio allocates every slot when the channel is
/// created, rounded up to a power of two, so that cost lands on device discovery rather than
/// on load.
///
/// What makes a ring this size safe is unchanged: [`coalesce_for_live`] collapses any device
/// more than a few frames behind to its newest packet, so a full ring materialises as one
/// frame rather than as history, and a lag is no longer catastrophic because the forwarder
/// drains to the present and resyncs from the newest keyframe.
const DEVICE_BROADCAST_CAP: usize = 128;

/// How many events one client may have queued across all its devices before its forwarders
/// block.
///
/// This is the head-of-line buffer, not the capacity that matters: when the socket is slow
/// the forwarders block here and **each device's own ring absorbs its own backlog**, which
/// is exactly the property the per-device split exists for. Delay is shared, capacity loss
/// is not. 256 is a little over five devices' worth of a 48-frame burst — deep enough that
/// an ordinary WebView2 hitch never reaches the rings at all.
const CLIENT_QUEUE_CAP: usize = 256;

/// Everything the hub knows about one device, including its own channel.
///
/// Consolidated from four separate maps. That was not cosmetic: `publish` runs per frame per
/// device — 480 acquisitions a second at twenty phones and 24 fps — and took four locks in
/// sequence with a torn window between them, so a reader could see a new `last_packet_at`
/// against a stale `last_h264`.
struct DeviceView {
    generation: u64,
    last_jpeg: Option<ViewPacket>,
    last_h264: Option<ViewPacket>,
    last_packet_at: Option<Instant>,
    tx: broadcast::Sender<ViewPacket>,
}

impl DeviceView {
    fn new(generation: u64) -> Self {
        let (tx, _) = broadcast::channel(DEVICE_BROADCAST_CAP);
        Self {
            generation,
            last_jpeg: None,
            last_h264: None,
            last_packet_at: None,
            tx,
        }
    }
}

pub struct ViewHub {
    devices: Mutex<HashMap<String, DeviceView>>,
    /// Announces a udid the first time the hub sees it.
    ///
    /// Clients connect once, at app start, before any phone has been enumerated. Without
    /// this a client would only ever forward the devices that existed at the instant it
    /// connected, and every phone discovered afterwards would be a canvas that never paints.
    roster: broadcast::Sender<String>,
    port: AtomicU16,
    /// Secret a client must present to receive a single frame.
    ///
    /// This socket carries the live screen of **every phone in the fleet**, and until this
    /// existed it handed that to anyone who completed a WebSocket handshake on the port —
    /// no token, no `Origin` check, and started unconditionally rather than off-by-default.
    /// Six lines away in `state.rs` the Local API is off by default, token-gated and
    /// constant-time compared, with a comment explaining why; the socket that streams
    /// twenty-one screens had none of it.
    ///
    /// Two attackers, both real on a single-operator box:
    /// * any other local process can scan loopback, and a successful handshake followed
    ///   immediately by binary frames is an unambiguous fingerprint;
    /// * a web page, because **WebSocket is not subject to CORS** — a browser will happily
    ///   open `ws://127.0.0.1:<port>` from any origin.
    ///
    /// Minted per process, so it dies with the app and never reaches disk.
    token: String,
}

impl ViewHub {
    pub fn new() -> Arc<Self> {
        // The roster carries one short string per device ever seen, so it can be generous;
        // a client that lags it re-snapshots rather than losing anything.
        let (roster, _) = broadcast::channel(256);
        Arc::new(Self {
            devices: Mutex::new(HashMap::new()),
            roster,
            port: AtomicU16::new(0),
            // Two v4 UUIDs, the same construction and the same CSPRNG (`getrandom`) the Local
            // API's token uses. 244 bits over 64 hex chars.
            token: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        })
    }

    /// The URL the frontend should open, token included.
    ///
    /// The token rides in the query string rather than a header because the browser
    /// `WebSocket` constructor cannot set headers — and the frontend passes this straight to
    /// `new WebSocket(url)` (`viewStore.ts`), so nothing on that side has to change. It never
    /// leaves the machine: the URL is handed to our own WebView over IPC and the socket is
    /// loopback-bound.
    pub fn endpoint(&self) -> Option<String> {
        let port = self.port.load(Ordering::Acquire);
        (port != 0).then(|| format!("ws://127.0.0.1:{port}/?k={}", self.token))
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
        let (generation, created) = {
            let mut devices = self.devices.lock();
            match devices.get(udid) {
                Some(device) if device.last_h264.is_some() => return true,
                Some(device) => (device.generation, false),
                None => {
                    devices.insert(udid.to_string(), DeviceView::new(1));
                    (1, true)
                }
            }
        };
        // Only on creation. This runs once per preview frame, so announcing unconditionally
        // would fill the roster with one device's name and make every other client
        // re-snapshot on a `Lagged` several times a second.
        if created {
            let _ = self.roster.send(udid.to_string());
        }
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
        self.devices
            .lock()
            .get(udid)
            .and_then(|device| device.last_packet_at)
            .map(|at| at.elapsed())
    }

    /// Which producer is current for this device.
    ///
    /// Same value [`ViewSink::generation`] returns, as an inherent method so callers that
    /// only need to date a piece of evidence do not have to import the sink trait. The
    /// watchdog uses it to decide whether a paint report describes the producer that is
    /// running or the one it replaced — counters from before a restart show arrivals far
    /// ahead of frames forever, and acting on them is a restart loop.
    pub fn current_generation(&self, udid: &str) -> u64 {
        self.devices
            .lock()
            .get(udid)
            .map(|device| device.generation)
            .unwrap_or(0)
    }

    /// Drop a device that has left the fleet, closing its channel.
    ///
    /// Every subscribed forwarder sees `Closed` and exits; the client's socket is untouched
    /// and its other devices keep flowing. Without this the hub keeps one fully-allocated
    /// ring per udid it has ever seen — a leak that was harmless when the maps held only a
    /// generation and a cached frame, and is not harmless now that each entry owns a channel.
    ///
    /// Deliberately **not** called on a producer stop or a status flap: a device whose view
    /// is merely being restarted is still there, and closing its channel would make every
    /// client tear down a canvas that is about to be repainted.
    pub fn forget(&self, udid: &str) {
        self.devices.lock().remove(udid);
    }

    /// The udids the hub currently knows about. For reconciling against the registry.
    pub fn known_devices(&self) -> Vec<String> {
        self.devices.lock().keys().cloned().collect()
    }

    /// A receiver for every device that exists right now.
    ///
    /// `subscribe` is synchronous, so every cursor is pinned before the caller awaits
    /// anything — which is what preserves the subscribe-before-replay rule even though the
    /// forwarders that use these receivers are spawned afterwards.
    fn subscribe_all(&self) -> Vec<(String, broadcast::Receiver<ViewPacket>)> {
        self.devices
            .lock()
            .iter()
            .map(|(udid, device)| (udid.clone(), device.tx.subscribe()))
            .collect()
    }

    fn subscribe_one(&self, udid: &str) -> Option<broadcast::Receiver<ViewPacket>> {
        self.devices
            .lock()
            .get(udid)
            .map(|device| device.tx.subscribe())
    }

    /// The newest independently-decodable frames this device has: its JPEG, its keyframe, or
    /// both. What a client needs to paint something immediately without waiting for the next
    /// i-frame interval.
    fn cached_frames(&self, udid: &str) -> Vec<ViewPacket> {
        self.devices
            .lock()
            .get(udid)
            .map(|device| {
                device
                    .last_jpeg
                    .iter()
                    .chain(device.last_h264.iter())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    fn peek_last_h264(&self, udid: &str) -> Option<ViewPacket> {
        self.devices
            .lock()
            .get(udid)
            .and_then(|device| device.last_h264.clone())
    }

    #[cfg(test)]
    fn peek_last_jpeg(&self, udid: &str) -> Option<ViewPacket> {
        self.devices
            .lock()
            .get(udid)
            .and_then(|device| device.last_jpeg.clone())
    }
}

impl ViewSink for ViewHub {
    fn generation(&self, udid: &str) -> u64 {
        self.current_generation(udid)
    }

    fn advance(&self, udid: &str) -> u64 {
        let (next, created) = {
            let mut devices = self.devices.lock();
            match devices.get_mut(udid) {
                Some(device) => {
                    device.generation = device.generation.saturating_add(1);
                    // The producer is being replaced, so its cached keyframe and its clock
                    // are about to describe something that no longer exists. The CHANNEL is
                    // kept: the watchdog's restart path is stop-then-start, and dropping the
                    // sender on every generation bump would close and reopen the device
                    // stream on every restart.
                    device.last_h264 = None;
                    device.last_packet_at = None;
                    (device.generation, false)
                }
                None => {
                    devices.insert(udid.to_string(), DeviceView::new(1));
                    (1, true)
                }
            }
        };
        if created {
            let _ = self.roster.send(udid.to_string());
        }
        next
    }

    fn publish(&self, packet: ViewPacket) -> bool {
        let (tx, created) = {
            let mut devices = self.devices.lock();
            // The stale-generation refusal comes FIRST so a packet from a producer that has
            // already been replaced can never create an entry for a device the hub has
            // forgotten.
            if packet.kind == ViewKind::H264 {
                let current = devices
                    .get(&packet.udid)
                    .map(|device| device.generation)
                    .unwrap_or(0);
                if packet.generation != current {
                    return false;
                }
            }
            let created = !devices.contains_key(&packet.udid);
            let device = devices
                .entry(packet.udid.clone())
                .or_insert_with(|| DeviceView::new(packet.generation));
            match packet.kind {
                ViewKind::Jpeg => device.last_jpeg = Some(packet.clone()),
                ViewKind::H264 if packet.key => device.last_h264 = Some(packet.clone()),
                ViewKind::H264 => {}
            }
            device.last_packet_at = Some(Instant::now());
            (device.tx.clone(), created)
        };
        if created {
            let _ = self.roster.send(packet.udid.clone());
        }
        // Outside the lock: `send` walks the receiver list, and this runs per frame per
        // device.
        let _ = tx.send(packet);
        true
    }
}

/// One thing that happened to one of a client's devices.
enum ClientEvent {
    Packet(ViewPacket),
    /// This device outran its own ring. Carries the udid, which is the whole gain of the
    /// per-device split: a lag is now attributable and costs only the device that caused it.
    Lagged {
        udid: String,
        dropped: u64,
    },
}

/// Move one device's packets onto the client's queue, classifying that device's own lag.
///
/// This is where the two hard-won rules about `Lagged` now live, and both were bugs first:
///
/// * **Never return on `Lagged`.** It is not a disconnect; the device is still producing.
/// * **Drain to the present BEFORE reporting it.** At the moment `recv` reports a lag the
///   cursor sits at the oldest retained value, so a keyframe written now would be followed
///   by up to a ring's worth of traffic that predates it. Across a generation bump that is a
///   gen-N key followed by gen-N-1 packets: the worker flips generation, closes its decoder,
///   and nothing reopens it.
///
/// `try_recv` reporting `Lagged` has already advanced the cursor and consumed the report, so
/// it has to be accumulated here rather than left for the outer arm — leaving it meant no
/// resync, no log, and any keyframe in the skipped span silently gone while every later
/// delta was delivered normally. That is precisely "packets keep arriving and nothing
/// paints".
/// Pump one device's packets to this client until the channel or the client goes.
///
/// Returns the udid it was serving. The caller needs that: a forwarder ends when the hub
/// forgets its device, and the client has to stop believing it is still subscribed —
/// otherwise the device can never be picked up again. See the `known` set below.
async fn forward_device(
    udid: String,
    mut rx: broadcast::Receiver<ViewPacket>,
    tx: mpsc::Sender<ClientEvent>,
) -> String {
    loop {
        match rx.recv().await {
            Ok(packet) => {
                if tx.send(ClientEvent::Packet(packet)).await.is_err() {
                    return udid;
                }
            }
            Err(RecvError::Lagged(mut dropped)) => {
                loop {
                    match rx.try_recv() {
                        // Everything still in the ring is history relative to the keyframe
                        // the resync is about to send, so it is discarded rather than
                        // forwarded.
                        Ok(_) => {}
                        Err(TryRecvError::Lagged(more)) => dropped += more,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Closed) => return udid,
                    }
                }
                if tx
                    .send(ClientEvent::Lagged {
                        udid: udid.clone(),
                        dropped,
                    })
                    .await
                    .is_err()
                {
                    return udid;
                }
            }
            Err(RecvError::Closed) => return udid,
        }
    }
}

/// Does this handshake carry the hub's token?
///
/// Split out and pure so the decision is testable without a socket — the property that matters
/// is "no token, no frames", and that must not depend on getting a TCP fixture right.
///
/// The token is compared in **constant time**: a byte-at-a-time comparison against a secret an
/// attacker can retry at loopback speed is a secret an attacker can read.
///
/// **`Origin` is deliberately not a gate, and that was learned the hard way.** The first
/// version of this refused any handshake carrying an `Origin`, on the theory that our own
/// WebView sends none and a browser page does. The running app disproved it in seconds: in dev
/// the page is served by vite from `http://localhost:5173` (`tauri.conf.json` devUrl), so our
/// own client sends `Origin` too — the log filled with `view websocket handshake refused` in a
/// reconnect loop and the fleet view went blank. An allowlist instead would have to track
/// `tauri://localhost`, `https://tauri.localhost` and the dev origin across platforms: more
/// surface, no more safety.
///
/// The token alone answers the attacker `Origin` was aimed at. A web page can open a
/// cross-origin WebSocket — they bypass CORS — but it cannot **read** the token, which reaches
/// only our own WebView, over IPC, and is 244 bits of CSPRNG.
fn handshake_is_authorised(path_and_query: &str, token: &str) -> bool {
    let Some(query) = path_and_query.split_once('?').map(|(_, q)| q) else {
        return false;
    };
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .any(|(key, value)| key == "k" && bytes_eq_ct(value.as_bytes(), token.as_bytes()))
}

/// Constant-time byte comparison. Same helper as `local_api.rs`, same reason.
fn bytes_eq_ct(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

type HandshakeRequest = tokio_tungstenite::tungstenite::handshake::server::Request;
type HandshakeResponse = tokio_tungstenite::tungstenite::handshake::server::Response;
type HandshakeRejection = tokio_tungstenite::tungstenite::handshake::server::ErrorResponse;

/// Accept or reject one handshake.
///
#[allow(clippy::result_large_err)]
fn gate_handshake(
    request: &HandshakeRequest,
    response: HandshakeResponse,
    token: &str,
) -> Result<HandshakeResponse, HandshakeRejection> {
    let target = request
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("");
    if handshake_is_authorised(target, token) {
        return Ok(response);
    }
    log::warn!("view websocket handshake refused: no valid token");
    Err(tokio_tungstenite::tungstenite::http::Response::builder()
        .status(tokio_tungstenite::tungstenite::http::StatusCode::UNAUTHORIZED)
        .body(None)
        .expect("a 401 with an empty body is always constructible"))
}

/// `result_large_err`: the handshake callback's `Err` is tungstenite's
/// `http::Response<Option<String>>` (136 bytes) and its shape is fixed by the `accept_hdr_async`
/// signature — clippy's suggestion to box it does not typecheck against that callback. The cost
/// is one 136-byte value, once per connection, and only on the rejection path.
#[allow(clippy::result_large_err)]
async fn serve_client(hub: Arc<ViewHub>, stream: tokio::net::TcpStream) -> anyhow::Result<()> {
    let _ = stream.set_nodelay(true);
    // Checked **during** the handshake, so an unauthorised client is refused before the
    // connection is upgraded — it never reaches `replay_latest` and so never receives a byte
    // of anyone's screen. Doing it after `accept_async` would mean the socket is already a
    // live WebSocket when we decide, which is a race worth not having.
    let token = hub.token.clone();
    let mut ws = tokio_tungstenite::accept_hdr_async(
        stream,
        |request: &HandshakeRequest, response: HandshakeResponse| {
            gate_handshake(request, response, &token)
        },
    )
    .await?;
    // Roster BEFORE the device snapshot, and the snapshot before the replay. Two ordering
    // windows now, and both lose a device permanently and silently if got wrong: a phone
    // created between the snapshot and the roster subscribe would never be forwarded to this
    // client at all, and a packet published between the replay's awaited writes and the
    // subscribe would be lost for that client. A duplicate is harmless; a missing keyframe is
    // a black canvas.
    let mut roster = hub.roster.subscribe();
    let subscriptions = hub.subscribe_all();
    let mut known: HashSet<String> = subscriptions.iter().map(|(udid, _)| udid.clone()).collect();
    replay_latest(&hub, &mut ws).await?;

    let (tx, mut rx) = mpsc::channel::<ClientEvent>(CLIENT_QUEUE_CAP);
    // Held here rather than detached, so that dropping this future on client disconnect
    // aborts every forwarder with it.
    let mut forwarders = JoinSet::new();
    for (udid, receiver) in subscriptions {
        forwarders.spawn(forward_device(udid, receiver, tx.clone()));
    }

    loop {
        let first = tokio::select! {
            biased;
            event = rx.recv() => match event {
                Some(event) => event,
                // Every forwarder is gone and so is the last sender clone; nothing else can
                // arrive. `tx` is still held below, so in practice this cannot fire.
                None => break,
            },
            announced = roster.recv() => {
                let fresh = match announced {
                    Ok(udid) => vec![udid],
                    // Missing an announcement must never kill a client, so re-snapshot and
                    // pick up whatever was missed.
                    Err(RecvError::Lagged(_)) => hub.known_devices(),
                    Err(RecvError::Closed) => Vec::new(),
                };
                // **Reap first, and this is the whole fix.** `known` used to only ever
                // grow. A device that left the fleet had its channel dropped by
                // `ViewHub::forget`, which ended its forwarder — but its udid stayed in
                // this set, so when the phone came back and the hub announced its *new*
                // channel, `insert` answered "already known" and no forwarder was ever
                // spawned for it. That client then received nothing for that device for
                // the rest of the connection, with the producer running, a keyframe sent
                // and no error anywhere.
                //
                // Measured on the fleet 17/08/2026: reboot one phone of twenty and the
                // host's paint evidence sits at 19/20 permanently. Reconnecting the socket
                // is what fixed it, because a fresh client calls `subscribe_all`.
                //
                // Reaping here rather than in a select arm keeps it ordered: a forwarder
                // whose device was forgotten has already returned by the time that device
                // can be announced again, since being announced needs a new producer.
                while let Some(finished) = forwarders.try_join_next() {
                    if let Ok(gone) = finished {
                        known.remove(&gone);
                    }
                }
                for udid in fresh {
                    if !known.insert(udid.clone()) {
                        continue;
                    }
                    let Some(receiver) = hub.subscribe_one(&udid) else {
                        continue;
                    };
                    forwarders.spawn(forward_device(udid.clone(), receiver, tx.clone()));
                    // Then replay what the device has already cached. A device is announced
                    // when the hub first sees it, and its producer publishes its first
                    // keyframe immediately afterwards -- so between the announcement and this
                    // client subscribing there is a window in which a broadcast receiver that
                    // did not exist yet cannot be handed anything. A loopback test found it:
                    // the client got the replay and then nothing at all for the new device.
                    //
                    // Subscribing FIRST and replaying second is what makes this safe in both
                    // directions: anything published after the subscribe arrives on the
                    // channel, anything before it is in the cache, and the overlap is a
                    // duplicate frame. A duplicate is invisible; a missing keyframe is a
                    // black canvas.
                    replay_device(&hub, &mut ws, &udid).await?;
                }
                continue;
            }
        };

        let mut batch: Vec<ViewPacket> = Vec::new();
        let mut lagged: Vec<(String, u64)> = Vec::new();
        let mut take = |event: ClientEvent| match event {
            ClientEvent::Packet(packet) => batch.push(packet),
            ClientEvent::Lagged { udid, dropped } => lagged.push((udid, dropped)),
        };
        take(first);
        while let Ok(event) = rx.try_recv() {
            take(event);
        }

        for (udid, dropped) in &lagged {
            // Only the device that lagged loses its batch. The blanket clear this replaces
            // punished every phone on the socket for one slow one, purely because a shared
            // ring could not say which had fallen behind.
            batch.retain(|packet| &packet.udid != udid);
            log::debug!("view subscriber lagged on {udid}, dropped {dropped} packets; resyncing");
        }
        for packet in coalesce_for_live(batch) {
            ws.feed(Message::Binary(encode_packet(&packet).into()))
                .await?;
        }
        for (udid, _) in lagged {
            replay_device(&hub, &mut ws, &udid).await?;
        }
        ws.flush().await?;
    }
    Ok(())
}

/// Hand the client everything **one** device can be caught up from: its newest JPEG and its
/// newest keyframe.
///
/// Used for two different catch-ups that want exactly the same bytes — a device the client
/// has just subscribed to, and a device whose forwarder has drained its receiver to the
/// present after a lag. In the lag case it is only correct after that drain; see
/// [`forward_device`].
///
/// After a lag it used to rewrite the newest keyframe of *every* device, which was never
/// about correctness: a shared ring could not attribute a lag, so the only safe answer was
/// to resync everything. Per-device channels make the question answerable.
async fn replay_device(
    hub: &ViewHub,
    ws: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    udid: &str,
) -> anyhow::Result<()> {
    for packet in hub.cached_frames(udid) {
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
    let replay: Vec<ViewPacket> = {
        let devices = hub.devices.lock();
        devices
            .values()
            .flat_map(|device| {
                device
                    .last_jpeg
                    .iter()
                    .chain(device.last_h264.iter())
                    .cloned()
            })
            .collect()
    };
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
/// Still per-device even though the channels are, because the batch drained from one
/// client's queue interleaves every device that had traffic in that window — the shared
/// thing is now the socket rather than the ring, and one slow phone must still not turn
/// every other phone into a slideshow.
///
/// For a device at or under the threshold, every packet is forwarded in order -- that is
/// what makes motion look like motion. Past it, the device collapses to its newest packet,
/// preceded by the newest key in the same batch if that newest is a delta, so a decoder that
/// lost its GOP can resync without waiting another `i-frame-interval`.
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
    let behind: HashSet<String> = counts
        .iter()
        .filter(|(_, count)| **count > COALESCE_AFTER_FRAMES)
        .map(|(udid, _)| (*udid).to_string())
        .collect();
    // A device that is keeping up must not lose frames just because another device on the
    // same socket fell behind.
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
        assert_eq!(hub.peek_last_h264("a").map(|p| p.bytes), Some(vec![2]));
        hub.advance("a");
        assert!(hub.peek_last_h264("a").is_none());
    }

    // Both walls, still decided at compile time — but the arithmetic behind them inverted
    // with the per-device split, so both numbers and both reasons changed. The upper one is
    // now fleet-MULTIPLIED: every device pays this ring in full, which is what makes a
    // generous value expensive rather than merely wasteful. The lower one is now expressed
    // purely in time, and must contain no mention of fleet size at all — that independence
    // is the entire point of the change.
    const _: () = assert!(
        DEVICE_BROADCAST_CAP <= 512,
        "every device pays this ring in full: 512 x ~2.6 KB x 20 phones is ~27 MB of video \
         held in memory, and tokio allocates the slots when the channel is created"
    );
    const _: () = assert!(
        DEVICE_BROADCAST_CAP >= 96,
        "at 24 fps a device needs about four seconds of its own ring to survive a host \
         hitch without a resync; this is per device, so the fleet size does not enter into it"
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
        // The shared thing is now the SOCKET rather than the channel, and this is still the
        // failure mode that matters at scale: one slow phone must not turn every other phone
        // into a slideshow.
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
        assert!(hub.peek_last_jpeg("a").is_none());
        assert_eq!(hub.peek_last_h264("a").map(|p| p.bytes), Some(vec![9]));
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

    #[test]
    fn a_flood_from_one_device_does_not_cost_another_device_a_frame() {
        // The direct assertion of AGENTS.md 9.68, and it cannot even be written against a
        // shared ring: there, `a` overrunning the channel evicts `b`'s packets too, so `b`
        // lags without having produced anything.
        let hub = ViewHub::new();
        hub.advance("a");
        hub.advance("b");
        let mut a = hub.subscribe_one("a").expect("a exists");
        let mut b = hub.subscribe_one("b").expect("b exists");

        assert!(hub.publish(h264("b", true, 1)));
        for byte in 0..(DEVICE_BROADCAST_CAP as u8 + 50) {
            assert!(hub.publish(h264("a", false, byte)));
        }

        // `b` produced one packet and receives exactly it, unaffected by the flood.
        match b.try_recv() {
            Ok(packet) => assert_eq!(packet.bytes, vec![1]),
            other => panic!("b should be untouched by a's flood, got {other:?}"),
        }
        // `a` overran its own ring and only `a` did.
        assert!(matches!(a.try_recv(), Err(TryRecvError::Lagged(_))));
    }

    #[test]
    fn advance_keeps_the_device_channel_open() {
        // The watchdog restarts producers routinely, and a restart is stop-then-start, so a
        // generation bump that closed the channel would tear down every client's canvas for
        // this device on every recovery.
        let hub = ViewHub::new();
        hub.advance("a");
        let mut rx = hub.subscribe_one("a").expect("a exists");
        assert_eq!(hub.advance("a"), 2);
        assert!(hub.publish(ViewPacket {
            udid: "a".into(),
            generation: 2,
            kind: ViewKind::H264,
            width: 10,
            height: 20,
            key: true,
            bytes: vec![7],
        }));
        let packet = rx.try_recv().expect("the same receiver still delivers");
        assert_eq!(packet.generation, 2);
        assert_eq!(packet.bytes, vec![7]);
    }

    #[test]
    fn forget_closes_only_that_device() {
        let hub = ViewHub::new();
        hub.advance("a");
        hub.advance("b");
        let mut a = hub.subscribe_one("a").expect("a exists");
        let mut b = hub.subscribe_one("b").expect("b exists");
        hub.forget("a");
        assert!(matches!(a.try_recv(), Err(TryRecvError::Closed)));
        assert!(hub.publish(h264("b", true, 5)));
        assert_eq!(b.try_recv().expect("b still flows").bytes, vec![5]);
        assert_eq!(hub.known_devices(), vec!["b".to_string()]);
    }

    #[test]
    fn a_device_that_appears_after_a_client_connected_is_announced() {
        // Clients connect once, at app start, before any phone is enumerated. Without the
        // roster every device discovered later would be a canvas that never paints.
        let hub = ViewHub::new();
        let mut roster = hub.roster.subscribe();
        hub.advance("late");
        assert_eq!(roster.try_recv().expect("announced"), "late".to_string());
        // Announced once, not on every generation bump.
        hub.advance("late");
        assert!(roster.try_recv().is_err());
    }

    #[test]
    fn an_ios_preview_announces_its_device_once_not_once_per_frame() {
        // `publish_jpeg` runs per preview frame. Announcing unconditionally would fill the
        // roster with one udid several times a second, and every client would then see
        // `Lagged` on the roster and re-snapshot the whole fleet for nothing.
        let hub = ViewHub::new();
        let mut roster = hub.roster.subscribe();
        for _ in 0..10 {
            assert!(hub.publish_jpeg("phone", vec![0xff, 0xd8]));
        }
        assert_eq!(roster.try_recv().expect("announced"), "phone".to_string());
        assert!(roster.try_recv().is_err(), "announced exactly once");
    }

    #[tokio::test]
    async fn a_client_receives_a_replay_then_live_packets_over_a_real_socket() {
        // The first test ever to run `serve_client`, `replay_latest` or the forwarder path.
        // Those had zero coverage while carrying every comment in this file about silent
        // frame loss, and this change rewrites all three.
        use futures_util::StreamExt;

        let hub = ViewHub::new();
        hub.advance("a");
        assert!(hub.publish(h264("a", true, 1)));
        let port = Arc::clone(&hub).listen().await.expect("bind loopback");

        // `client_async` over a socket we dial ourselves, rather than `connect_async`: the
        // latter is behind tokio-tungstenite's `connect` feature, which this crate
        // deliberately does not enable — it would pull in DNS and TLS for a loopback
        // listener that will never need either.
        let socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("dial the loopback listener");
        let (mut client, _) =
            tokio_tungstenite::client_async(hub.endpoint().expect("the hub is listening"), socket)
                .await
                .expect("websocket handshake");

        // Replay: the cached keyframe of every device that already existed.
        let replayed = client.next().await.expect("a message").expect("no error");
        assert_eq!(
            replayed.into_data().to_vec(),
            encode_packet(&h264("a", true, 1))
        );

        // A device discovered after the client connected still reaches it, via the roster.
        hub.advance("b");
        assert!(hub.publish(h264("b", true, 2)));
        let live = tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .expect("a packet for a device added after connect")
            .expect("a message")
            .expect("no error");
        assert_eq!(
            live.into_data().to_vec(),
            encode_packet(&h264("b", true, 2))
        );
    }

    #[tokio::test]
    async fn a_device_that_leaves_and_comes_back_still_reaches_an_open_client() {
        // The failure this exists for, measured on the fleet 17/08/2026: reboot one phone
        // of twenty and the host's paint evidence sits at 19/20 for as long as the window
        // stays open. The producer is running, its keyframe was sent, no error is logged
        // anywhere -- the client simply never subscribed to the phone's *new* channel,
        // because its `known` set still held the udid from the old one. Reconnecting the
        // socket was the only cure, since a fresh client calls `subscribe_all`.
        //
        // A phone leaving and returning is the most ordinary event this fleet has: a
        // reboot, a cable knocked loose, a hub that browns out.
        use futures_util::StreamExt;

        let hub = ViewHub::new();
        hub.advance("a");
        assert!(hub.publish(h264("a", true, 1)));
        let port = Arc::clone(&hub).listen().await.expect("bind loopback");
        let socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("dial the loopback listener");
        let (mut client, _) =
            tokio_tungstenite::client_async(hub.endpoint().expect("the hub is listening"), socket)
                .await
                .expect("websocket handshake");
        let replayed = client.next().await.expect("a message").expect("no error");
        assert_eq!(
            replayed.into_data().to_vec(),
            encode_packet(&h264("a", true, 1))
        );

        // The phone leaves the fleet: the 3 s scan reconciles the hub against the registry.
        hub.forget("a");
        // ...and comes back, with a new channel and a fresh keyframe.
        hub.advance("a");
        assert!(hub.publish(h264("a", true, 9)));

        let live = tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .expect("the returned device must still reach an already-connected client")
            .expect("a message")
            .expect("no error");
        assert_eq!(
            live.into_data().to_vec(),
            encode_packet(&h264("a", true, 9))
        );
    }
    #[test]
    fn the_handshake_gate_answers_the_only_question_that_matters() {
        // Pure, so the decision is provable without a socket: no token, no frames.
        let token = "0123456789abcdef";
        assert!(handshake_is_authorised("/?k=0123456789abcdef", token));
        assert!(handshake_is_authorised(
            "/?other=1&k=0123456789abcdef",
            token
        ));

        // No token at all — the shape every caller had before this existed.
        assert!(!handshake_is_authorised("/", token));
        assert!(!handshake_is_authorised("/?k=", token));
        assert!(!handshake_is_authorised("/?j=0123456789abcdef", token));
        // Wrong token, including the prefix that a byte-at-a-time compare would leak.
        assert!(!handshake_is_authorised("/?k=0123456789abcdee", token));
        assert!(!handshake_is_authorised("/?k=0123456789abcde", token));
        assert!(!handshake_is_authorised("/?k=0123456789abcdefff", token));
    }

    #[test]
    fn every_hub_mints_its_own_token_and_publishes_it_in_the_endpoint() {
        let a = ViewHub::new();
        let b = ViewHub::new();
        assert_ne!(a.token, b.token, "a fixed token would be no token at all");
        assert_eq!(a.token.len(), 64, "two v4 UUIDs, hex, no dashes");
        assert!(a.token.chars().all(|c| c.is_ascii_hexdigit()));
        // Not listening yet, so there is nothing to hand out.
        assert!(a.endpoint().is_none());
    }

    #[tokio::test]
    async fn a_client_without_the_token_is_refused_and_receives_no_frame() {
        // The property in one test: an unauthorised client must not get a single byte of
        // anyone's screen. Before this, connecting to the port was the whole authorisation.
        use futures_util::StreamExt;

        let hub = ViewHub::new();
        hub.advance("a");
        assert!(hub.publish(h264("a", true, 1)));
        let port = Arc::clone(&hub).listen().await.expect("bind loopback");

        let socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("dial the loopback listener");
        let refused =
            tokio_tungstenite::client_async(format!("ws://127.0.0.1:{port}/"), socket).await;
        assert!(
            refused.is_err(),
            "a tokenless handshake must be rejected, not upgraded"
        );

        // And the hub is still serving: the refusal is per-connection, not a wedged listener.
        let socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("dial again");
        let (mut client, _) =
            tokio_tungstenite::client_async(hub.endpoint().expect("listening"), socket)
                .await
                .expect("the real endpoint still works");
        let replayed = client.next().await.expect("a message").expect("no error");
        assert_eq!(
            replayed.into_data().to_vec(),
            encode_packet(&h264("a", true, 1))
        );
    }
}
