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
            })
        }
        Err(err) => {
            tracing::warn!("pymobiledevice3 sidecar unavailable ({err:#}); empty real device list");
            let driver = PmdIosDriver::degraded(&config)?;
            let streams = driver.stream_hub();
            Ok(DriverBundle {
                driver: Arc::new(driver),
                streams,
                mode: DriverMode::Pymobiledevice3,
                interaction_capabilities,
            })
        }
    }
}
