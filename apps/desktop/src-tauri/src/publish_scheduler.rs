//! Due jobs run independently across devices, serially on any one device.
use std::collections::{HashMap, HashSet};

pub(crate) const MAX_SCHEDULED_PUBLISH_JOBS: usize = 10;

#[derive(Default)]
pub(crate) struct PublishScheduleDispatch {
    active: HashMap<String, Vec<String>>,
}
impl PublishScheduleDispatch {
    pub fn reserve(&mut self, campaign: &str, devices: &[String]) -> bool {
        if self.active.len() >= MAX_SCHEDULED_PUBLISH_JOBS
            || devices.is_empty()
            || self.active.contains_key(campaign)
        {
            return false;
        }
        let occupied: HashSet<_> = self.active.values().flatten().collect();
        if devices.iter().any(|id| occupied.contains(id)) {
            return false;
        }
        self.active.insert(campaign.to_owned(), devices.to_vec());
        true
    }
    pub fn release(&mut self, campaign: &str) {
        self.active.remove(campaign);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn ten_due_devices_start_without_waiting_for_the_first_to_finish() {
        let mut dispatch = PublishScheduleDispatch::default();
        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(11));
        let mut tasks = tokio::task::JoinSet::new();
        for index in 0..10 {
            let campaign = format!("job{index}");
            let device = format!("phone{index}");
            assert!(dispatch.reserve(&campaign, std::slice::from_ref(&device)));
            assert!(!dispatch.reserve(&campaign, std::slice::from_ref(&device)));
            assert!(!dispatch.reserve("same-phone", &[device]));
            let tx = tx.clone();
            let barrier = barrier.clone();
            tasks.spawn(async move {
                tx.send(campaign.clone()).await.unwrap();
                barrier.wait().await;
                campaign
            });
        }
        assert!(!dispatch.reserve("over-cap", &["extra".into()]));
        for _ in 0..10 {
            tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
        }
        barrier.wait().await;
        while let Some(result) = tasks.join_next().await {
            dispatch.release(&result.unwrap());
        }
        assert!(dispatch.reserve("later", &["phone0".into()]));
    }
}
