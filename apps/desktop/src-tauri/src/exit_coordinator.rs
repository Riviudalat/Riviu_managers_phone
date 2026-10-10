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

    #[cfg(test)]
    pub(crate) fn request(
        &self,
        drain: impl FnOnce() + Send + 'static,
        exit: impl FnOnce() + Send + 'static,
    ) -> std::io::Result<bool> {
        self.request_checked(move || { drain(); true }, exit)
    }

    pub(crate) fn request_checked(
        &self,
        drain: impl FnOnce() -> bool + Send + 'static,
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
                // The drain is finite: every device step carries its own deadline.
                // An unsettled drain still exits. Keeping the window open only made the
                // operator kill the process (2026-10-10: every phone had left adb, so
                // owned-helper absence was unprovable). Unresolved ownership lives in the
                // durable helper journals, not in this process, and the next start
                // reconciles it automatically when the effect ledger allows.
                if !drain() {
                    log::warn!("đóng ứng dụng: chưa xác minh xong việc dọn phiên trên máy; giữ nguyên bản ghi để tự đối soát ở lần mở sau");
                }
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
    fn failed_drain_still_exits_once_and_marks_completion() {
        let gate = ExitCoordinator::default();
        let (exited, wait_exit) = mpsc::channel();
        assert!(gate
            .request_checked(|| false, move || exited.send(()).unwrap())
            .unwrap());
        wait_exit.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(gate.completed());
        assert!(!gate
            .request_checked(|| panic!("second drain"), || panic!("second exit"))
            .unwrap());
    }

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
