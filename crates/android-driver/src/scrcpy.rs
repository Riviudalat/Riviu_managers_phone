//! scrcpy-server 3.3.4 as the Android **view** encoder.
//!
//! Not a [`riviu_core::FrameSource`]. H.264 never enters `StreamHub`. The
//! protocol here is the official 3.3.4 socket: dummy byte, 64-byte name,
//! 12-byte video header (`codec` + width + height), then 12-byte media
//! packets. 3.3.4 has no session packets; config is bit 63 and key is bit
//! 62. Audio stays off.
//!
//! **Control is on, for exactly one message.** The socket exists so the host can send
//! `RESET_VIDEO` and get a fresh keyframe without restarting the producer — one byte against
//! ~11.5 s of black tile. That changes the handshake: the server accepts one socket per
//! enabled channel and only then closes its listener, so the control socket must be opened
//! **between** reading the dummy byte and reading the device name (see
//! [`ScrcpyStream::try_accept`]). One `adb forward` serves both.
//!
//! **Input is split between the two paths, on purpose.** Taps, keys and text stay on
//! uiautomator2 — `INJECT_TEXT` cannot type Vietnamese diacritics, and a discrete tap is not
//! slow enough there to be worth the coordinate risk. The continuous middle of a drag goes
//! through this socket, because it previously went nowhere at all: samples were buffered and
//! replayed as one swipe on release, so the phone stood still under the operator's finger.
//! See [`CONTROL_MESSAGE_INJECT_TOUCH`] for the coordinate trap and how it is closed.
//!
//! 4.1 is not pinned: live Note 8 (API 26) dies in `dequeueOutputBuffer`
//! on `OMX.Exynos.AVC.Encoder`. 3.3.4 on the same phone returns Annex-B.
//! Tile `max_size` is 480, not 176: Redmi API 35 rejects `MediaCodec.configure`
//! at 80×176, and 320 on Note 8 yields SPS `avc1.42000D` that WebView2 can
//! refuse (measured 14/08/2026).
//!
//! Launch is `app_process` with `CLASSPATH` pointing at a JAR we pushed to
//! `/data/local/tmp/riviu-scrcpy-server` — our own name, same reason minicap
//! is `riviu-minicap.apk`. Forward is `adb forward tcp:0` **once**; stale
//! forwards to the same abstract socket are pruned first (AGENTS.md §9).

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::TcpStream;

use crate::adb::AdbProgram;
use crate::frames;

/// Official server version string. `Options.parse` refuses any other value.
pub const SERVER_VERSION: &str = "3.3.4";

/// Where we push the JAR. Named for us so a co-resident `scrcpy` or
/// `genscrcpy.jar` is not mistaken for ours.
pub const REMOTE_SERVER: &str = "/data/local/tmp/riviu-scrcpy-server";

const MAIN_CLASS: &str = "com.genymobile.scrcpy.Server";

/// `'h264'` as a big-endian u32. The only codec this path accepts.
pub const CODEC_H264: u32 = 0x6832_3634;

/// 3.3.4 `Streamer.PACKET_FLAG_CONFIG`. 4.1 moved this to bit 62 and used
/// bit 63 for session packets — do not mix the two parsers.
pub const PACKET_FLAG_CONFIG: u64 = 1 << 63;
pub const PACKET_FLAG_KEY_FRAME: u64 = 1 << 62;

const DEVICE_NAME_LEN: usize = 64;
const VIDEO_HEADER_LEN: usize = 12;
const HEADER_LEN: usize = 12;
const MAX_PACKET_BYTES: u32 = 4 * 1024 * 1024;
/// Dummy is written in the same `accept()` call. If it is not here in 2 s
/// this TCP did not pair; drop it and try again.
const DUMMY_DEADLINE: Duration = Duration::from_secs(2);
/// Name is on the main thread right after accept; the 12-byte video header
/// is `writeVideoHeader` on the video thread after `capture.init`.
const META_DEADLINE: Duration = Duration::from_secs(20);

/// Encode size / bitrate / fps for one view. Retune = stop + start the same
/// process with a different preset, not a second `app_process`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPreset {
    Tile,
    Overlay,
}

/// Macroblocks a level-3.0 decoder must accept (H.264 Table A-1, MaxFS).
///
/// Load-bearing because every entry of the desktop's codec candidate ladder is level 3.0
/// (`avc1.42E01E`, `avc1.42001E`, `avc1.4D401E`). Encode above this and the phone produces
/// a stream its own SPS says is out of level, which WebView2 is entitled to refuse -- and
/// the refusal arrives asynchronously, as a black canvas.
pub const LEVEL_3_0_MAX_MACROBLOCKS: u32 = 1620;

/// Squarest screen this cap is computed for: 16:9.
///
/// The budget is on frame **area**, so the safe long edge depends on the aspect ratio, and
/// the squarer the screen the smaller it gets. Deriving the cap from the two phones that
/// happen to be plugged in here would have shipped a number that breaks on the next one:
/// at a long edge of 900, this fleet's 19.5:9 and 18.5:9 both fit, but a plain 18:9 comes
/// out at 1653 macroblocks and 16:9 at 1824. Both over.
pub const SQUAREST_SUPPORTED_ASPECT: (u32, u32) = (9, 16);

/// Long edge the overlay asks for, and the ceiling every preset is clamped to.
///
/// 832 is the largest multiple of 16 that keeps a 16:9 screen inside level 3.0 (1560 of
/// 1620 macroblocks); the true break-even is 848 and the next step, 864, is already over.
/// An earlier commit put 1600 here, which is ~4500 macroblocks -- 2.8x the limit -- and it
/// only escaped notice because `view_set_preset` had no caller, so no phone was ever asked
/// for it.
///
/// **Known limit:** a 4:3 screen exceeds the budget even at 832. This is a phone farm and
/// no such device has appeared, but a tablet would need the cap derived per device from the
/// resolution scrcpy reports rather than from a constant.
pub const MAX_LONG_EDGE: u32 = 832;

/// Long edge the overlay asks for. One phone at a time rather than sixteen tiles is what
/// makes the larger encode affordable.
pub const OVERLAY_LONG_EDGE: u32 = MAX_LONG_EDGE;

impl ViewPreset {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "tile" => Ok(Self::Tile),
            "overlay" => Ok(Self::Overlay),
            other => anyhow::bail!("unknown view preset {other:?}; expected tile or overlay"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tile => "tile",
            Self::Overlay => "overlay",
        }
    }

    /// The smallest encode this preset may ever ask for.
    ///
    /// A **floor**, not a default, and the difference is what keeps an operator's
    /// "low quality" from breaking the stream outright: 176 fails
    /// `MediaCodec.configure` on Redmi API 35, and 320 encodes on both phones but
    /// gives the Note 8 a Baseline *level 1.3* SPS (`avc1.42000D`) that WebView2 may
    /// refuse. 480 is the size that returned a packet on the Redmi in the 3.3.4/4.1
    /// probe and lifts the level off 1.3 (14/08/2026). So quality below Medium buys a
    /// smaller bitrate, never a smaller frame.
    ///
    /// **Overlay is sized from how large it is displayed, not from what encodes.**
    ///
    /// 600 was upscaling badly and the operator saw it as a broken picture. The
    /// arithmetic: `max_size` caps the **long** edge, so a 1080×2400 phone encoded at 600
    /// is 270 px wide, while the overlay displays it at 400 px by default and up to 760
    /// (`FOCUS_ZOOM` in `zoom.ts`) — a 1.48× upscale at rest and 2.81× at full zoom.
    ///
    /// 1600 on the long edge is 720 px wide on that phone, so the default 400 is a
    /// downscale and even full zoom is 1.06×. This is one phone at a time, not sixteen
    /// tiles, which is what makes the larger encode affordable.
    pub fn max_size(self) -> u32 {
        match self {
            Self::Tile => 480,
            Self::Overlay => OVERLAY_LONG_EDGE,
        }
    }

    /// The bitrate at [`StreamQuality::Medium`], which is what shipped before quality
    /// was settable — so the default path encodes exactly as it did.
    ///
    /// Overlay CSS-scales this same encode. 400 kbps / 15 fps was a slide show;
    /// 800 kbps still blocks on TikTok motion.
    pub fn bit_rate(self) -> u32 {
        match self {
            Self::Tile => 1_200_000,
            // Raised with the frame size above. 1.5 Mbps was chosen for a 270-px-wide
            // encode; spending the same on 720 px would trade a soft upscale for a
            // blocky one on TikTok motion.
            Self::Overlay => 4_000_000,
        }
    }

    /// The highest frame rate this preset will ask a phone for.
    ///
    /// **Not the same question for the two presets, and it used to be answered as if it
    /// were.** The overlay is the one phone the operator is working on; a tile is one of
    /// twenty they are glancing at. Twenty tiles at the same rate as the overlay is twenty
    /// times the decode work for the part nobody is looking at, and it is paid in the one
    /// place it hurts — the WebView that also has to keep the overlay smooth.
    ///
    /// Measured on this fleet, twenty phones, whole-app CPU across the Tauri and WebView
    /// processes:
    ///
    /// | grid rate | CPU (one core = 100%) | working set |
    /// |---|---|---|
    /// | 24 fps | **135 %** | 1166 MB |
    /// | 5 fps | **85 %** | 1121 MB |
    ///
    /// Note the saving is **sub-linear** — 4.8x fewer frames buys 37 % less CPU, not 80 %.
    /// That is `video_codec_options=i-frame-interval:int=1` doing what it was asked: one
    /// keyframe per second regardless of rate, so the *proportion* of expensive frames rises
    /// as the rate falls. Below about ten there is little left to win, which is why this is
    /// 10 and not 1.
    ///
    /// The operator's frame-rate setting still applies; this is a ceiling on top of it, so
    /// asking for 24 gives the overlay 24 and a tile 10.
    pub fn max_fps(self) -> u32 {
        match self {
            Self::Tile => 10,
            Self::Overlay => 30,
        }
    }

    /// Apply the operator's quality and frame-rate choice to this preset.
    ///
    /// Both halves of this were settings the app **stored and never read**:
    /// `StreamSettings::grid_quality` had no reader anywhere in the tree, and
    /// `set_stream_settings` overwrote `fps` with the compiled-in constant. So an
    /// operator could move either control and nothing whatsoever happened.
    ///
    /// Quality moves the bitrate freely and the frame size only **upward**, because
    /// downward is where the measured encoder failures live (see [`Self::max_size`]).
    /// Frame rate is clamped into a range the fleet has actually run at; a 0 would ask
    /// scrcpy for an unbounded rate and a 120 asks for one no phone here delivers.
    pub fn tuned(self, quality: riviu_core::StreamQuality, fps: u32) -> ViewTuning {
        let floor = self.max_size();
        let base = self.bit_rate();
        // Relative to the preset's own bitrate, not absolute numbers. Absolute ones made
        // High *worse* than Medium the moment Overlay's base moved to 4 Mbps — a higher
        // setting that degrades the picture is the kind of thing nobody reports as a bug,
        // they just stop trusting the control.
        let (size, bit_rate) = match quality {
            riviu_core::StreamQuality::Low => (floor, base / 2),
            riviu_core::StreamQuality::Medium => (floor, base),
            riviu_core::StreamQuality::High => (floor * 3 / 2, base * 3 / 2),
            riviu_core::StreamQuality::Extra => (floor * 2, base * 2),
        };
        ViewTuning {
            // Capped after the multiplier, deliberately. High and Extra scale the size up,
            // so leaving the cap on the base would let a quality setting walk the encode
            // out of level 3.0 -- which is a black canvas, not a soft picture, because the
            // refusal comes back asynchronously from the decoder.
            max_size: size.min(MAX_LONG_EDGE),
            bit_rate,
            max_fps: fps.clamp(MIN_VIEW_FPS, self.max_fps()),
        }
    }
}

/// The lowest frame rate worth encoding at. Below this the tile reads as a series of
/// stills, and an operator who wanted that would turn the view off instead.
pub const MIN_VIEW_FPS: u32 = 5;

/// What one view will actually ask the encoder for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewTuning {
    pub max_size: u32,
    pub bit_rate: u32,
    pub max_fps: u32,
}

/// One parsed media packet after the video header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScrcpyPacket {
    Media {
        config: bool,
        key: bool,
        payload: Vec<u8>,
    },
}

/// Device name + codec + size from the 3.3.4 hello.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrcpyHello {
    pub device_name: String,
    pub codec: u32,
    pub width: u32,
    pub height: u32,
}

/// The most bytes `app_process` will take in its **argv** on this fleet before it dies.
///
/// Measured 16/08/2026 on SM-G955F and SM-G950F (Android 9), and the threshold is identical
/// and razor sharp on both: **254 bytes of argv runs, 255 aborts.** Past it the server prints
///
/// ```text
/// stack corruption detected (-fstack-protector)
/// Aborted
/// ```
///
/// to its stdout and dies — *after* it has answered the handshake and sent the device name
/// and video header, so the host sees a perfectly good hello followed by no video at all.
///
/// It is argv specifically, not the command. Sixty extra bytes added to the shell line as an
/// **environment** assignment stream fine; twelve added to argv abort. So `CLASSPATH` (44 of
/// those bytes) is free and shortening it buys nothing.
///
/// This is the real cause of AGENTS.md 9.71 — twenty phones, zero producers, no warning. The
/// suspicion recorded there, that `power_on=false` was not a valid option name, is wrong:
/// it is a valid key, unknown keys are only warned about, and an unknown key of the same
/// length crashes exactly the same way. What broke it was that turning the control socket on
/// added 24 bytes to a budget with 14 left.
pub const MAX_SERVER_ARGV: usize = 254;

/// The part of a launch command that [`MAX_SERVER_ARGV`] applies to.
///
/// Everything from `app_process` onward. The `CLASSPATH=` assignment in front of it is
/// environment, not argv, and does not count against the budget.
pub fn server_argv(command: &str) -> &str {
    match command.find("app_process") {
        Some(at) => &command[at..],
        None => command,
    }
}

/// Build the `adb shell` payload. Version must be exact `3.3.4`.
///
/// `scid` is hex (`Integer.parseInt(value, 16)`). Socket name is
/// `scrcpy_%08x`. `cleanup=false` keeps our JAR after the process exits so
/// the next start can skip the push.
///
/// `video_codec_options` uses scrcpy's `key[:type]=value` form. A third
/// colon (`i-frame-interval:int:2`) makes 3.3.4 throw `'=' expected` in
/// `CodecOption.parseOption` and exit before it binds the abstract socket
/// — the tile then shows "exited before it accepted a connection".
///
/// **Every option here is spent from a 254-byte budget** — see [`MAX_SERVER_ARGV`]. There
/// are about fourteen bytes of headroom at the longest tuning, so an option cannot simply be
/// added: one must come out first, and the guard test in this module is what says so before
/// twenty phones do.
pub fn launch_command(scid: u32, tuning: ViewTuning) -> String {
    format!(
        "CLASSPATH={REMOTE_SERVER} app_process / {MAIN_CLASS} {SERVER_VERSION} \
         scid={scid:08x} tunnel_forward=true audio=false control=true video=true \
         video_codec=h264 max_size={} max_fps={} video_bit_rate={} \
         video_codec_options=i-frame-interval:int=1 cleanup=false",
        tuning.max_size, tuning.max_fps, tuning.bit_rate
    )
}

/// The one control message this project sends: **ask for a keyframe**.
///
/// `ControlMessage.TYPE_RESET_VIDEO = 17`, one byte, no payload — confirmed against the
/// static values in the shipped jar. It reaches `Controller.resetVideo` ->
/// `SurfaceCapture.requestInvalidate`, and the server answers by logging
/// `Video capture reset` and emitting a fresh IDR.
///
/// **Why one byte matters.** `ControlMessageReader` has no framing: it reads a type byte and
/// then however many bytes that type implies. A partial write, or two writes interleaved from
/// different tasks, desynchronises it permanently — and one bad byte is not a dropped message,
/// it is `ControlProtocolException` -> `Ln.e("Controller error")` -> `onTerminated` ->
/// `Looper.quitSafely()`, which takes the **video** down too, on that phone. So this returns a
/// whole message as one array and the caller sends it with a single `write_all` under a lock.
/// There is deliberately no builder and no second message type: the blast radius of getting
/// this stream wrong is every tile on the device going black.
pub const fn reset_video() -> [u8; 1] {
    [17]
}

/// `INJECT_TOUCH_EVENT`, type `0x02`, 32 bytes.
///
/// **Sent now, where it was not before, and the reason for the change is a measurement.**
/// The old note here said the agent "is not slow — 130–280 ms a click", which is true and
/// beside the point: a *drag* never went through it at all. `FocusStream` buffered every
/// `pointerMove` and posted one swipe on release, so the phone did not move until the
/// operator let go. That is the thing that felt broken, and no round-trip figure shows it.
/// See AGENTS.md 9.77.
///
/// What has not changed is the rest of that note. `INJECT_TEXT` still walks a
/// `KeyCharacterMap` and still cannot type Vietnamese diacritics, so text stays on
/// uiautomator2, and so does every discrete tap: this path is for the continuous middle of a
/// gesture, where the win is real and a dropped sample costs nothing.
///
/// **The coordinate trap (upstream #4925).** The server calls `Device.getPhysicalPoint`,
/// which compares the size declared in this message against the size it is *currently*
/// encoding and returns null — silently ignoring the touch — when they differ. So the size
/// written here is never the caller's: it is the dimension of the last sample this host
/// actually received from that device, which is by construction the size the server is
/// encoding. A stale caller gets its coordinates rescaled rather than its touch dropped.
///
/// Layout, all big-endian: type, action, `u64` pointer id, `i32` x, `i32` y, `u16` width,
/// `u16` height, `u16` pressure as Q0.16 with `0xFFFF` meaning exactly 1.0, `u32` action
/// button, `u32` buttons.
///
/// One array, one `write_all`, same as [`reset_video`] and for the same reason: the reader on
/// the device has no framing, and desynchronising it kills the video too.
pub const CONTROL_MESSAGE_INJECT_TOUCH: u8 = 0x02;

/// Which end of a gesture a touch message carries.
///
/// The values are `AMOTION_EVENT_ACTION_*` and must stay these numbers — the server passes
/// them to `MotionEvent.obtain` unmapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchAction {
    Down,
    Up,
    Move,
}

impl TouchAction {
    pub fn code(self) -> u8 {
        match self {
            Self::Down => 0,
            Self::Up => 1,
            Self::Move => 2,
        }
    }

    pub fn parse(raw: &str) -> anyhow::Result<Self> {
        match raw {
            "down" => Ok(Self::Down),
            "up" => Ok(Self::Up),
            "move" => Ok(Self::Move),
            other => anyhow::bail!("unknown touch action {other:?}"),
        }
    }
}

/// A finger rather than a mouse.
///
/// The server reads this as a signed 64-bit value and treats `-1` — and only `-1` — as
/// `TOOL_TYPE_MOUSE`; every other id becomes `TOOL_TYPE_FINGER`. Apps that discriminate on
/// tool type (scroll physics, long-press timing) behave like a real touchscreen only on the
/// finger path, so this is 0 deliberately and not the mouse constant scrcpy's own client uses.
const TOUCH_POINTER_ID: u64 = 0;

/// `MotionEvent.BUTTON_PRIMARY`, which a finger holds while it is down and never on release.
const BUTTON_PRIMARY: u32 = 1;

pub fn inject_touch(
    action: TouchAction,
    x: i32,
    y: i32,
    frame_width: u16,
    frame_height: u16,
) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[0] = CONTROL_MESSAGE_INJECT_TOUCH;
    out[1] = action.code();
    out[2..10].copy_from_slice(&TOUCH_POINTER_ID.to_be_bytes());
    out[10..14].copy_from_slice(&x.to_be_bytes());
    out[14..18].copy_from_slice(&y.to_be_bytes());
    out[18..20].copy_from_slice(&frame_width.to_be_bytes());
    out[20..22].copy_from_slice(&frame_height.to_be_bytes());
    // Full pressure while the finger is down, exactly zero once it is lifted. A non-zero
    // pressure on an UP is what a stuck finger looks like to the framework.
    let (pressure, buttons) = match action {
        TouchAction::Up => (0u16, 0u32),
        _ => (u16::MAX, BUTTON_PRIMARY),
    };
    out[22..24].copy_from_slice(&pressure.to_be_bytes());
    // Action button stays 0: it names *which* mouse button changed state, and a finger has
    // none. Sending BUTTON_PRIMARY here would make the server report a mouse click.
    out[24..28].copy_from_slice(&0u32.to_be_bytes());
    out[28..32].copy_from_slice(&buttons.to_be_bytes());
    out
}

/// Longest stable prefix of the remote socket for every server we start.
///
/// Only a prefix is available: [`socket_name`] appends a random `scid`, so this is as
/// specific as a match against `adb forward --list` can be. Tied to `socket_name` by a
/// test rather than by eye, because the two drifting apart would silently turn a prune
/// into a no-op.
pub const FORWARD_PREFIX: &str = "localabstract:scrcpy_";

/// Abstract socket the server binds when `scid` is set.
pub fn socket_name(scid: u32) -> String {
    format!("scrcpy_{scid:08x}")
}

/// Push the JAR when the on-device byte count differs. Same size-only check
/// as minicap: the host file is already SHA-256 pinned in the installer
/// manifest, so a matching length is enough to skip a 120 s push.
pub async fn ensure_server(adb: &AdbProgram, serial: &str, local: &Path) -> anyhow::Result<()> {
    let local_len = tokio::fs::metadata(local)
        .await
        .with_context(|| format!("read {}", local.display()))?
        .len();
    let remote_len = adb
        .shell(serial, &format!("wc -c < {REMOTE_SERVER} 2>/dev/null"))
        .await
        .ok()
        .and_then(|out| out.trim().parse::<u64>().ok());
    if remote_len == Some(local_len) {
        return Ok(());
    }
    adb.device(
        serial,
        &["push", &local.display().to_string(), REMOTE_SERVER],
        Duration::from_secs(120),
    )
    .await
    .context("push the scrcpy server")?;
    Ok(())
}

/// Forward once to the scrcpy abstract socket. Reuses minicap's prune.
pub async fn forward(adb: &AdbProgram, serial: &str, scid: u32) -> anyhow::Result<u16> {
    frames::forward(adb, serial, &socket_name(scid)).await
}

/// A connected video socket after dummy, name and the 12-byte video header.
pub struct ScrcpyStream {
    reader: tokio::io::BufReader<TcpStream>,
    pub hello: ScrcpyHello,
    pending_config: Option<Vec<u8>>,
    width: u32,
    height: u32,
}

/// Why a video-socket attempt failed. Retry is only safe before `accept()`.
#[derive(Debug)]
pub enum AcceptError {
    /// adb forwarded TCP but the abstract socket was not listening yet.
    /// `accept()` was not consumed; another TCP is allowed.
    NotListening(anyhow::Error),
    /// Dummy arrived, so this TCP *is* the video socket. Do not open another.
    Protocol(anyhow::Error),
}

impl std::fmt::Display for AcceptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotListening(error) | Self::Protocol(error) => write!(f, "{error:#}"),
        }
    }
}

impl std::error::Error for AcceptError {}

impl ScrcpyStream {
    /// Open the host side of `adb forward`.
    ///
    /// On this Windows adb, connecting while the server is not listening
    /// fails the *device* side immediately (`Connection refused`) and the
    /// host socket then EOFs. That is `AcceptError::NotListening`. After the
    /// dummy byte, 3.3.4 has already `accept()`'d and closed `LocalServerSocket`
    /// — a second TCP never sees dummy.
    pub async fn connect_host(local_port: u16) -> anyhow::Result<TcpStream> {
        let stream = TcpStream::connect(("127.0.0.1", local_port))
            .await
            .with_context(|| format!("connect to scrcpy on 127.0.0.1:{local_port}"))?;
        let _ = stream.set_nodelay(true);
        Ok(stream)
    }

    /// How quickly a refused connection announces itself.
    ///
    /// The documented Windows-adb refusal EOFs the host socket essentially at once, so a
    /// dummy read that fails *inside* this window is that refusal and a retry is free.
    /// Anything slower is not: with `control=true` the server has already consumed this TCP
    /// as its video socket, and the retry's fresh connection would be taken as the **control**
    /// socket. The server then closes its listener and writes the device name to a socket
    /// nobody is reading, while the retry blocks out the whole `META_DEADLINE` and dies. End
    /// state: server alive and parked, forward removed, nothing logged, zero producers —
    /// which is exactly the signature AGENTS.md 9.71 recorded.
    const REFUSAL_WINDOW: std::time::Duration = std::time::Duration::from_millis(300);

    /// One attempt at the whole handshake, including the control socket.
    ///
    /// The ordering is the protocol and it is not negotiable (two independent sources, see
    /// AGENTS.md 9.71): the server accepts **one socket per enabled channel**, video then
    /// control, and only then closes its listener. `sendDeviceMeta` runs after `open()`
    /// returns, so with control enabled the device name and video header are not sent until
    /// the *second* socket lands. Reading them before connecting socket #2 blocks forever.
    ///
    /// Measured on SM-G955F: socket #1 gets the dummy byte immediately, then **3.00 s of
    /// nothing**; connecting socket #2 to the same forwarded port releases both the 64-byte
    /// name and the 12-byte video header at once. One `adb forward` serves both.
    ///
    /// Returns the control socket alongside the stream. The caller owns it: dropping it
    /// closes the channel, and one malformed byte written to it takes the whole server down
    /// including video.
    pub async fn try_accept(local_port: u16) -> Result<(Self, TcpStream), AcceptError> {
        let stream = Self::connect_host(local_port)
            .await
            .map_err(AcceptError::NotListening)?;
        let connected_at = std::time::Instant::now();
        let mut reader = tokio::io::BufReader::new(stream);
        match tokio::time::timeout(DUMMY_DEADLINE, read_dummy(&mut reader)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                // The only failure a retry may follow. See `REFUSAL_WINDOW`.
                return Err(if connected_at.elapsed() < Self::REFUSAL_WINDOW {
                    AcceptError::NotListening(error)
                } else {
                    AcceptError::Protocol(error.context(
                        "the video socket was accepted and then failed; retrying would be \
                         taken as the control connection",
                    ))
                });
            }
            Err(_) => {
                return Err(AcceptError::NotListening(anyhow::anyhow!(
                    "dummy byte did not arrive"
                )));
            }
        }

        // Between the dummy and the meta, never before or after. This is the whole
        // ordering constraint of the 3.3.4 handshake.
        let control = Self::connect_host(local_port)
            .await
            .map_err(|error| AcceptError::Protocol(error.context("open the control socket")))?;

        match tokio::time::timeout(META_DEADLINE, read_name_and_video_header(&mut reader)).await {
            Ok(Ok(hello)) => Ok((
                Self {
                    width: hello.width,
                    height: hello.height,
                    hello,
                    reader,
                    pending_config: None,
                },
                control,
            )),
            Ok(Err(error)) => Err(AcceptError::Protocol(error)),
            Err(_) => Err(AcceptError::Protocol(anyhow::anyhow!(
                "timed out waiting for the scrcpy device name / video header; with control \
                 enabled that means the control socket never reached the server"
            ))),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// First sample the tile can decode: a keyframe or an Annex-B IDR.
    /// Exynos sometimes omits `BUFFER_FLAG_KEY_FRAME` on the first AU;
    /// publishing that as a delta leaves the worker waiting forever while
    /// `view_is_active` stays true and the keeper never retries.
    pub async fn next_sync_sample(&mut self) -> anyhow::Result<MergedSample> {
        loop {
            let sample = self.next_sample().await?;
            if sample.key || annexb_has_idr(&sample.bytes) {
                return Ok(MergedSample {
                    key: true,
                    ..sample
                });
            }
        }
    }

    /// Next publishable media sample. Config packets are held and prepended
    /// to the next sample (scrcpy's `packet_merger`).
    pub async fn next_sample(&mut self) -> anyhow::Result<MergedSample> {
        loop {
            match read_packet(&mut self.reader).await? {
                ScrcpyPacket::Media {
                    config: true,
                    payload,
                    ..
                } => {
                    self.pending_config = Some(payload);
                }
                ScrcpyPacket::Media {
                    config: false,
                    key,
                    payload,
                } => {
                    let bytes = match self.pending_config.take() {
                        Some(mut config) => {
                            config.extend_from_slice(&payload);
                            config
                        }
                        None => payload,
                    };
                    let key = key || annexb_has_idr(&bytes);
                    return Ok(MergedSample {
                        width: self.width,
                        height: self.height,
                        key,
                        bytes,
                    });
                }
            }
        }
    }
}

/// One H.264 access unit with config already merged when it was pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedSample {
    pub width: u32,
    pub height: u32,
    pub key: bool,
    pub bytes: Vec<u8>,
}

/// Read dummy byte + 64-byte name + 12-byte video header.
pub async fn read_hello<R: AsyncRead + Unpin>(reader: &mut R) -> anyhow::Result<ScrcpyHello> {
    read_dummy(reader).await?;
    read_name_and_video_header(reader).await
}

pub async fn read_dummy<R: AsyncRead + Unpin>(reader: &mut R) -> anyhow::Result<()> {
    let mut dummy = [0u8; 1];
    reader
        .read_exact(&mut dummy)
        .await
        .context("read the scrcpy dummy byte")?;
    Ok(())
}

pub async fn read_name_and_video_header<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> anyhow::Result<ScrcpyHello> {
    let mut name = [0u8; DEVICE_NAME_LEN];
    reader
        .read_exact(&mut name)
        .await
        .context("read the scrcpy device name")?;
    let mut header = [0u8; VIDEO_HEADER_LEN];
    reader
        .read_exact(&mut header)
        .await
        .context("read the scrcpy video header")?;
    let codec = u32::from_be_bytes(header[0..4].try_into().expect("4 bytes"));
    if codec != CODEC_H264 {
        anyhow::bail!(
            "scrcpy codec {:#010x} is not H.264 ({:#010x}); this path does not advertise H.265",
            codec,
            CODEC_H264
        );
    }
    let width = u32::from_be_bytes(header[4..8].try_into().expect("4 bytes"));
    let height = u32::from_be_bytes(header[8..12].try_into().expect("4 bytes"));
    if width == 0 || height == 0 {
        anyhow::bail!("scrcpy video header reported {width}x{height}");
    }
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    let device_name = String::from_utf8_lossy(&name[..end]).into_owned();
    Ok(ScrcpyHello {
        device_name,
        codec,
        width,
        height,
    })
}

/// List PIDs of *our* 3.3.4 server.
///
/// The encoder argv is `app_process / com.genymobile.scrcpy.Server 3.3.4 …`.
/// `CLASSPATH` is environ, so a cmdline grep for the JAR path only hits the
/// `sh -c` wrapper and leaves the OMX holder alive. GenFarmer is
/// `Server 2.4` + `genscrcpy.jar` — this script must not match it.
/// PIDs of our leftover servers, in one pass instead of one process per PID.
///
/// **This was 95 % of the cost of starting a view.** The previous form looped over
/// `/proc/[0-9]*/cmdline` and forked *two* `grep`s inside the loop — on a Galaxy S8 with 648
/// processes that is ~1300 process spawns through one `sh`, and it measured **5.5 s idle and
/// 21 s** with twenty phones starting at once. Since every preset switch stops the old
/// producer and starts a new one, that is what the operator waited, staring at a frozen
/// picture, after double-clicking a phone: **17.8 s of no frames at all**, of which this was
/// nearly all. One sweeping `grep -al` over the same files costs **230 ms**.
///
/// The second `grep` stays, but now runs only for the handful of files the first one
/// matched rather than for all 648.
///
/// **The `/proc/` exclusion is not cosmetic.** This script's own text contains the pattern it
/// searches for, so the transient shell running it matches itself — the old form did too, and
/// that self-match is why `stop_our_scrcpy_leftovers` never took its "nothing to do" early
/// return: it always found at least one PID, always slept, and always listed a second time.
/// A real server's command line never contains `/proc/`; this script's always does. Anything
/// mentioning it is the sweep looking at its own reflection.
///
/// Still version-pinned to 3.3.4, which is what keeps GenFarmer's 2.4 servers out of it.
pub const LEFTOVER_LIST_SCRIPT: &str = "\
for f in $(grep -al com.genymobile.scrcpy.Server /proc/[0-9]*/cmdline 2>/dev/null); do \
grep -aq 3.3.4 \"$f\" 2>/dev/null || continue; \
grep -aq /proc/ \"$f\" 2>/dev/null && continue; \
pid=${f#/proc/}; echo ${pid%/cmdline}; \
done";

/// PIDs of our 3.3.4 server (encoder or `sh -c` wrapper). Never GenFarmer.
pub fn pids_running_our_server(proc_listing: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    for line in proc_listing.lines() {
        if line.contains("genscrcpy") {
            continue;
        }
        if !line.contains("scrcpy.Server") || !line.contains(SERVER_VERSION) {
            continue;
        }
        for token in line.split(|c: char| !c.is_ascii_digit()) {
            if let Ok(pid) = token.parse::<u32>() {
                if pid > 0 && !pids.contains(&pid) {
                    pids.push(pid);
                }
                break;
            }
        }
    }
    pids
}

/// Annex-B NAL type 5 (IDR). Used when the encoder omits the key flag.
pub fn annexb_has_idr(bytes: &[u8]) -> bool {
    annexb_has_nal(bytes, 5)
}

pub fn annexb_has_sps(bytes: &[u8]) -> bool {
    annexb_has_nal(bytes, 7)
}

fn annexb_has_nal(bytes: &[u8], nal_type: u8) -> bool {
    let mut i = 0;
    while i + 4 < bytes.len() {
        let start = if bytes[i] == 0 && bytes[i + 1] == 0 && bytes[i + 2] == 0 && bytes[i + 3] == 1
        {
            i + 4
        } else if bytes[i] == 0 && bytes[i + 1] == 0 && bytes[i + 2] == 1 {
            i + 3
        } else {
            i += 1;
            continue;
        };
        if start < bytes.len() && bytes[start] & 0x1f == nal_type {
            return true;
        }
        i = start;
    }
    false
}

/// Read one 12-byte header and its payload.
pub async fn read_packet<R: AsyncRead + Unpin>(reader: &mut R) -> anyhow::Result<ScrcpyPacket> {
    let mut header = [0u8; HEADER_LEN];
    reader
        .read_exact(&mut header)
        .await
        .context("read a scrcpy packet header")?;
    parse_header(&header, reader).await
}

async fn parse_header<R: AsyncRead + Unpin>(
    header: &[u8; HEADER_LEN],
    reader: &mut R,
) -> anyhow::Result<ScrcpyPacket> {
    let pts_and_flags = u64::from_be_bytes(header[0..8].try_into().expect("8 bytes"));
    let size = u32::from_be_bytes(header[8..12].try_into().expect("4 bytes"));
    if size > MAX_PACKET_BYTES {
        anyhow::bail!("scrcpy packet {size} bytes exceeds the {MAX_PACKET_BYTES} cap");
    }
    let mut payload = vec![0u8; size as usize];
    if size > 0 {
        reader
            .read_exact(&mut payload)
            .await
            .context("read a scrcpy media payload")?;
    }
    Ok(ScrcpyPacket::Media {
        config: pts_and_flags & PACKET_FLAG_CONFIG != 0,
        key: pts_and_flags & PACKET_FLAG_KEY_FRAME != 0,
        payload,
    })
}

/// Parse a header that already has its payload attached (tests / merger).
pub fn parse_header_only(header: &[u8; HEADER_LEN]) -> anyhow::Result<HeaderKind> {
    let pts_and_flags = u64::from_be_bytes(header[0..8].try_into().expect("8 bytes"));
    let size = u32::from_be_bytes(header[8..12].try_into().expect("4 bytes"));
    Ok(HeaderKind::Media {
        config: pts_and_flags & PACKET_FLAG_CONFIG != 0,
        key: pts_and_flags & PACKET_FLAG_KEY_FRAME != 0,
        size,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderKind {
    Media { config: bool, key: bool, size: u32 },
}

/// Merge a held config payload onto the next non-config sample.
pub fn merge_config(pending: &mut Option<Vec<u8>>, payload: Vec<u8>) -> Vec<u8> {
    match pending.take() {
        Some(mut config) => {
            config.extend_from_slice(&payload);
            config
        }
        None => payload,
    }
}

pub fn encode_media(config: bool, key: bool, payload: &[u8]) -> Vec<u8> {
    let mut pts_and_flags = 0u64;
    if config {
        pts_and_flags |= PACKET_FLAG_CONFIG;
    } else if key {
        pts_and_flags |= PACKET_FLAG_KEY_FRAME;
    }
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(&pts_and_flags.to_be_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn encode_hello(device_name: &str, width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + DEVICE_NAME_LEN + VIDEO_HEADER_LEN);
    out.push(0);
    let mut name = [0u8; DEVICE_NAME_LEN];
    let raw = device_name.as_bytes();
    let n = raw.len().min(DEVICE_NAME_LEN - 1);
    name[..n].copy_from_slice(&raw[..n]);
    out.extend_from_slice(&name);
    out.extend_from_slice(&CODEC_H264.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn launch_command_pins_version_h264_and_the_named_jar() {
        let command = launch_command(
            0x00ab_12cd,
            ViewPreset::Tile.tuned(riviu_core::StreamQuality::Medium, riviu_core::STREAM_FPS),
        );
        assert!(command.contains(REMOTE_SERVER), "{command}");
        assert!(command.contains(" 3.3.4 "), "{command}");
        assert!(!command.contains(" 4.1 "), "{command}");
        assert!(command.contains("video_codec=h264"), "{command}");
        assert!(command.contains("scid=00ab12cd"), "{command}");
        assert!(command.contains("tunnel_forward=true"), "{command}");
        assert!(command.contains("audio=false"), "{command}");
        assert!(command.contains("control=true"), "{command}");
        // Exactly once. `control` defaults to TRUE in Options.<init>, so a refactor that
        // drops the flag does not disable the control socket -- it enables it on a host
        // that has no second socket to offer, which parks the server in accept() forever
        // and looks exactly like a phone that stopped sending.
        assert_eq!(command.matches("control=").count(), 1, "{command}");
        assert!(command.contains("max_size=480"), "{command}");
        // A tile is capped below the operator's declared rate, on purpose: twenty of these
        // decode in the same WebView that has to keep the overlay smooth, and the fleet
        // measured 135 % of a core at 24 against 85 % at 5. The setting still reaches the
        // overlay untouched -- see `overlay_preset_is_the_larger_encode_not_a_second_process`.
        assert!(riviu_core::STREAM_FPS > ViewPreset::Tile.max_fps());
        assert!(
            command.contains(&format!("max_fps={}", ViewPreset::Tile.max_fps())),
            "{command}"
        );
        // The cap is a ceiling, not a fixed rate: an operator who asks for less than the
        // ceiling gets what they asked for, on both presets.
        assert_eq!(
            ViewPreset::Tile
                .tuned(riviu_core::StreamQuality::Medium, MIN_VIEW_FPS)
                .max_fps,
            MIN_VIEW_FPS
        );
        assert!(command.contains("video_bit_rate=1200000"), "{command}");
        assert!(
            command.contains("video_codec_options=i-frame-interval:int=1"),
            "{command}"
        );
        assert!(
            !command.contains("i-frame-interval:int:2"),
            "3.3.4 CodecOption wants key[:type]=value; a third colon exits with '=' expected"
        );
        assert!(
            !command.contains("ignore_video_encoder_constraints"),
            "{command}"
        );
        assert!(!command.contains("hevc"), "{command}");
        assert!(!command.contains("av1"), "{command}");
    }

    #[test]
    fn no_tuning_can_push_the_server_argv_past_what_app_process_survives() {
        // The guard that would have saved twenty phones. Past 254 bytes of argv, Android 9
        // `app_process` on this fleet dies with `stack corruption detected
        // (-fstack-protector)` -- and it dies AFTER answering the handshake, so the host
        // reads a healthy hello and then simply never receives a frame. That is the "6
        // minutes, 0 producers, not one warning" of AGENTS.md 9.71.
        //
        // Swept over every preset and quality rather than the default tile, because the
        // budget is spent by the numbers: `max_size`, `max_fps` and `video_bit_rate` all
        // grow with quality, and Extra on Overlay is the longest line this can emit.
        let mut worst = 0usize;
        let mut worst_command = String::new();
        for preset in [ViewPreset::Tile, ViewPreset::Overlay] {
            for quality in [
                riviu_core::StreamQuality::Low,
                riviu_core::StreamQuality::Medium,
                riviu_core::StreamQuality::High,
                riviu_core::StreamQuality::Extra,
            ] {
                for fps in [MIN_VIEW_FPS, riviu_core::STREAM_FPS, 30] {
                    // 0xffffffff is the widest scid the formatter can produce.
                    let command = launch_command(u32::MAX, preset.tuned(quality.clone(), fps));
                    let argv = server_argv(&command).len();
                    if argv > worst {
                        worst = argv;
                        worst_command = command;
                    }
                }
            }
        }
        assert!(
            worst <= MAX_SERVER_ARGV,
            "the longest launch is {worst} bytes of argv, over the measured {MAX_SERVER_ARGV} \
             this fleet survives; app_process will abort AFTER the handshake and the tile \
             will look like a phone that simply stopped sending: {worst_command}"
        );
    }

    #[test]
    fn only_argv_is_counted_against_the_budget_not_the_classpath() {
        // Measured: sixty extra bytes added to the shell line as an environment assignment
        // stream perfectly, twelve added to argv abort. So `CLASSPATH=` -- 45 of the
        // command's bytes -- is free, and anyone hunting for room must not go looking there.
        let command = launch_command(
            0x00ab_12cd,
            ViewPreset::Tile.tuned(riviu_core::StreamQuality::Medium, riviu_core::STREAM_FPS),
        );
        assert!(command.starts_with("CLASSPATH="));
        assert!(server_argv(&command).starts_with("app_process "));
        assert!(
            command.len() - server_argv(&command).len() > 40,
            "the CLASSPATH assignment is a large part of the command and none of it counts"
        );
    }

    #[test]
    fn turning_clipboard_autosync_off_would_not_fit_which_is_why_it_is_left_on() {
        // The control socket itself fits with room to spare -- it is one byte CHEAPER than
        // `control=false`. What does not fit is the option everyone assumes goes with it.
        //
        // `clipboard_autosync` defaults to TRUE, so with control enabled the phone pushes a
        // message every time its clipboard changes. Turning that off costs 25 bytes against
        // roughly 14 spare, which is what took twenty phones down when all three of
        // `control=true clipboard_autosync=false power_on=false` were added at once
        // (AGENTS.md 9.71, 9.74).
        //
        // So it stays on and the host drains the socket instead. Measured, 75 s soak on
        // SM-G955F with the control socket deliberately never read and the clipboard changed
        // twelve times: 2.2 MB of video, twelve keyframe requests all honoured, server alive.
        let command = launch_command(
            u32::MAX,
            ViewPreset::Overlay.tuned(riviu_core::StreamQuality::Extra, 30),
        );
        assert!(
            server_argv(&command).len() <= MAX_SERVER_ARGV,
            "the control socket as shipped must fit"
        );
        assert!(
            server_argv(&command).len() + " clipboard_autosync=false".len() > MAX_SERVER_ARGV,
            "if this ever fits, the drain stops being a requirement and this comment is stale"
        );
    }

    #[test]
    fn the_overlay_encode_covers_its_default_width_and_bounds_the_rest() {
        // `max_size` caps the LONG edge, so on a 1080x2400 phone the encoded width is
        // `max_size * 1080/2400`. The overlay shows 400 px by default and up to 760 at full
        // zoom (`FOCUS_ZOOM` in zoom.ts), and anything narrower than the displayed width is
        // upscaled -- which is what the operator reported as a broken picture.
        //
        // This test used to demand the encode cover the *maximum* zoom. It cannot, and
        // saying so is the point: 760 px wide on this aspect needs a long edge of ~1689,
        // which is over 4x the level-3.0 macroblock budget the codec ladder is built on.
        // Removing the residual upscale means putting a level-4.0 candidate in front of
        // that ladder, not raising this constant -- an earlier commit raised it to 1600 and
        // the only reason no phone went black is that the call site did not exist yet.
        const FOCUS_DEFAULT_WIDTH: u32 = 400;
        const FOCUS_MAX_DISPLAY_WIDTH: u32 = 760;
        let encoded_width = ViewPreset::Overlay.max_size() * 1080 / 2400;
        assert!(
            encoded_width >= FOCUS_DEFAULT_WIDTH * 9 / 10,
            "overlay encodes {encoded_width}px wide, under its {FOCUS_DEFAULT_WIDTH}px \
             default -- the default view must not be an upscale"
        );
        // Bounded and named rather than left to be rediscovered. Measured on hardware at
        // 832: 376x832, which reads sharply enough to make out individual notification
        // text; the 600 this replaced gave 270 px and a 2.81x stretch.
        let worst_upscale_x100 = FOCUS_MAX_DISPLAY_WIDTH * 100 / encoded_width;
        assert!(
            worst_upscale_x100 <= 210,
            "full zoom upscales {}.{:02}x, worse than the 2.10x this cap allows",
            worst_upscale_x100 / 100,
            worst_upscale_x100 % 100
        );
        assert!(
            ViewPreset::Overlay.max_size() > ViewPreset::Tile.max_size(),
            "the overlay must ask for more than the tile encode or it has no reason to exist"
        );
    }

    #[test]
    fn low_quality_lowers_the_bitrate_and_never_the_frame_size() {
        // The load-bearing safety property. 176 fails MediaCodec.configure on the Redmi
        // and 320 gives the Note 8 a Baseline L1.3 SPS WebView2 can refuse, so a
        // quality control that shrank the frame would let an operator turn a working
        // tile into a dead one from a dropdown.
        for preset in [ViewPreset::Tile, ViewPreset::Overlay] {
            let floor = preset.max_size();
            for quality in [
                riviu_core::StreamQuality::Low,
                riviu_core::StreamQuality::Medium,
                riviu_core::StreamQuality::High,
                riviu_core::StreamQuality::Extra,
            ] {
                let tuned = preset.tuned(quality.clone(), 24);
                assert!(
                    tuned.max_size >= floor,
                    "{preset:?} at {quality:?} asked for {}, below the measured floor {floor}",
                    tuned.max_size
                );
            }
        }
        let low = ViewPreset::Tile.tuned(riviu_core::StreamQuality::Low, 24);
        let medium = ViewPreset::Tile.tuned(riviu_core::StreamQuality::Medium, 24);
        assert_eq!(low.max_size, medium.max_size);
        assert!(low.bit_rate < medium.bit_rate);
    }

    #[test]
    fn quality_rises_monotonically_so_a_higher_setting_is_never_worse() {
        let sizes: Vec<u32> = [
            riviu_core::StreamQuality::Low,
            riviu_core::StreamQuality::Medium,
            riviu_core::StreamQuality::High,
            riviu_core::StreamQuality::Extra,
        ]
        .iter()
        .map(|q| ViewPreset::Tile.tuned(q.clone(), 24).max_size)
        .collect();
        assert!(sizes.windows(2).all(|w| w[0] <= w[1]), "{sizes:?}");

        // Bitrate too, on BOTH presets. Checking only the size is what let an absolute
        // mapping ship High at 2 Mbps against Medium's 4 — higher and visibly worse.
        for preset in [ViewPreset::Tile, ViewPreset::Overlay] {
            let rates: Vec<u32> = [
                riviu_core::StreamQuality::Low,
                riviu_core::StreamQuality::Medium,
                riviu_core::StreamQuality::High,
                riviu_core::StreamQuality::Extra,
            ]
            .iter()
            .map(|q| preset.tuned(q.clone(), 24).bit_rate)
            .collect();
            assert!(
                rates.windows(2).all(|w| w[0] <= w[1]),
                "{preset:?} {rates:?}"
            );
        }
    }

    #[test]
    fn a_touch_message_is_thirty_two_bytes_in_the_order_the_server_reads_them() {
        // Pinned byte by byte because `ControlMessageReader` has no framing: it takes the
        // type byte and then reads exactly as many bytes as that type implies. Get the
        // length wrong and the next message starts mid-field forever; get it wrong enough
        // and the server raises ControlProtocolException and quits the looper, which takes
        // the video down on that phone. A wrong *value* is a bad touch; a wrong *length* is
        // a black screen.
        let down = inject_touch(TouchAction::Down, 0x0011_2233, -2, 832, 1560);
        assert_eq!(down.len(), 32);
        assert_eq!(down[0], CONTROL_MESSAGE_INJECT_TOUCH);
        assert_eq!(down[1], 0);
        assert_eq!(&down[2..10], &0u64.to_be_bytes(), "a finger, not the mouse");
        assert_eq!(&down[10..14], &0x0011_2233i32.to_be_bytes());
        assert_eq!(&down[14..18], &(-2i32).to_be_bytes(), "y is signed");
        assert_eq!(&down[18..20], &832u16.to_be_bytes());
        assert_eq!(&down[20..22], &1560u16.to_be_bytes());
        assert_eq!(&down[22..24], &[0xff, 0xff], "Q0.16: 0xFFFF is exactly 1.0");
        assert_eq!(
            &down[24..28],
            &0u32.to_be_bytes(),
            "a finger presses no button"
        );
        assert_eq!(
            &down[28..32],
            &1u32.to_be_bytes(),
            "BUTTON_PRIMARY while held"
        );
    }

    #[test]
    fn only_the_release_reports_no_pressure_and_no_button() {
        // A finger that lifts while still reporting pressure is a finger the framework
        // thinks is stuck: the next DOWN then arrives as part of the same gesture and the
        // phone behaves as if the operator never let go.
        let held = [TouchAction::Down, TouchAction::Move];
        for action in held {
            let bytes = inject_touch(action, 10, 10, 480, 1000);
            assert_eq!(&bytes[22..24], &u16::MAX.to_be_bytes(), "{action:?}");
            assert_eq!(&bytes[28..32], &1u32.to_be_bytes(), "{action:?}");
        }
        let up = inject_touch(TouchAction::Up, 10, 10, 480, 1000);
        assert_eq!(&up[22..24], &0u16.to_be_bytes());
        assert_eq!(&up[28..32], &0u32.to_be_bytes());

        // The action codes are AMOTION_EVENT_ACTION_*, passed to MotionEvent.obtain
        // unmapped, so they are not ours to renumber.
        assert_eq!(TouchAction::Down.code(), 0);
        assert_eq!(TouchAction::Up.code(), 1);
        assert_eq!(TouchAction::Move.code(), 2);
        assert_eq!(TouchAction::parse("move").unwrap(), TouchAction::Move);
        assert!(TouchAction::parse("hover").is_err());
    }

    #[test]
    fn a_touch_can_never_be_confused_with_the_keyframe_request() {
        // The two messages share one socket and one lock. They are different lengths, so the
        // only thing keeping them apart is the type byte -- if these ever collide, every
        // touch becomes a video reset and the stream stutters on every drag.
        assert_ne!(CONTROL_MESSAGE_INJECT_TOUCH, reset_video()[0]);
    }

    #[test]
    fn the_settings_hint_names_the_same_tile_ceiling_this_file_enforces() {
        // The operator reads a number in the settings panel; the encoder obeys the one
        // here. Nothing but this test connects them, and a mismatch is silent -- the panel
        // would simply explain a cap that is not the cap.
        let panel = include_str!("../../../apps/desktop/src/components/SettingsPanel.tsx");
        let declared = panel
            .lines()
            .find_map(|line| line.trim().strip_prefix("const TILE_FPS_CEILING = "))
            .and_then(|rest| rest.trim_end_matches(';').parse::<u32>().ok())
            .expect("SettingsPanel.tsx declares TILE_FPS_CEILING");
        assert_eq!(
            declared,
            ViewPreset::Tile.max_fps(),
            "the settings panel promises {declared} fps for tiles, the launch asks for {}",
            ViewPreset::Tile.max_fps()
        );
    }

    #[test]
    fn an_absurd_frame_rate_is_clamped_into_what_the_fleet_has_run_at() {
        // 0 would ask scrcpy for an unbounded rate; 240 asks for one no phone here
        // delivers. Both are reachable from a settings row, so both are clamped.
        let fast = ViewPreset::Tile.tuned(riviu_core::StreamQuality::Medium, 240);
        let stopped = ViewPreset::Tile.tuned(riviu_core::StreamQuality::Medium, 0);
        assert_eq!(fast.max_fps, ViewPreset::Tile.max_fps());
        assert_eq!(stopped.max_fps, MIN_VIEW_FPS);
    }

    #[test]
    fn overlay_preset_is_the_larger_encode_not_a_second_process() {
        let command = launch_command(
            1,
            ViewPreset::Overlay.tuned(riviu_core::StreamQuality::Medium, riviu_core::STREAM_FPS),
        );
        assert!(
            command.contains(&format!("max_size={MAX_LONG_EDGE}")),
            "{command}"
        );
        assert!(
            command.contains(&format!("max_fps={}", riviu_core::STREAM_FPS)),
            "{command}"
        );
        assert!(command.contains("video_bit_rate=4000000"), "{command}");
        assert_eq!(ViewPreset::parse("overlay").unwrap(), ViewPreset::Overlay);
        assert!(ViewPreset::parse("hevc").is_err());
    }

    #[test]
    fn the_two_presets_can_be_tuned_apart() {
        // `focus_quality` had no reader for as long as it existed: the driver held ONE
        // quality for both presets, so the overlay silently encoded at the grid's setting
        // and moving the overlay control did nothing at all. The two are different pictures
        // — one phone filling a window against one of twenty tiles — so they have to be able
        // to differ, and this is what says so.
        let tile_low = ViewPreset::Tile.tuned(riviu_core::StreamQuality::Low, 24);
        let overlay_extra = ViewPreset::Overlay.tuned(riviu_core::StreamQuality::Extra, 24);
        assert!(
            overlay_extra.bit_rate > tile_low.bit_rate * 4,
            "a low grid and an extra overlay must be far apart: {tile_low:?} vs {overlay_extra:?}"
        );
        // And the same quality on both still differs, because the presets themselves do.
        let tile = ViewPreset::Tile.tuned(riviu_core::StreamQuality::Medium, 24);
        let overlay = ViewPreset::Overlay.tuned(riviu_core::StreamQuality::Medium, 24);
        assert!(overlay.max_size > tile.max_size);
    }

    #[test]
    fn socket_name_matches_the_server_format() {
        assert_eq!(socket_name(0x00ab_12cd), "scrcpy_00ab12cd");
        assert_eq!(socket_name(1), "scrcpy_00000001");
    }

    #[test]
    fn forward_prefix_matches_every_socket_name() {
        // A prune keyed on this prefix goes silently dead if `socket_name` is ever
        // reworded, so assert the relationship instead of trusting two literals to agree.
        for scid in [1u32, 0x62e3_7875, 0x7fff_ffff] {
            assert!(
                format!("localabstract:{}", socket_name(scid)).starts_with(FORWARD_PREFIX),
                "socket_name({scid:#x}) escaped FORWARD_PREFIX"
            );
        }
    }

    #[test]
    fn no_preset_at_any_quality_leaves_level_3_0() {
        // The property that keeps the picture from going black rather than merely soft: a
        // stream whose SPS declares a level the decoder was configured for, and then
        // exceeds, is refused asynchronously -- which surfaces as a black canvas.
        //
        // Checked against real aspect ratios, squarest first, because the budget is on
        // area. 16:9 is the binding case and the reason the cap is 832 and not the 900 an
        // earlier plan of mine asserted was fleet-safe.
        let aspects = [
            ("16:9", 9u32, 16u32),
            ("18:9", 9, 18),
            ("18.5:9 Note 8", 1440, 2960),
            ("19.5:9 Redmi", 1080, 2400),
        ];
        for preset in [ViewPreset::Tile, ViewPreset::Overlay] {
            for quality in [
                riviu_core::StreamQuality::Low,
                riviu_core::StreamQuality::Medium,
                riviu_core::StreamQuality::High,
                riviu_core::StreamQuality::Extra,
            ] {
                let long_edge = preset.tuned(quality.clone(), 24).max_size;
                for (name, short, long) in aspects {
                    let short_edge = long_edge * short / long;
                    let blocks = short_edge.div_ceil(16) * long_edge.div_ceil(16);
                    assert!(
                        blocks <= LEVEL_3_0_MAX_MACROBLOCKS,
                        "{preset:?}/{quality:?} on {name}: {short_edge}x{long_edge} is                          {blocks} macroblocks, over {LEVEL_3_0_MAX_MACROBLOCKS}"
                    );
                }
                assert!(
                    long_edge >= ViewPreset::Tile.max_size(),
                    "{preset:?}/{quality:?} dropped below the measured encoder floor"
                );
            }
        }
        assert_eq!(SQUAREST_SUPPORTED_ASPECT, (9, 16));
    }

    #[tokio::test]
    async fn hello_reads_dummy_name_size_and_refuses_a_non_h264_codec() {
        let mut good = Cursor::new(encode_hello("Pixel", 144, 320));
        let hello = read_hello(&mut good).await.expect("hello");
        assert_eq!(hello.device_name, "Pixel");
        assert_eq!(hello.codec, CODEC_H264);
        assert_eq!((hello.width, hello.height), (144, 320));

        let mut bad = encode_hello("Pixel", 144, 320);
        bad[1 + DEVICE_NAME_LEN..1 + DEVICE_NAME_LEN + 4]
            .copy_from_slice(&0x6832_6536u32.to_be_bytes()); // 'h2e6'
        let err = read_hello(&mut Cursor::new(bad)).await.expect_err("hevc");
        assert!(err.to_string().contains("H.264"), "{err}");
    }

    #[tokio::test]
    async fn a_4_1_session_header_is_a_3_3_4_config_packet_not_a_size() {
        // 4.1 used bit 63 for session + width/height in the same 12 bytes.
        // Feeding that into this parser must not invent a size and drop the
        // Annex-B payload that follows.
        let mut bytes = encode_hello("Note8", 152, 320);
        bytes.extend_from_slice(&encode_media(true, false, b"SPS-PPS"));
        bytes.extend_from_slice(&encode_media(false, true, b"IDR"));

        let mut reader = Cursor::new(bytes);
        let hello = read_hello(&mut reader).await.unwrap();
        assert_eq!((hello.width, hello.height), (152, 320));

        let config = read_packet(&mut reader).await.unwrap();
        let mut pending = match config {
            ScrcpyPacket::Media {
                config: true,
                payload,
                ..
            } => Some(payload),
            other => panic!("expected config, got {other:?}"),
        };
        let key = read_packet(&mut reader).await.unwrap();
        let ScrcpyPacket::Media {
            config: false,
            key: true,
            payload,
        } = key
        else {
            panic!("expected keyframe");
        };
        assert_eq!(merge_config(&mut pending, payload), b"SPS-PPSIDR");
    }

    #[tokio::test]
    async fn hello_can_arrive_in_two_stages_dummy_then_meta() {
        let bytes = encode_hello("Redmi", 144, 320);
        let mut reader = Cursor::new(bytes);
        read_dummy(&mut reader).await.expect("dummy");
        let hello = read_name_and_video_header(&mut reader).await.expect("meta");
        assert_eq!(hello.device_name, "Redmi");
        assert_eq!(hello.codec, CODEC_H264);
        assert_eq!((hello.width, hello.height), (144, 320));
    }

    #[test]
    fn pids_running_our_server_match_3_3_4_and_skip_genfarmer() {
        let listing = "\
shell  9448  1 app_process / com.genymobile.scrcpy.Server 2.4 scid=00000032\n\
shell  20663 1 app_process / com.genymobile.scrcpy.Server 3.3.4 scid=4afcb05e\n\
shell  20661 1 sh -c CLASSPATH=/data/local/tmp/riviu-scrcpy-server app_process / com.genymobile.scrcpy.Server 3.3.4\n\
shell  29325 1 app_process / com.genymobile.scrcpy.Server 2.4 genscrcpy.jar\n\
shell  91    1 grep riviu-scrcpy-server /proc/1/cmdline\n";
        assert_eq!(pids_running_our_server(listing), vec![20663, 20661]);
        assert!(LEFTOVER_LIST_SCRIPT.contains("3.3.4"));
        assert!(LEFTOVER_LIST_SCRIPT.contains("scrcpy.Server"));
        assert!(!LEFTOVER_LIST_SCRIPT.contains("genscrcpy"));
        assert!(!LEFTOVER_LIST_SCRIPT.contains("riviu-scrcpy-server"));

        // One sweeping grep over every cmdline, not one grep per cmdline. Measured on a
        // 648-process Galaxy S8: 230 ms against 5.5 s idle, and 21 s with the fleet starting
        // at once -- which was 95 % of what the operator waited when they opened an overlay.
        // Losing this is not a small regression, so it is pinned by shape.
        assert!(
            LEFTOVER_LIST_SCRIPT.contains("grep -al"),
            "the sweep must match every cmdline in one pass"
        );
        assert!(
            !LEFTOVER_LIST_SCRIPT.contains("for f in /proc/"),
            "iterating /proc and grepping per file is the slow form that was removed"
        );
        // The script's own text contains the pattern it hunts, so without this the transient
        // shell matches itself, `stop_our_scrcpy_leftovers` never takes its early return, and
        // every spawn pays a sleep and a second listing for a process that is already gone.
        assert!(
            LEFTOVER_LIST_SCRIPT.contains("grep -aq /proc/"),
            "the sweep must exclude its own reflection"
        );
    }

    #[test]
    fn annexb_finds_idr_and_sps() {
        let idr = [0, 0, 0, 1, 0x65, 0x88, 0, 0, 1, 0x67, 0x42, 0xe0, 0x1e];
        assert!(annexb_has_idr(&idr));
        assert!(annexb_has_sps(&idr));
        assert!(!annexb_has_idr(&[0, 0, 0, 1, 0x61, 0x00]));
    }

    #[tokio::test]
    async fn a_delta_after_config_is_still_a_sync_sample_when_it_carries_idr() {
        let mut bytes = encode_hello("Note8", 152, 320);
        bytes.extend_from_slice(&encode_media(
            true,
            false,
            &[0, 0, 0, 1, 0x67, 0x42, 0xe0, 0x1e],
        ));
        bytes.extend_from_slice(&encode_media(false, false, &[0, 0, 0, 1, 0x65, 0x88]));
        let mut reader = Cursor::new(bytes);
        let _ = read_hello(&mut reader).await.unwrap();
        let mut pending = None;
        loop {
            match read_packet(&mut reader).await.unwrap() {
                ScrcpyPacket::Media {
                    config: true,
                    payload,
                    ..
                } => pending = Some(payload),
                ScrcpyPacket::Media {
                    config: false,
                    key,
                    payload,
                } => {
                    let merged = merge_config(&mut pending, payload);
                    assert!(!key, "Exynos omitted the key flag");
                    assert!(annexb_has_idr(&merged));
                    break;
                }
            }
        }
    }

    #[test]
    fn config_is_bit_63_and_key_is_bit_62() {
        let media = encode_media(true, false, b"x");
        let header: [u8; 12] = media[..12].try_into().unwrap();
        assert_eq!(
            parse_header_only(&header).unwrap(),
            HeaderKind::Media {
                config: true,
                key: false,
                size: 1
            }
        );
        let key = encode_media(false, true, b"x");
        let header: [u8; 12] = key[..12].try_into().unwrap();
        assert_eq!(
            parse_header_only(&header).unwrap(),
            HeaderKind::Media {
                config: false,
                key: true,
                size: 1
            }
        );
        assert_eq!(header[0], 0x40, "key must be bit 62, not the 4.1 bit 61");
    }
}

#[cfg(test)]
mod handshake_tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    /// A fake server with 3.3.4's exact ordering: accept the video socket and write the
    /// dummy, then **block until a second socket lands** before sending the name and video
    /// header. That is `DesktopConnection.open()` returning only once every enabled channel
    /// has been accepted, with `sendDeviceMeta` after it.
    ///
    /// This shape is the whole reason the handshake is written the way it is, and until now
    /// nothing exercised it: `try_accept` needed a live socket, so it had no test at all —
    /// while being the function whose ordering, got wrong, produced twenty phones with a
    /// perfect hello and no video.
    async fn fake_server(hold_sockets: bool) -> (u16, tokio::task::JoinHandle<bool>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let handle = tokio::spawn(async move {
            let (mut video, _) = listener.accept().await.expect("video socket");
            video.write_all(&[0u8]).await.expect("dummy");
            video.flush().await.expect("flush dummy");
            // Nothing else until the control socket arrives.
            let control = listener.accept().await.expect("control socket");
            let hello = encode_hello("SM-G955F", 232, 480);
            // `encode_hello` includes the dummy, which has already gone.
            video.write_all(&hello[1..]).await.expect("meta");
            video.flush().await.expect("flush meta");
            if hold_sockets {
                // Keep both alive so the caller can write to the control socket.
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            drop(control);
            true
        });
        (port, handle)
    }

    #[tokio::test]
    async fn the_control_socket_is_opened_between_the_dummy_and_the_device_name() {
        // If `try_accept` read the name before connecting the control socket it would block
        // here forever, because the server has not sent it and cannot until socket #2 lands.
        let (port, server) = fake_server(true).await;

        let accepted = tokio::time::timeout(Duration::from_secs(5), ScrcpyStream::try_accept(port))
            .await
            .expect("the handshake must not block waiting for meta that needs socket #2")
            .expect("handshake succeeds");

        let (stream, _control) = accepted;
        assert_eq!(stream.hello.device_name, "SM-G955F");
        assert_eq!(stream.size(), (232, 480));
        server.abort();
    }

    #[tokio::test]
    async fn the_returned_control_socket_carries_a_keyframe_request() {
        // One byte, type 17, and nothing else on the wire — the device's reader has no
        // framing, so anything extra desynchronises it permanently and takes the video down
        // with it.
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = tokio::spawn(async move {
            let (mut video, _) = listener.accept().await.expect("video socket");
            video.write_all(&[0u8]).await.expect("dummy");
            video.flush().await.expect("flush");
            let (mut control, _) = listener.accept().await.expect("control socket");
            let hello = encode_hello("SM-G955F", 232, 480);
            video.write_all(&hello[1..]).await.expect("meta");
            video.flush().await.expect("flush");
            let mut received = Vec::new();
            let mut buf = [0u8; 8];
            // Read exactly what the host sends, then report it.
            if let Ok(read) = control.read(&mut buf).await {
                received.extend_from_slice(&buf[..read]);
            }
            received
        });

        let (_stream, control) = ScrcpyStream::try_accept(port).await.expect("handshake");
        let (_read, mut write) = control.into_split();
        write.write_all(&reset_video()).await.expect("send reset");
        write.flush().await.expect("flush reset");

        let received = tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("server responded")
            .expect("join");
        assert_eq!(
            received,
            vec![17],
            "RESET_VIDEO is one byte with no payload"
        );
    }

    #[tokio::test]
    async fn a_socket_accepted_and_then_dropped_is_a_protocol_error_not_a_retry() {
        // The hole that made the first attempt at this unfixable. A retry is only free while
        // the server has accepted nothing; once it has taken this TCP as the video channel,
        // the retry's fresh connection is consumed as the CONTROL channel, the server closes
        // its listener and writes the device name to the socket nobody is reading — and the
        // retry then blocks out the whole META_DEADLINE. Server alive and parked, forward
        // removed, nothing logged: the exact signature of AGENTS.md 9.71.
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            let (video, _) = listener.accept().await.expect("video socket");
            // Accepted, then closed without ever writing the dummy, and slowly enough that
            // it cannot be the immediate Windows-adb refusal.
            tokio::time::sleep(ScrcpyStream::REFUSAL_WINDOW * 3).await;
            drop(video);
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        match ScrcpyStream::try_accept(port).await {
            Err(AcceptError::Protocol(_)) => {}
            Err(AcceptError::NotListening(error)) => panic!(
                "a slow failure must not be retried; retrying eats the control accept: {error:#}"
            ),
            Ok(_) => panic!("the server never sent a dummy byte"),
        }
    }
}
