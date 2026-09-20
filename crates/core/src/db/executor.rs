use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Admission precedes spawn_blocking: saturation never fills Tokio's blocking
/// pool with threads waiting for SQLite or a storage slot.
pub(super) struct StorageExecutor {
    readers: Arc<Semaphore>,
    writers: Arc<Semaphore>,
    waiting: Arc<Semaphore>,
}

impl Default for StorageExecutor {
    fn default() -> Self {
        Self {
            readers: Arc::new(Semaphore::new(2)),
            writers: Arc::new(Semaphore::new(1)),
            waiting: Arc::new(Semaphore::new(64)),
        }
    }
}

impl StorageExecutor {
    async fn admit(&self, write: bool) -> anyhow::Result<OwnedSemaphorePermit> {
        let lane = if write { &self.writers } else { &self.readers };
        if let Ok(permit) = lane.clone().try_acquire_owned() {
            return Ok(permit);
        }
        let waiting = self
            .waiting
            .clone()
            .try_acquire_owned()
            .map_err(|_| anyhow::anyhow!("StorageBusy: 64 storage requests already waiting"))?;
        let permit = lane.clone().acquire_owned().await?;
        drop(waiting);
        Ok(permit)
    }
}

impl super::Database {
    pub async fn storage_read<T, F>(self: &Arc<Self>, action: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Self) -> anyhow::Result<T> + Send + 'static,
    {
        self.storage_run(false, action).await
    }

    pub async fn storage_write<T, F>(self: &Arc<Self>, action: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Self) -> anyhow::Result<T> + Send + 'static,
    {
        self.storage_run(true, action).await
    }

    async fn storage_run<T, F>(self: &Arc<Self>, write: bool, action: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Self) -> anyhow::Result<T> + Send + 'static,
    {
        let permit = self.storage.admit(write).await?;
        let db = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            action(&db)
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn regression_storage_admission_bounds_queue_and_cancel_releases_waiter() {
        let storage = Arc::new(StorageExecutor::default());
        let writer = storage.admit(true).await.unwrap();
        let r1 = storage.admit(false).await.unwrap();
        let r2 = storage.admit(false).await.unwrap();
        let mut requests = Vec::new();
        for _ in 0..64 {
            let storage = storage.clone();
            requests.push(tokio::spawn(async move { storage.admit(true).await }));
        }
        while storage.waiting.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
        assert!(storage
            .admit(false)
            .await
            .unwrap_err()
            .to_string()
            .contains("StorageBusy"));
        for request in requests {
            request.abort();
            let _ = request.await;
        }
        assert_eq!(storage.waiting.available_permits(), 64);
        drop((writer, r1, r2));
        assert!(storage.admit(true).await.is_ok());
    }
}
