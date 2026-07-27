//! Riviu core types, device registry, job queue, and persistence.

pub mod db;
pub mod driver;
pub mod events;
pub mod frame_source;
pub mod human_behavior;
pub mod job_queue;
pub mod nurture;
pub mod openai_client;
pub mod registry;
pub mod screen;
pub mod screen_match;
pub mod screen_watch;
pub mod types;

pub use driver::{ui_error_kind, DeviceDriver, UiError, UiErrorKind, UiSession};
pub use events::{AppEvent, EventBus};
pub use frame_source::{Frame, FrameSource, FrameStream, NullFrameSource};
pub use job_queue::JobQueue;
pub use nurture::NurtureEngine;
pub use registry::DeviceRegistry;
pub use screen::{ScreenKind, ScreenObservation};
pub use screen_watch::{ScreenWatcher, WatchStats};
pub use types::*;
