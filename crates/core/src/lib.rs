//! Riviu core types, device registry, job queue, and persistence.

pub mod db;
pub mod device_capabilities;
pub mod device_control;
pub mod device_work;
pub mod driver;
pub mod events;
pub mod flow;
pub mod frame_source;
pub mod human_behavior;
pub mod job_queue;
pub mod nurture;
pub mod openai_client;
pub mod registry;
pub mod screen;
pub mod screen_match;
pub mod screen_watch;
pub mod stream_budget;
pub mod types;

pub use device_capabilities::*;
pub use device_control::{
    DeviceControlError, DeviceControlPlane, DeviceExclusiveContext, DeviceReleaseProof,
    ForegroundAppProof, InteractionAcquireResult, UiCapacityReservation, UiSessionContext,
    UiWithStreamContext,
};
pub use device_work::{
    DeviceBusy, DeviceWorkAcquireError, DeviceWorkCoordinator, DeviceWorkLease, DeviceWorkOwner,
    DeviceWorkTokenError,
};
pub use driver::{
    ui_error_kind, DeviceDriver, GuardedClipboardOperation, GuardedClipboardOutput,
    GuardedClipboardProgress, GuardedClipboardTransition, UiError, UiErrorKind, UiSession,
    UnsupportedCapability,
};
pub use events::{AppEvent, EventBus};
pub use flow::*;
pub use frame_source::{
    Frame, FrameSource, FrameStream, GenerationFrame, GenerationFrameEvent, GenerationFrameSource,
    GenerationFrameStream, NullFrameSource,
};
pub use job_queue::JobQueue;
pub use nurture::NurtureEngine;
pub use registry::DeviceRegistry;
pub use screen::{ScreenKind, ScreenObservation};
pub use screen_watch::{ScreenWatcher, WatchStats};
pub use stream_budget::{
    BackgroundStreamLease, ForegroundStreamReservation, ForegroundTransfer, StreamBudgetError,
    StreamBudgetManager, StreamStopRequest,
};
pub use types::*;
