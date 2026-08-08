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
        });
    }

    match PmdIosDriver::probe(&config).await {
        Ok(driver) => {
            tracing::info!("using pymobiledevice3 iOS driver (real devices)");
            let streams = driver.stream_hub();
            Ok(DriverBundle {
                driver: Arc::new(driver),
                streams,
                mode: DriverMode::Pymobiledevice3,
                interaction_capabilities,
                degraded_reason: None,
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
            })
        }
    }
}
