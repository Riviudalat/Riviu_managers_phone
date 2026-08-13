//! iOS device driver: pymobiledevice3 sidecar + WebDriverAgent, with optional mock mode.

pub mod agent;
pub mod config;
mod interaction_capabilities;
mod interaction_runtime;
mod mock;
mod pmd;
mod process_tree;
mod stream;
mod supervisor;
pub mod telemetry;
mod wda;

pub use agent::{
    decide_install, AgentArtifact, AgentInstallDecision, AgentManifest, InstalledAppMetadata,
    SUPPORTED_AGENT_PROTOCOL, SUPPORTED_CANDIDATE_AGENT_PROTOCOL,
};
pub use config::{AgentToken, DriverConfig, DriverTarget, UnifiedAgentConfig};
pub use interaction_capabilities::load_production_registry;
pub use mock::MockIosDriver;
pub use pmd::PmdIosDriver;
pub use process_tree::install_process_tree_guard;
pub use stream::{encode_rgb_jpeg, jpeg_quality, StreamHub};
pub use wda::WdaClient;

use std::sync::Arc;

use riviu_core::{DeviceCapabilityRegistry, DeviceDriver};

pub struct DriverBundle {
    pub driver: Arc<dyn DeviceDriver>,
    pub streams: StreamHub,
    pub mode: DriverMode,
    pub interaction_capabilities: Arc<DeviceCapabilityRegistry>,
    /// Why real-device support is unavailable, when the sidecar could not start.
    ///
    /// The app still runs in this state, but every device list comes back empty.
    /// Without this the UI could only say "no iPhone found" — indistinguishable
    /// from an unplugged cable, and wrong whenever the real cause is a broken
    /// sidecar. Carrying the reason lets the operator be told what to fix.
    pub degraded_reason: Option<String>,
    /// Why the **last** device listing came back empty, asked fresh each time.
    ///
    /// Separate from [`Self::degraded_reason`], which is a boot snapshot: this one changes
    /// while the app runs, so "the operator just installed Apple Devices" becomes visible
    /// without a restart, and a listing that succeeds clears it.
    ///
    /// A closure rather than a trait method on `DeviceDriver`. The reason is `MultiplexDriver`:
    /// with two backends there is no single answer to "why was the listing empty", so a
    /// trait method would have to invent one. The closure is captured from the concrete iOS
    /// driver at construction, where the question does have an answer.
    ///
    /// `None` for the mock and for a driver that never started.
    pub list_error: Option<Arc<dyn Fn() -> Option<String> + Send + Sync>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverMode {
    Mock,
    Pymobiledevice3,
}

pub async fn create_driver(config: DriverConfig) -> anyhow::Result<DriverBundle> {
    let interaction_capabilities = Arc::new(load_production_registry(
        &config
            .sidecar_root
            .join("wda")
            .join("interaction-capabilities.json"),
    )?);
    if matches!(&config.target, DriverTarget::Mock) {
        tracing::info!("using mock iOS driver");
        let mock = MockIosDriver::new();
        let streams = mock.stream_hub();
        return Ok(DriverBundle {
            driver: Arc::new(mock),
            streams,
            mode: DriverMode::Mock,
            interaction_capabilities,
            degraded_reason: None,
            list_error: None,
        });
    }

    match PmdIosDriver::probe(&config).await {
        Ok(driver) => {
            tracing::info!("using pymobiledevice3 iOS driver (real devices)");
            let streams = driver.stream_hub();
            // Not unconditionally `None` any more, and this one line is the whole fix
            // for the case the exit-2 tolerance opened: `probe` returns `Ok` for a
            // sidecar that answered but answered badly, and until now that verdict was
            // thrown away here, so a broken install reported as a healthy one with
            // nothing plugged in. The red banner already existed; it was simply never
            // given a reason to show.
            let degraded_reason = driver.boot_degraded_reason();
            if let Some(reason) = &degraded_reason {
                tracing::error!("iOS sidecar started degraded: {reason}");
            }
            let driver = Arc::new(driver);
            // Captured from the concrete driver, before it is erased to `dyn DeviceDriver`.
            let probe = driver.clone();
            Ok(DriverBundle {
                driver,
                streams,
                mode: DriverMode::Pymobiledevice3,
                interaction_capabilities,
                degraded_reason,
                list_error: Some(Arc::new(move || probe.last_list_error())),
            })
        }
        Err(err) => {
            // Starting degraded keeps the rest of the app usable, but it must not
            // look like a healthy install with nothing plugged in: carry the cause
            // out so the UI can show it.
            let reason = format!("{err:#}");
            tracing::error!(
                "pymobiledevice3 sidecar unavailable ({reason}); real devices cannot be listed"
            );
            let driver = PmdIosDriver::degraded(&config)?;
            let streams = driver.stream_hub();
            Ok(DriverBundle {
                driver: Arc::new(driver),
                streams,
                mode: DriverMode::Pymobiledevice3,
                interaction_capabilities,
                degraded_reason: Some(reason),
                // No sidecar means no listing, so there is no listing error to report —
                // `degraded_reason` already carries the more specific fact.
                list_error: None,
            })
        }
    }
}
