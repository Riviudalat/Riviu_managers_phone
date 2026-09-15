use std::{collections::HashMap, future::Future};
use tokio::task::{Id, JoinSet};

/// Bounded observers; pending publications remain durable database rows.
pub(crate) struct VerificationQueue {
    tasks: JoinSet<anyhow::Result<bool>>,
    capacity: usize,
    devices: HashMap<Id, String>,
}

impl Default for VerificationQueue {
    fn default() -> Self {
        Self::with_capacity(4)
    }
}
impl VerificationQueue {
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.clamp(1, 64);
    }
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            tasks: JoinSet::new(),
            devices: HashMap::new(),
            capacity: capacity.clamp(1, 64),
        }
    }
    pub fn device_can_observe(device: &riviu_core::DeviceInfo) -> bool {
        use riviu_core::{DevicePlatform, DeviceStatus};
        device.status == DeviceStatus::Ready
            || (device.platform == DevicePlatform::Android
                && device.status == DeviceStatus::Connected
                && device.wda_ready)
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// True when this UDID is not already running an observer.
    pub fn available(&self, udid: &str) -> bool {
        self.tasks.len() < self.capacity && !self.devices.values().any(|id| id == udid)
    }

    pub fn push(
        &mut self,
        udid: String,
        task: impl Future<Output = anyhow::Result<bool>> + Send + 'static,
    ) {
        assert!(self.available(&udid));
        let id = self.tasks.spawn(task).id();
        self.devices.insert(id, udid);
    }

    pub async fn next(&mut self) -> Option<(String, anyhow::Result<bool>)> {
        let (id, result) = match self.tasks.join_next_with_id().await? {
            Ok((id, result)) => (id, result),
            Err(error) => (error.id(), Err(anyhow::anyhow!(error))),
        };
        Some((
            self.devices.remove(&id).expect("owned verification task"),
            result,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reconnected_android_can_verify_without_opening_manual_control_first() {
        let mut device: riviu_core::DeviceInfo = serde_json::from_value(serde_json::json!({
            "udid":"phone", "name":"SM G955F", "model":"SM G955F", "platform":"android",
            "osVersion":"9", "connection":"usb", "status":"connected", "battery":78,
            "wdaReady":true, "wdaExpiresAt":null, "streamUrl":null, "tileStreamState":"live", "lastError":null
        })).unwrap();
        assert!(VerificationQueue::device_can_observe(&device));
        device.wda_ready = false;
        assert!(!VerificationQueue::device_can_observe(&device));
        device.wda_ready = true;
        for status in [
            riviu_core::DeviceStatus::Disconnected,
            riviu_core::DeviceStatus::Pairing,
            riviu_core::DeviceStatus::Preparing,
            riviu_core::DeviceStatus::Busy,
            riviu_core::DeviceStatus::Error,
        ] {
            device.status = status;
            assert!(!VerificationQueue::device_can_observe(&device));
        }
        device.platform = riviu_core::DevicePlatform::Ios;
        device.status = riviu_core::DeviceStatus::Connected;
        assert!(!VerificationQueue::device_can_observe(&device));
        device.status = riviu_core::DeviceStatus::Ready;
        assert!(VerificationQueue::device_can_observe(&device));
    }

    #[tokio::test(start_paused = true)]
    async fn slow_device_does_not_hold_a_completed_phone_or_the_next_phone() {
        let mut queue = VerificationQueue::with_capacity(6);
        queue.push("slow".into(), async {
            tokio::time::sleep(Duration::from_secs(120)).await;
            Ok(false)
        });
        assert!(!queue.available("slow"));
        for name in ["fast", "third", "fourth", "fifth", "sixth"] {
            assert!(queue.available(name), "{name}");
            queue.push(name.to_string(), async { Ok(true) });
        }
        assert!(!queue.available("fast"));
        let start = tokio::time::Instant::now();
        let mut finished =
            std::collections::HashSet::from(["fast", "third", "fourth", "fifth", "sixth"]);
        for _ in 0..5 {
            let (device, result) = queue.next().await.unwrap();
            assert!(result.unwrap());
            assert!(finished.remove(device.as_str()), "{device}");
            assert!(start.elapsed() < Duration::from_secs(120));
        }
        assert_eq!(queue.next().await.unwrap().0, "slow");
        assert!(queue.is_empty());
        assert!(queue.available("slow"));
    }

    #[tokio::test]
    async fn a_failed_observer_releases_only_its_own_device() {
        let mut queue = VerificationQueue::default();
        queue.push("broken".into(), async {
            panic!("fixture observer");
        });
        let (device, result) = queue.next().await.unwrap();
        assert_eq!(device, "broken");
        assert!(result.is_err());
        assert!(queue.available("broken"));
    }
}
