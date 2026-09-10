use std::{collections::HashMap, future::Future};
use tokio::task::{Id, JoinSet};

/// Two independently progressing phones, with one observer per device.
#[derive(Default)]
pub(crate) struct VerificationQueue {
    tasks: JoinSet<anyhow::Result<bool>>,
    devices: HashMap<Id, String>,
}

impl VerificationQueue {
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn available(&self, udid: &str) -> bool {
        self.tasks.len() < 2 && !self.devices.values().any(|id| id == udid)
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

    #[tokio::test(start_paused = true)]
    async fn slow_device_does_not_hold_a_completed_phone_or_the_next_phone() {
        let mut queue = VerificationQueue::default();
        queue.push("slow".into(), async {
            tokio::time::sleep(Duration::from_secs(120)).await;
            Ok(false)
        });
        assert!(!queue.available("slow"));
        queue.push("fast".into(), async { Ok(true) });
        assert!(!queue.available("third"));
        let start = tokio::time::Instant::now();
        let (device, result) = queue.next().await.unwrap();
        assert_eq!(device, "fast");
        assert!(result.unwrap());
        assert!(start.elapsed() < Duration::from_secs(120));
        assert!(queue.available("third"));
        queue.push("third".into(), async { Ok(true) });
        assert_eq!(queue.next().await.unwrap().0, "third");
        assert_eq!(queue.next().await.unwrap().0, "slow");
        assert!(queue.is_empty());
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
