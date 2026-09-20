//! Drain the control plane without blocking the native window event loop.
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

#[derive(Clone, Default)]
pub(crate) struct ExitCoordinator(Arc<AtomicU8>);
impl ExitCoordinator {
    pub(crate) fn completed(&self) -> bool {
        self.0.load(Ordering::Acquire) == 2
    }

    pub(crate) fn request(
        &self,
        drain: impl FnOnce() + Send + 'static,
        exit: impl FnOnce() + Send + 'static,
    ) -> std::io::Result<bool> {
        if self
            .0
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(false);
        }
        let state = self.0.clone();
        let worker = std::thread::Builder::new()
            .name("riviu-graceful-exit".into())
            .spawn(move || {
                drain();
                state.store(2, Ordering::Release);
                exit();
            });
        if let Err(error) = worker {
            self.0.store(0, Ordering::Release);
            return Err(error);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn native_loop_remains_available_until_drain_finishes_and_duplicate_close_is_coalesced() {
        let gate = ExitCoordinator::default();
        let (work_started, started) = mpsc::channel();
        let (native_ack, wait_native_ack) = mpsc::channel();
        let (exited, wait_exit) = mpsc::channel();
        let native_thread = std::thread::current().id();
        assert!(gate
            .request(
                move || {
                    assert_ne!(std::thread::current().id(), native_thread);
                    work_started.send(()).unwrap();
                    wait_native_ack
                        .recv_timeout(Duration::from_secs(3))
                        .unwrap();
                },
                move || {
                    exited.send(()).unwrap();
                }
            )
            .unwrap());
        started.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(!gate.completed());
        assert!(!gate
            .request(|| panic!("second drain"), || panic!("second exit"))
            .unwrap());
        assert!(wait_exit.try_recv().is_err());
        native_ack.send(()).unwrap();
        wait_exit.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(gate.completed());
    }
}
