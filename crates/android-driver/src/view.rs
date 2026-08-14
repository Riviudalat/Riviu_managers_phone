//! The desktop **view** path: H.264 (Android scrcpy) or JPEG bytes (iOS MJPEG).
//!
//! This is not [`riviu_core::FrameSink`]. That trait publishes JPEG into
//! `StreamHub` and is the evidence contract for nurture, interaction and the
//! popup watcher. A view packet must never become a `Frame`. Old readers of a
//! UDID must not paint into a canvas that belongs to a newer generation.

/// What the frontend decoder should do with [`ViewPacket::bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    /// Annex-B H.264 access unit. Config (SPS/PPS) is already merged onto the
    /// following sample when this is published.
    H264,
    /// A complete JPEG. Used for iOS MJPEG so the same canvas/worker paints
    /// both platforms.
    Jpeg,
}

/// One sample for the view WebSocket. Cheap to clone the header; the payload
/// is moved, not shared, because the worker consumes it once.
#[derive(Debug, Clone)]
pub struct ViewPacket {
    pub udid: String,
    pub generation: u64,
    pub kind: ViewKind,
    pub width: u32,
    pub height: u32,
    /// True when this H.264 sample is a keyframe (or a JPEG, which is always
    /// independently decodable).
    pub key: bool,
    pub bytes: Vec<u8>,
}

/// Fan-out for view samples. Implemented by the desktop `ViewHub`.
///
/// `publish` returns `false` when `packet.generation` is stale — the reader
/// must stop, the same signal `FrameSink::publish_if_current` uses.
pub trait ViewSink: Send + Sync {
    fn generation(&self, udid: &str) -> u64;
    fn advance(&self, udid: &str) -> u64;
    fn publish(&self, packet: ViewPacket) -> bool;
}
