//! JPEG frames from an Android screen, for the same `StreamHub::publish` sink the
//! iOS MJPEG reader feeds.
//!
//! Why minicap and not scrcpy: measured on a Redmi Note 12 / Android 15
//! (`docs/re/genfarmer/README.md` §7). `FrameSource::Frame` is JPEG, and the
//! desktop builds `image` with only the `jpeg` feature, so a source has to emit
//! JPEG or drag a video decoder in behind it.
//!
//! | candidate | measured |
//! |---|---|
//! | `screencap -p` | 512 ms/frame — a snapshot, not a stream |
//! | `screencap` raw | 990 ms and 10 MB/frame; the USB transfer dominates |
//! | scrcpy 2.4 | H.264, so it needs a decoder before core can look at a pixel |
//! | minicap native | dead on Android 15: prebuilt `.so` stops at android-30 and the platform dropped `android::ui::Size::INVALID` |
//! | **minicap Java** | **25.8 FPS, 43.2 KB/frame** at `1080x2400@540x1200`, Q70, read by this module |
//!
//! The 25.8 FPS is what the G1 probe measures through this code. A scripted
//! PowerShell reader over the same socket managed 11 FPS on the same phone and
//! projection — the difference was the reader, not the device, so do not treat a
//! slow harness as the device's ceiling.
//!
//! The Java build is also the one that does not need installing — it runs through
//! `app_process`, so it is unaffected by the MIUI gate that blocks `pm install`
//! (AGENTS.md §9). It emits a frame only when the display changes, which is what
//! the watcher wants: a still feed costs nothing (§3.4).
//!
//! This module produces frames. It deliberately does **not** own generations,
//! stream budget or ownership — `StreamHub` already does, and duplicating that
//! would create a second source of truth for evidence ordering.

use std::time::Duration;

use anyhow::{anyhow, Context};
use tokio::io::{AsyncReadExt, BufReader};
use tokio::net::TcpStream;

use crate::adb::AdbProgram;

/// Where the APK is pushed. Named for us so a co-resident farm tool's copy is not
/// mistaken for ours — GenFarmer pushes its scrcpy build as `genscrcpy.jar` for
/// the same reason.
pub const REMOTE_APK: &str = "/data/local/tmp/riviu-minicap.apk";

/// The Kotlin entry point inside `noarch/minicap.apk`.
const MAIN_CLASS: &str = "io.devicefarmer.minicap.Main";

/// A frame larger than this means the length prefix desynchronised, not a big
/// screen: a full-scale Q100 JPEG of 1080x2400 measured ~1 MB.
const MAX_FRAME_BYTES: u32 = 8 * 1024 * 1024;

const BANNER_BYTES: usize = 24;

/// What minicap should capture, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    pub real_width: u32,
    pub real_height: u32,
    pub virtual_width: u32,
    pub virtual_height: u32,
    pub rotation: u32,
}

impl Projection {
    /// Capture at the device's own size.
    pub fn native(width: u32, height: u32) -> Self {
        Self {
            real_width: width,
            real_height: height,
            virtual_width: width,
            virtual_height: height,
            rotation: 0,
        }
    }

    /// Half of each edge. The detectors already tolerate this — `session.rs`
    /// notes that "a half-scale screenshot maps back to full device pixels" — and
    /// it is the shape the 11 FPS measurement was taken at.
    pub fn half(width: u32, height: u32) -> Self {
        Self {
            real_width: width,
            real_height: height,
            virtual_width: (width / 2).max(1),
            virtual_height: (height / 2).max(1),
            rotation: 0,
        }
    }

    /// minicap's `-P` argument.
    pub fn to_arg(&self) -> String {
        format!(
            "{}x{}@{}x{}/{}",
            self.real_width,
            self.real_height,
            self.virtual_width,
            self.virtual_height,
            self.rotation
        )
    }
}

#[derive(Debug, Clone)]
pub struct MinicapOptions {
    pub projection: Projection,
    /// 0–100. 70 measured 55.9 KB/frame at half scale.
    pub quality: u8,
    /// Abstract unix socket name on the device. One per device keeps two phones
    /// from colliding on the same name.
    pub socket: String,
    /// Drop frames the reader cannot keep up with instead of queueing them. This
    /// is the same "coalesce rather than queue" contract `FrameStream` documents.
    pub skip_frames: bool,
}

impl MinicapOptions {
    pub fn for_device(serial: &str, projection: Projection) -> Self {
        Self {
            projection,
            quality: 70,
            socket: format!("riviu-minicap-{serial}"),
            skip_frames: true,
        }
    }
}

/// minicap's 24-byte greeting, read once before any frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinicapBanner {
    pub version: u8,
    pub pid: u32,
    pub real_width: u32,
    pub real_height: u32,
    pub virtual_width: u32,
    pub virtual_height: u32,
    pub orientation: u8,
    pub quirks: u8,
}

/// Parse the banner. Layout is fixed little-endian, measured against the real
/// device: `version=1 pid=… real=1080x2400 virtual=540x1200 orient=0 quirks=2`.
pub fn parse_banner(bytes: &[u8]) -> anyhow::Result<MinicapBanner> {
    if bytes.len() < BANNER_BYTES {
        return Err(anyhow!(
            "minicap banner is {} bytes, expected {BANNER_BYTES}",
            bytes.len()
        ));
    }
    let read_u32 =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let banner = MinicapBanner {
        version: bytes[0],
        pid: read_u32(2),
        real_width: read_u32(6),
        real_height: read_u32(10),
        virtual_width: read_u32(14),
        virtual_height: read_u32(18),
        orientation: bytes[22],
        quirks: bytes[23],
    };
    // A zero-sized display means we are reading something that is not minicap —
    // fail here rather than hand the watcher frames of nothing.
    if banner.real_width == 0 || banner.real_height == 0 {
        return Err(anyhow!(
            "minicap reported a {}x{} display",
            banner.real_width,
            banner.real_height
        ));
    }
    Ok(banner)
}

/// Pick the local port adb assigned for `tcp:0`.
///
/// `adb forward tcp:0 …` is the right request on a fleet — a fixed host port per
/// device runs out and collides with whatever else is forwarding — but it means
/// the caller has to read the port back out of `forward --list` rather than
/// assume it (`docs/re/genfarmer/README.md` §4.2).
pub fn parse_forward_port(stdout: &str, serial: &str, remote: &str) -> Option<u16> {
    parse_forward_ports(stdout, serial, remote)
        .into_iter()
        .next()
}

/// Every host port forwarded to `remote` for this device.
///
/// More than one is the interesting case: `adb forward tcp:0` allocates a fresh
/// port per call, so a producer that died without tearing down leaves its port
/// bound to the same socket. Measured two stranded ports after one app run.
pub fn parse_forward_ports(stdout: &str, serial: &str, remote: &str) -> Vec<u16> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let line_serial = parts.next()?;
            let local = parts.next()?;
            let line_remote = parts.next()?;
            if line_serial != serial || line_remote != remote {
                return None;
            }
            local.strip_prefix("tcp:")?.parse().ok()
        })
        .collect()
}

/// Reclaim host ports already forwarded to our socket for this device.
///
/// The socket name carries the serial and our own prefix, so anything forwarded
/// to it that we are not currently reading is a leftover. Pruning here makes the
/// leak structurally impossible rather than depending on every teardown path
/// having run — which is what actually failed in testing.
pub async fn prune_forwards(adb: &AdbProgram, serial: &str, socket: &str) -> usize {
    let remote = format!("localabstract:{socket}");
    let listing = match adb
        .run(&["forward", "--list"], Duration::from_secs(30))
        .await
    {
        Ok(listing) => listing,
        Err(_) => return 0,
    };
    let stale = parse_forward_ports(&listing, serial, &remote);
    let mut removed = 0;
    for port in stale {
        if remove_forward(adb, serial, port).await.is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(serial, removed, "reclaimed stale minicap forwards");
    }
    removed
}

/// Push the APK if the device does not already have the same number of bytes.
///
/// Byte count rather than a hash because `md5sum` is another 1–2 s adb round trip
/// per device, and the file is ours at a name nothing else writes.
pub async fn ensure_apk(
    adb: &AdbProgram,
    serial: &str,
    local_apk: &std::path::Path,
) -> anyhow::Result<()> {
    let local_len = tokio::fs::metadata(local_apk)
        .await
        .with_context(|| format!("read {}", local_apk.display()))?
        .len();
    let remote_len = adb
        .shell(serial, &format!("wc -c < {REMOTE_APK} 2>/dev/null"))
        .await
        .ok()
        .and_then(|out| out.trim().parse::<u64>().ok());
    if remote_len == Some(local_len) {
        return Ok(());
    }
    adb.device(
        serial,
        &["push", &local_apk.display().to_string(), REMOTE_APK],
        Duration::from_secs(120),
    )
    .await
    .context("push the minicap apk")?;
    Ok(())
}

/// The screen size minicap must be told about, in the space everything else uses.
///
/// `wm size` prints two lines when an override is set and the override is what is
/// rendered, so this goes through the same parser the rest of the driver does
/// rather than reading the physical line (AGENTS.md §9).
pub async fn device_screen(adb: &AdbProgram, serial: &str) -> anyhow::Result<(u32, u32)> {
    let stdout = adb.shell(serial, "wm size").await?;
    crate::adb::parse_wm_size(&stdout)
        .ok_or_else(|| anyhow!("could not read `wm size` for {serial}: {stdout:?}"))
}

/// Forward a host port to minicap's abstract socket and return the port adb chose.
///
/// Asks for `tcp:0` rather than a fixed port: on a 20-phone fleet fixed ports
/// collide, both between devices and with whatever else on the machine is
/// forwarding. The assigned port is then read back out of `forward --list`,
/// because the create call does not report it.
pub async fn forward(adb: &AdbProgram, serial: &str, socket: &str) -> anyhow::Result<u16> {
    let remote = format!("localabstract:{socket}");
    // Take back anything a previous producer left bound to this socket first.
    prune_forwards(adb, serial, socket).await;
    adb.device(
        serial,
        &["forward", "tcp:0", &remote],
        Duration::from_secs(30),
    )
    .await
    .with_context(|| format!("forward tcp:0 to {remote}"))?;
    let listing = adb
        .run(&["forward", "--list"], Duration::from_secs(30))
        .await
        .context("list adb forwards")?;
    parse_forward_port(&listing, serial, &remote).ok_or_else(|| {
        anyhow!("adb reported no forward for {serial} -> {remote}; listing was {listing:?}")
    })
}

/// Drop a forward we created. Best-effort: leaving one behind wastes a port, so
/// callers log rather than abort when this fails.
pub async fn remove_forward(adb: &AdbProgram, serial: &str, port: u16) -> anyhow::Result<()> {
    adb.device(
        serial,
        &["forward", "--remove", &format!("tcp:{port}")],
        Duration::from_secs(30),
    )
    .await
    .map(|_| ())
}

/// The command that runs minicap without installing it.
pub fn launch_command(options: &MinicapOptions) -> String {
    let mut command = format!(
        "CLASSPATH={REMOTE_APK} app_process / {MAIN_CLASS} -n {} -P {} -Q {}",
        options.socket,
        options.projection.to_arg(),
        options.quality
    );
    if options.skip_frames {
        command.push_str(" -S");
    }
    command
}

/// A connected minicap feed.
pub struct MinicapStream {
    reader: BufReader<TcpStream>,
    banner: MinicapBanner,
}

impl MinicapStream {
    /// Connect to an already-forwarded minicap and read its banner.
    pub async fn connect(local_port: u16) -> anyhow::Result<Self> {
        let stream = TcpStream::connect(("127.0.0.1", local_port))
            .await
            .with_context(|| format!("connect to minicap on 127.0.0.1:{local_port}"))?;
        let mut reader = BufReader::new(stream);
        let mut raw = [0u8; BANNER_BYTES];
        reader
            .read_exact(&mut raw)
            .await
            .context("read the minicap banner")?;
        let banner = parse_banner(&raw)?;
        Ok(Self { reader, banner })
    }

    pub fn banner(&self) -> &MinicapBanner {
        &self.banner
    }

    /// The next JPEG frame.
    ///
    /// Blocks until the display changes — minicap publishes on change, so a still
    /// screen simply yields nothing rather than repeating the last frame. Callers
    /// that need a deadline put one around this; do not add an internal timeout
    /// that would turn "nothing moved" into an error.
    pub async fn next_frame(&mut self) -> anyhow::Result<Vec<u8>> {
        let mut length = [0u8; 4];
        self.reader
            .read_exact(&mut length)
            .await
            .context("read a minicap frame length")?;
        let length = u32::from_le_bytes(length);
        if length == 0 || length > MAX_FRAME_BYTES {
            return Err(anyhow!(
                "minicap frame length {length} is out of range; the stream desynchronised"
            ));
        }
        let mut frame = vec![0u8; length as usize];
        self.reader
            .read_exact(&mut frame)
            .await
            .context("read a minicap frame")?;
        anyhow::ensure!(
            is_jpeg(&frame),
            "minicap frame of {length} bytes does not start with the JPEG magic"
        );
        Ok(frame)
    }
}

/// JPEG start-of-image. Checked on every frame for the same reason the screenshot
/// paths check the PNG magic: a length-prefixed blob of the right size is not
/// evidence that it decodes.
pub fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact banner measured on the Redmi Note 12 at half scale.
    fn measured_banner() -> [u8; BANNER_BYTES] {
        let mut raw = [0u8; BANNER_BYTES];
        raw[0] = 1; // version
        raw[1] = BANNER_BYTES as u8;
        raw[2..6].copy_from_slice(&4567u32.to_le_bytes()); // pid
        raw[6..10].copy_from_slice(&1080u32.to_le_bytes());
        raw[10..14].copy_from_slice(&2400u32.to_le_bytes());
        raw[14..18].copy_from_slice(&540u32.to_le_bytes());
        raw[18..22].copy_from_slice(&1200u32.to_le_bytes());
        raw[22] = 0; // orientation
        raw[23] = 2; // quirks: ALWAYS_UPRIGHT
        raw
    }

    #[test]
    fn banner_matches_what_the_device_sent() {
        let banner = parse_banner(&measured_banner()).expect("banner");
        assert_eq!(banner.version, 1);
        assert_eq!(banner.pid, 4567);
        assert_eq!((banner.real_width, banner.real_height), (1080, 2400));
        assert_eq!((banner.virtual_width, banner.virtual_height), (540, 1200));
        assert_eq!(banner.orientation, 0);
        assert_eq!(banner.quirks, 2);
    }

    #[test]
    fn a_short_or_empty_display_banner_is_refused() {
        assert!(parse_banner(&[0u8; 10]).is_err());
        // Right length, zero display: this is the case that would otherwise feed
        // the watcher frames of nothing.
        let mut raw = measured_banner();
        raw[6..10].copy_from_slice(&0u32.to_le_bytes());
        assert!(parse_banner(&raw).is_err());
    }

    #[test]
    fn projection_renders_minicaps_argument() {
        assert_eq!(
            Projection::native(1080, 2400).to_arg(),
            "1080x2400@1080x2400/0"
        );
        // The shape the 11 FPS measurement used.
        assert_eq!(
            Projection::half(1080, 2400).to_arg(),
            "1080x2400@540x1200/0"
        );
    }

    #[test]
    fn launch_command_runs_without_installing_anything() {
        let options = MinicapOptions::for_device("SERIAL", Projection::half(1080, 2400));
        let command = launch_command(&options);
        assert!(command.starts_with("CLASSPATH=/data/local/tmp/riviu-minicap.apk app_process / io.devicefarmer.minicap.Main"));
        assert!(command.contains("-n riviu-minicap-SERIAL"));
        assert!(command.contains("-P 1080x2400@540x1200/0"));
        assert!(command.contains("-Q 70"));
        // Skipping is the default: a slow consumer must not build a backlog.
        assert!(command.ends_with(" -S"));
        // `pm install` is what MIUI blocks; this path must never need it.
        assert!(!command.contains("pm install"));
    }

    #[test]
    fn forward_port_is_read_back_not_assumed() {
        let listing = "10969614 tcp:52341 localabstract:riviu-minicap-10969614\n\
                       OTHER    tcp:6790  localabstract:riviu-minicap-OTHER\n";
        assert_eq!(
            parse_forward_port(listing, "10969614", "localabstract:riviu-minicap-10969614"),
            Some(52341)
        );
        // Another device's forward for the same kind of socket is not ours.
        assert_eq!(
            parse_forward_port(listing, "10969614", "localabstract:riviu-minicap-OTHER"),
            None
        );
        assert_eq!(parse_forward_port("", "10969614", "localabstract:x"), None);
    }

    #[test]
    fn every_stranded_port_for_our_socket_is_found() {
        // The real shape after a teardown that did not run: two host ports bound
        // to the same device socket. Pruning has to see both, not just the first.
        let listing = "10969614 tcp:51417 localabstract:riviu-minicap-10969614\n\
                       10969614 tcp:6790  tcp:6790\n\
                       10969614 tcp:57639 localabstract:riviu-minicap-10969614\n";
        assert_eq!(
            parse_forward_ports(listing, "10969614", "localabstract:riviu-minicap-10969614"),
            vec![51417, 57639]
        );
        // The agent's own forward is a different remote and must be left alone.
        assert_eq!(
            parse_forward_ports(listing, "10969614", "tcp:6790"),
            vec![6790]
        );
    }

    #[test]
    fn jpeg_magic_gates_the_frame() {
        assert!(is_jpeg(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]));
        // A PNG is the realistic wrong answer here: `screencap -p` returns one.
        assert!(!is_jpeg(&[0x89, b'P', b'N', b'G']));
        assert!(!is_jpeg(&[]));
    }
}
