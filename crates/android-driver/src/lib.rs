//! Android backend for the device fleet, behind the same `DeviceDriver` and
//! `UiSession` traits the iOS driver implements.
//!
//! The original design doc reserved this seam on day one — *"manage and
//! automate multiple iPhones, with Android deferred behind a `DeviceDriver`
//! trait"* (`docs/superpowers/specs/2026-07-25-riviu-managers-phone-design.md:7`).
//! This crate fills it.
//!
//! Two layers, split by what each is good for, and the split is measured
//! rather than assumed (`docs/ANDROID_PROBE_REPORT_2026-08-09.md`):
//!
//! - [`adb`] — the `adb` CLI, for device lifecycle. A call costs 1–2 s on the
//!   Galaxy S8+ fleet, which is fine for install/launch/stop and wrong for
//!   anything repeated.
//! - [`agent`] — HTTP to a resident on-device agent. Click 130–280 ms, find
//!   609 ms, read attribute 241 ms.
//!
//! The one operation the agent does *not* make cheap is dumping the whole
//! hierarchy (3403 ms), because that cost is traversing the accessibility tree,
//! not starting a tool. So the rule for anything in a control loop is: query
//! for the element you want, never walk the tree.
//!
//! Where this differs from iOS, it is usually simpler. `adb install` needs no
//! per-device code signing and nothing expires. Accessibility read-back works,
//! so `screen.rs`-style pixel geometry is not needed: TikTok labels its own
//! controls (`Like` flips to `Video liked`) and those labels are English
//! whatever the UI language. Vietnamese types cleanly through
//! `ACTION_SET_TEXT`, which is why no custom keyboard is shipped here.

pub mod adb;
pub mod agent;
pub mod driver;
pub mod frames;
pub mod publish;
pub mod session;

pub use adb::{AdbCandidate, AdbOrigin, AdbProgram};
pub use agent::{AgentClient, Locator, Rect};
pub use driver::{create_driver, detect_driver, AndroidDriver, AndroidDriverConfig};
pub use frames::{MinicapBanner, MinicapOptions, MinicapStream, Projection};
pub use session::AndroidUiSession;
