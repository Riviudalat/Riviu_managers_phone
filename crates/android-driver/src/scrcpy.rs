//! scrcpy-server 3.3.4 as the Android **view** encoder.
//!
//! Not a [`riviu_core::FrameSource`]. H.264 never enters `StreamHub`. The
//! protocol here is the official 3.3.4 socket: dummy byte, 64-byte name,
//! 12-byte video header (`codec` + width + height), then 12-byte media
//! packets. 3.3.4 has no session packets; config is bit 63 and key is bit
//! 62. Control and audio stay off; tap and swipe still go through the
//! uiautomator session / overlay lease.
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
            Self::Overlay => 1600,
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

    /// The highest frame rate measured to work on this fleet.
    pub fn max_fps(self) -> u32 {
        match self {
            Self::Tile => 30,
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
            max_size: size,
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
pub fn launch_command(scid: u32, tuning: ViewTuning) -> String {
    format!(
        "CLASSPATH={REMOTE_SERVER} app_process / {MAIN_CLASS} {SERVER_VERSION} \
         scid={scid:08x} tunnel_forward=true audio=false control=false video=true \
         video_codec=h264 max_size={} max_fps={} video_bit_rate={} \
         video_codec_options=i-frame-interval:int=1 cleanup=false",
        tuning.max_size, tuning.max_fps, tuning.bit_rate
    )
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

    /// One attempt: TCP + dummy. Retry only on [`AcceptError::NotListening`].
    pub async fn try_accept(local_port: u16) -> Result<Self, AcceptError> {
        let stream = Self::connect_host(local_port)
            .await
            .map_err(AcceptError::NotListening)?;
        let mut reader = tokio::io::BufReader::new(stream);
        match tokio::time::timeout(DUMMY_DEADLINE, read_dummy(&mut reader)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(AcceptError::NotListening(error)),
            Err(_) => {
                return Err(AcceptError::NotListening(anyhow::anyhow!(
                    "dummy byte did not arrive"
                )));
            }
        }
        match tokio::time::timeout(META_DEADLINE, read_name_and_video_header(&mut reader)).await {
            Ok(Ok(hello)) => Ok(Self {
                width: hello.width,
                height: hello.height,
                hello,
                reader,
                pending_config: None,
            }),
            Ok(Err(error)) => Err(AcceptError::Protocol(error)),
            Err(_) => Err(AcceptError::Protocol(anyhow::anyhow!(
                "timed out waiting for the scrcpy device name / video header"
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
pub const LEFTOVER_LIST_SCRIPT: &str = "\
for f in /proc/[0-9]*/cmdline; do \
grep -aq com.genymobile.scrcpy.Server \"$f\" || continue; \
grep -aq 3.3.4 \"$f\" || continue; \
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
        assert!(command.contains("control=false"), "{command}");
        assert!(command.contains("max_size=480"), "{command}");
        // The app's own declared rate, not a hardcoded 30. Before quality and fps were
        // wired, `get_stream_settings` told the operator 24 while the launch asked for
        // 30 — the UI and the encoder disagreed and neither said so. They agree now.
        assert!(
            command.contains(&format!("max_fps={}", riviu_core::STREAM_FPS)),
            "{command}"
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
    fn the_overlay_encode_is_wide_enough_for_the_size_it_is_shown_at() {
        // The bug the operator reported as a broken picture. `max_size` caps the LONG
        // edge, so on a 1080x2400 phone the encoded width is max_size * 1080/2400. The
        // overlay displays up to 760 px wide (FOCUS_ZOOM.max in zoom.ts), and an encode
        // narrower than that is upscaled — 600 gave 270 px and a 2.81x stretch.
        const FOCUS_MAX_DISPLAY_WIDTH: u32 = 760;
        let encoded_width = ViewPreset::Overlay.max_size() * 1080 / 2400;
        assert!(
            encoded_width >= FOCUS_MAX_DISPLAY_WIDTH * 9 / 10,
            "overlay encodes {encoded_width}px wide but is shown at up to              {FOCUS_MAX_DISPLAY_WIDTH}px"
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
        assert!(command.contains("max_size=1600"), "{command}");
        assert!(
            command.contains(&format!("max_fps={}", riviu_core::STREAM_FPS)),
            "{command}"
        );
        assert!(command.contains("video_bit_rate=4000000"), "{command}");
        assert_eq!(ViewPreset::parse("overlay").unwrap(), ViewPreset::Overlay);
        assert!(ViewPreset::parse("hevc").is_err());
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
