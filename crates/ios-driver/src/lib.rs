//! iOS device driver: pymobiledevice3 sidecar + WebDriverAgent, with optional mock mode.

mod mock;
mod pmd;
mod stream;
mod supervisor;
pub mod telemetry;
mod wda;

pub use mock::MockIosDriver;
pub use pmd::PmdIosDriver;
pub use stream::{encode_rgb_jpeg, jpeg_quality, StreamHub};
pub use wda::WdaClient;

use std::path::PathBuf;
use std::sync::Arc;

use riviu_core::DeviceDriver;

pub struct DriverBundle {
    pub driver: Arc<dyn DeviceDriver>,
    pub streams: StreamHub,
    pub mode: DriverMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverMode {
    Mock,
    Pymobiledevice3,
}

/// Real devices by default. Mock only when `RIVIU_MOCK_DEVICES=1`.
pub async fn create_driver(sidecar_dir: PathBuf) -> DriverBundle {
    let force_mock = std::env::var("RIVIU_MOCK_DEVICES")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if force_mock {
        tracing::info!("using mock iOS driver (RIVIU_MOCK_DEVICES=1)");
        let mock = MockIosDriver::new();
        let streams = mock.stream_hub();
        return DriverBundle {
            driver: Arc::new(mock),
            streams,
            mode: DriverMode::Mock,
        };
    }

    match PmdIosDriver::probe(sidecar_dir).await {
        Ok(driver) => {
            tracing::info!("using pymobiledevice3 iOS driver (real devices)");
            let streams = driver.stream_hub();
            DriverBundle {
                driver: Arc::new(driver),
                streams,
                mode: DriverMode::Pymobiledevice3,
            }
        }
        Err(err) => {
            tracing::warn!(
                "pymobiledevice3 sidecar unavailable ({err:#}); empty real device list"
            );
            let driver = PmdIosDriver::degraded();
            let streams = driver.stream_hub();
            DriverBundle {
                driver: Arc::new(driver),
                streams,
                mode: DriverMode::Pymobiledevice3,
            }
        }
    }
}
