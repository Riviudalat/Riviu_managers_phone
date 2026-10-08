//! Offline regression for the scrcpy view start path. No real ADB is resolved: the
//! executable is a per-test command recorder that answers `get-state` like adb does for
//! a phone that has left the bus.
use super::*;
use std::sync::atomic::AtomicU64;

struct GoneAdb {
    root: PathBuf,
    executable: PathBuf,
    recording: PathBuf,
}

impl GoneAdb {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("riviu-view-start-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let recording = root.join("calls.txt");
        let executable = root.join(if cfg!(windows) { "adb-fixture.cmd" } else { "adb-fixture.sh" });
        #[cfg(windows)]
        let script = {
            let log = recording.to_string_lossy().replace('%', "%%");
            format!("@echo off\r\n>>\"{log}\" echo %*\r\nif \"%~3\"==\"get-state\" goto gone\r\nexit /b 0\r\n:gone\r\n>&2 echo error: device '%~2' not found\r\nexit /b 1\r\n")
        };
        #[cfg(not(windows))]
        let script = {
            let log = recording.to_string_lossy().replace('\'', "'\\''");
            format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\nif [ \"$3\" = get-state ]; then echo \"error: device '$2' not found\" >&2; exit 1; fi\nexit 0\n")
        };
        std::fs::write(&executable, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self { root, executable, recording }
    }

    fn calls(&self) -> Vec<String> {
        match std::fs::read_to_string(&self.recording) {
            Ok(calls) => calls
                .lines()
                .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => panic!("cannot read fixture command recording: {error}"),
        }
    }
}

impl Drop for GoneAdb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[derive(Default)]
struct CountingSink {
    generation: AtomicU64,
}

impl crate::view::ViewSink for CountingSink {
    fn generation(&self, _udid: &str) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
    fn advance(&self, _udid: &str) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }
    fn publish(&self, _packet: crate::view::ViewPacket) -> bool {
        true
    }
}

#[tokio::test]
async fn a_phone_that_left_the_bus_costs_one_adb_call_and_is_never_woken() {
    let adb = GoneAdb::new();
    let server = adb.root.join("scrcpy-server");
    std::fs::write(&server, b"fixture jar").unwrap();
    let driver = AndroidDriver::with_adb(
        AdbProgram::at(adb.executable.clone()),
        adb::AdbOrigin::Configured,
        &AndroidDriverConfig {
            scrcpy_server: Some(server),
            automatic_setup_allowed: true,
            ..Default::default()
        },
    );
    driver.set_view_sink(Arc::new(CountingSink::default()));

    let error = driver
        .start_view_stream("fixture-gone", crate::scrcpy::ViewPreset::Tile)
        .await
        .unwrap_err();

    assert_eq!(
        adb.calls(),
        vec!["-s fixture-gone get-state".to_string()],
        "a missing transport must be refused before dumpsys, wake or push"
    );
    let message = format!("{error:#}");
    assert!(message.contains("Kiểm tra kết nối ADB của fixture-gone"), "{message}");
    assert!(!driver.view_start_in_flight("fixture-gone"), "the start claim is released");
}
