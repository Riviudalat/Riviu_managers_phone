//! Offline behavioral regression for borrowed Appium transport shutdown.
//! No real ADB is resolved: the executable is a per-test command recorder. The
//! HTTP endpoint is an ephemeral loopback fake, not a device/server session.
use super::*;
use crate::agent::AndroidObservationMode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct FakeAdb {
    root: PathBuf,
    executable: PathBuf,
    recording: PathBuf,
}
impl FakeAdb {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("riviu-borrowed-shutdown-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let recording = root.join("calls.txt");
        let executable = root.join(if cfg!(windows) {
            "adb-fixture.cmd"
        } else {
            "adb-fixture.sh"
        });
        #[cfg(windows)]
        let script = {
            let log = recording.to_string_lossy().replace('%', "%%");
            // Keep exit labels outside parenthesized IF blocks: an offline cmd
            // reproduction showed the first pidof returned 0 despite exit /b 1
            // inside the block. Both absence branches below were exercised with
            // the exact production argv and returned 1, empty stdout/stderr.
            format!("@echo off\r\n>>\"{log}\" echo %~1 %~2 %~3 %~4 %~5 %~6\r\nif not \"%~3\"==\"shell\" goto success\r\nif \"%~4\"==\"pidof\" goto absent\r\nif \"%~4\"==\"pidof io.appium.uiautomator2.server\" goto absent\r\nif \"%~4\"==\"pidof io.appium.uiautomator2.server.test\" goto absent\r\n:success\r\nexit /b 0\r\n:absent\r\nexit /b 1\r\n")
        };
        #[cfg(not(windows))]
        let script = {
            let log = recording.to_string_lossy().replace('\'', "'\\''");
            format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\ncase \"$4\" in\n  'pidof io.appium.uiautomator2.server'|'pidof io.appium.uiautomator2.server.test') exit 1 ;;\nesac\nexit 0\n")
        };
        std::fs::write(&executable, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self {
            root,
            executable,
            recording,
        }
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
impl Drop for FakeAdb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct AppiumFixture {
    base: String,
    calls: Arc<Mutex<Vec<String>>>,
    stopped: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}
impl AppiumFixture {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let (stopped, mut stop) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    _ = &mut stop => break,
                    connection = listener.accept() => connection.unwrap(),
                };
                let (mut socket, _) = accepted;
                let mut data = Vec::new();
                let mut chunk = [0u8; 4096];
                let route = loop {
                    let n = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut chunk))
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(n > 0, "fixture HTTP request ended before headers");
                    data.extend_from_slice(&chunk[..n]);
                    assert!(
                        data.len() <= 32768,
                        "fixture request exceeded bounded header limit"
                    );
                    if data.windows(4).any(|w| w == b"\r\n\r\n") {
                        break String::from_utf8_lossy(&data)
                            .lines()
                            .next()
                            .unwrap()
                            .to_owned();
                    }
                };
                let header_end = data.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                let content_length = String::from_utf8_lossy(&data[..header_end])
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                assert!(content_length <= 32768, "fixture HTTP body limit");
                while data.len() < header_end + content_length {
                    let n = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut chunk))
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(n > 0, "fixture body ended before Content-Length");
                    data.extend_from_slice(&chunk[..n]);
                }
                let body = if route.starts_with("POST /session HTTP/") {
                    r#"{"value":{"sessionId":"fixture-session"}}"#
                } else if route.starts_with("POST /session/fixture-session/appium/settings HTTP/")
                    || route.starts_with("DELETE /session/fixture-session HTTP/")
                {
                    r#"{"value":null}"#
                } else {
                    panic!("unexpected production HTTP route: {route}");
                };
                recorded.lock().push(route);
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        Self {
            base,
            calls,
            stopped: Some(stopped),
            task,
        }
    }
    async fn finish(mut self) -> Vec<String> {
        self.stopped.take().unwrap().send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), &mut self.task)
            .await
            .unwrap()
            .unwrap();
        self.calls.lock().clone()
    }
}

async fn exercise_shutdown(automatic_setup_allowed: bool) -> (Vec<String>, Vec<String>) {
    let adb = FakeAdb::new();
    let server = AppiumFixture::new().await;
    // This is the real client setup against our fake, with no dependency on a
    // mutable observation-mode environment variable or any real server port.
    let agent = AgentClient::connect_with_observation_mode(
        "fixture-owned",
        &server.base,
        AndroidObservationMode::Legacy,
    )
    .await
    .unwrap();
    let driver = AndroidDriver::with_adb(
        AdbProgram::at(adb.executable.clone()),
        adb::AdbOrigin::Configured,
        &AndroidDriverConfig {
            automatic_setup_allowed,
            ..Default::default()
        },
    );
    driver.agents.lock().insert("fixture-owned".into(), agent);
    driver.ports.lock().insert("fixture-owned".into(), 23101);
    driver.forwarded.lock().insert("fixture-owned".into());
    // Allocating/caching a port does NOT make its forward ours. Nothing for this
    // sibling may be terminated or removed, even in ordinary production mode.
    driver
        .ports
        .lock()
        .insert("fixture-not-forwarded".into(), 23102);
    DeviceDriver::shutdown_owned_processes(&driver)
        .await
        .unwrap();
    let calls_after_first = adb.calls();
    DeviceDriver::shutdown_owned_processes(&driver)
        .await
        .unwrap();
    assert_eq!(
        adb.calls(),
        calls_after_first,
        "second shutdown must not replay teardown"
    );
    assert!(driver.agents.lock().is_empty());
    assert!(driver.forwarded.lock().is_empty());
    assert!(driver.ports.lock().is_empty());
    let http = server.finish().await;
    (calls_after_first, http)
}

#[tokio::test]
async fn borrowed_agent_shutdown_preserves_server_and_removes_only_owned_forward() {
    let (adb, http) = exercise_shutdown(false).await;
    assert_eq!(adb, ["-s fixture-owned forward --remove tcp:23101"],
        "borrowed teardown must not force-stop, probe/remove unowned transports, kill-server, or use --remove-all");
    assert_eq!(
        http,
        [
            "POST /session HTTP/1.1",
            "POST /session/fixture-session/appium/settings HTTP/1.1"
        ],
        "borrowed Appium session must never receive DELETE during production shutdown"
    );
}

#[tokio::test]
async fn owned_agent_shutdown_still_deletes_session_stops_both_halves_and_removes_exact_forward() {
    let (adb, http) = exercise_shutdown(true).await;
    assert_eq!(
        http,
        [
            "POST /session HTTP/1.1",
            "POST /session/fixture-session/appium/settings HTTP/1.1",
            "DELETE /session/fixture-session HTTP/1.1"
        ],
        "ordinary production ownership must retain actual AgentClient close"
    );
    assert_eq!(
        adb,
        [
            "-s fixture-owned shell am force-stop io.appium.uiautomator2.server",
            "-s fixture-owned shell am force-stop io.appium.uiautomator2.server.test",
            "-s fixture-owned shell pidof io.appium.uiautomator2.server",
            "-s fixture-owned shell pidof io.appium.uiautomator2.server.test",
            "-s fixture-owned forward --remove tcp:23101",
        ],
        "production teardown must be selective and retain positive process-absence verification"
    );
}
