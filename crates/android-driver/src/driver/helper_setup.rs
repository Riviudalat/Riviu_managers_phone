//! Install the branded helper on arrival without opening it, switching IME, or starting a session.
use super::*;
use futures_util::StreamExt as _;

const MIN_LAUNCHER_VERSION: u64 = 7;
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);
/// How long one phone's model/OS/battery read is reused. r111 (14/10/2026, 28 phones):
/// at 15 s this read was ~110 of the ~160 `adb shell` a minute an idle fleet ran, one per
/// phone every ~15 s; none of the three fields needs that. A failed read still buys no TTL.
const INVENTORY_METADATA_TTL: Duration = Duration::from_secs(60);
/// How many agent `/status` reads one inventory scan runs at once. Chosen, not measured:
/// one read is a loopback HTTP call with a 5 s ceiling, so a fleet of 30 with a few busy
/// agents costs a few ceilings instead of one per phone. r110 (14/10/2026, 28 Samsung):
/// the reads ran one after another and cost ~1.1 s per scan even with every phone cached.
const AGENT_READY_CONCURRENCY: usize = 8;

/// When a fleet scan began and how long it waited for the inventory caller lock, for the
/// one per-scan timing line.
pub(super) struct InventoryScanClock {
    pub(super) started: std::time::Instant,
    pub(super) lock_wait: Duration,
}

#[derive(Debug, PartialEq, Eq)]
enum HelperPackage {
    Missing,
    Installed(u64),
}

#[async_trait]
trait HelperPackageIo: Sync {
    async fn installed(&self) -> anyhow::Result<HelperPackage>;
    async fn install(&self) -> anyhow::Result<()>;
    async fn launcher(&self) -> anyhow::Result<bool>;
}

async fn prepare_package(io: &impl HelperPackageIo) -> anyhow::Result<()> {
    match io.installed().await? {
        HelperPackage::Installed(version) if version >= MIN_LAUNCHER_VERSION => {}
        _ => {
            // One install effect only. A timeout is not permission to install again.
            io.install().await?;
            anyhow::ensure!(
                matches!(io.installed().await?, HelperPackage::Installed(version) if version >= MIN_LAUNCHER_VERSION),
                "Cài Riviu Helper chưa được xác minh: máy chưa báo phiên bản có biểu tượng ứng dụng."
            );
        }
    }
    anyhow::ensure!(
        io.launcher().await?,
        "Riviu Helper đã có nhưng biểu tượng chưa khả dụng. Kiểm tra ứng dụng có bị vô hiệu hóa trên điện thoại không."
    );
    Ok(())
}

struct AdbHelperPackage<'a> {
    driver: &'a AndroidDriver,
    serial: &'a str,
}

fn checked_package_path(output: &adb::ShellOutput) -> anyhow::Result<bool> {
    anyhow::ensure!(
        output.stderr.trim().is_empty(),
        "Đọc Riviu Helper: {}",
        output.stderr.trim()
    );
    if output.stdout.trim().is_empty() && matches!(output.exit_code, 0 | 1) {
        return Ok(false);
    }
    anyhow::ensure!(
        output.exit_code == 0,
        "Đọc Riviu Helper thất bại ({}): {}",
        output.exit_code,
        output.stdout.trim()
    );
    anyhow::ensure!(
        output
            .stdout
            .lines()
            .all(|line| line.trim().starts_with("package:")),
        "Kết quả package Riviu Helper không hợp lệ: {}",
        output.stdout.trim()
    );
    Ok(true)
}

#[async_trait]
impl HelperPackageIo for AdbHelperPackage<'_> {
    async fn installed(&self) -> anyhow::Result<HelperPackage> {
        let package = crate::riviu_agent::PACKAGE;
        let path = self
            .driver
            .adb
            .shell_output(self.serial, &format!("pm path {package}"), QUERY_TIMEOUT)
            .await?;
        if !checked_package_path(&path)? {
            return Ok(HelperPackage::Missing);
        }
        let dump = self
            .driver
            .adb
            .shell_output(
                self.serial,
                &format!("dumpsys package {package}"),
                QUERY_TIMEOUT,
            )
            .await?;
        anyhow::ensure!(
            dump.exit_code == 0 && dump.stderr.trim().is_empty(),
            "Không đọc được phiên bản Riviu Helper: {} {}",
            dump.stdout.trim(),
            dump.stderr.trim()
        );
        let version = parse_package_version(&dump.stdout)
            .version_code
            .and_then(|value| value.parse::<u64>().ok())
            .context("Máy không trả versionCode hợp lệ cho Riviu Helper")?;
        Ok(HelperPackage::Installed(version))
    }

    async fn install(&self) -> anyhow::Result<()> {
        let apk = self
            .driver
            .riviu_agent_apk
            .as_deref()
            .context("Thiếu APK Riviu Helper trong bộ cài. Cài lại Riviu Manager.")?;
        crate::riviu_agent::install_apk(&self.driver.adb, self.serial, apk).await
    }

    async fn launcher(&self) -> anyhow::Result<bool> {
        let output = self.driver.adb.shell_output(self.serial,
            "cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -p com.riviu.agent",
            QUERY_TIMEOUT).await?;
        anyhow::ensure!(
            output.exit_code == 0 && output.stderr.trim().is_empty(),
            "Không kiểm tra được biểu tượng Riviu Helper: {} {}",
            output.stdout.trim(),
            output.stderr.trim()
        );
        Ok(output.stdout.lines().any(|line| {
            let line = line.trim();
            line == "com.riviu.agent/.MainActivity"
                || line == "com.riviu.agent/com.riviu.agent.MainActivity"
        }))
    }
}

impl AndroidDriver {
    /// Only a stable successful Android scan may advance connection generations.
    /// A multiplexed iOS-only result after an Android scan error is not a disconnect.
    pub fn helper_connection_snapshot(&self) -> Option<Vec<DeviceInfo>> {
        self.helper_inventory_snapshot.lock().clone()
    }

    /// Cache one Wi-Fi read and answer the address to show. A failed read is unknown, not
    /// absent: the last address stays until a read answers, and is retried a minute later.
    pub(super) fn record_wifi_ip(
        &self,
        serial: String,
        read: Option<Option<String>>,
    ) -> Option<String> {
        let mut wifi_ips = self.wifi_ips.lock();
        let ip = match read {
            Some(ip) => ip,
            None => wifi_ips.get(&serial).and_then(|(_, ip)| ip.clone()),
        };
        wifi_ips.insert(serial, (std::time::Instant::now(), ip.clone()));
        ip
    }

    /// Merge optional enrichment with fresh roster states under the inventory caller lock.
    pub(super) async fn inventory_from_reading<F, Fut>(
        &self,
        clock: InventoryScanClock,
        reading: adb::DeviceListReading,
        mut probe: F,
    ) -> Vec<DeviceInfo>
    where
        F: FnMut(AdbProgram, String, Option<String>) -> Fut,
        Fut: std::future::Future<Output = DeviceInfo> + Send + 'static,
    {
        let started = std::time::Instant::now();
        let lines = reading.devices;
        // A phone stuck `offline` after a Riviu reboot gets that reboot's one per-device
        // reconnect from here too, also while the automatic workers are frozen (13/10/2026).
        for line in &lines {
            self.observe_adb_state_after_reboot(
                &line.serial,
                line.state == AdbDeviceState::Offline,
            )
            .await;
        }
        let serial_order: HashMap<_, _> = lines
            .iter()
            .enumerate()
            .map(|(index, line)| (line.serial.clone(), index))
            .collect();
        let connected: HashSet<_> = lines
            .iter()
            .filter(|line| line.state == AdbDeviceState::Device)
            .map(|line| line.serial.clone())
            .collect();
        if reading.stable && reading.failure.is_none() {
            self.inventory_cache
                .lock()
                .retain(|serial, _| connected.contains(serial));
            self.inventory_metadata_at
                .lock()
                .retain(|serial, _| connected.contains(serial));
            self.wifi_ips
                .lock()
                .retain(|serial, _| connected.contains(serial));
            // Failure remains visible across disconnect; only qualified setup clears it.
        }

        // Metadata is optional and bounded; every row's connection comes from this
        // reading. Keep parallel cold probes and the existing package-setup reader.
        // JoinSet aborts owned read-only probes if this scan is cancelled.
        // Raw JoinHandles would detach and outlive the inventory caller lock.
        let mut inflight = tokio::task::JoinSet::new();
        // Wi-Fi addresses refresh on their own minute-long clock, beside the probes.
        let mut wifi_reads = tokio::task::JoinSet::new();
        let mut probe_fallbacks = HashMap::new();
        let mut unreachable_devices = Vec::new();
        let mut cached_devices = Vec::new();
        let mut cached = 0usize;
        let mut probed = 0usize;
        let mut setup_busy = 0usize;
        for line in lines {
            match line.state {
                AdbDeviceState::Device => {
                    let wifi_fresh = self
                        .wifi_ips
                        .lock()
                        .get(&line.serial)
                        .is_some_and(|(at, _)| at.elapsed() < WIFI_IP_TTL);
                    if !wifi_fresh {
                        wifi_reads.spawn(read_wifi_ip(self.adb.clone(), line.serial.clone()));
                    }
                    let Ok(guard) = self.helper_inventory_lock(&line.serial).try_read_owned()
                    else {
                        let mut device = self
                            .inventory_cache
                            .lock()
                            .get(&line.serial)
                            .cloned()
                            .unwrap_or_else(|| {
                                let mut device = unusable_device(
                                    &line.serial, line.model, AdbDeviceState::Device,
                                );
                                device.last_error = None;
                                device
                            });
                        // A writer reserves setup, not a ready agent. Never carry a
                        // prior capability/status into this fresh connected row.
                        device.status = DeviceStatus::Connected;
                        device.wda_ready = false;
                        setup_busy += 1;
                        cached_devices.push((device, None));
                        continue;
                    };
                    let fresh = self
                        .inventory_metadata_at
                        .lock()
                        .get(&line.serial)
                        .is_some_and(|at| at.elapsed() < INVENTORY_METADATA_TTL);
                    let known = if fresh {
                        self.inventory_cache.lock().get(&line.serial).cloned()
                    } else {
                        None
                    };
                    if let Some(mut device) = known {
                        device.status = DeviceStatus::Connected;
                        device.wda_ready = false;
                        cached += 1;
                        cached_devices.push((device, Some(guard)));
                        continue;
                    }
                    let mut fallback = unusable_device(
                        &line.serial, line.model.clone(), AdbDeviceState::Device,
                    );
                    fallback.status = DeviceStatus::Error;
                    fallback.last_error = Some("Đọc thông tin máy bị gián đoạn".into());
                    let probe = probe(self.adb.clone(), line.serial, line.model);
                    probed += 1;
                    probe_fallbacks.insert(fallback.udid.clone(), fallback);
                    inflight.spawn(async move {
                        (probe.await, std::time::Instant::now(), guard)
                    });
                }
                // Explicit unreachable states invalidate even an unsettled reading;
                // absence alone needs the stable successful scan above.
                state => {
                    self.inventory_metadata_at.lock().remove(&line.serial);
                    self.inventory_cache.lock().remove(&line.serial);
                    self.wifi_ips.lock().remove(&line.serial);
                    unreachable_devices.push(unusable_device(&line.serial, line.model, state));
                }
            }
        }

        let offline = unreachable_devices.len();
        let mut devices = Vec::with_capacity(
            inflight.len() + cached_devices.len() + unreachable_devices.len(),
        );
        // Rows whose agent is asked once probes settle, each still holding its reader:
        // `Some(may_be_ready)` says whether a reachable agent makes the row Ready.
        let mut readiness = Vec::with_capacity(inflight.len() + cached_devices.len());
        while let Some(result) = inflight.join_next().await {
            let Ok((device, observed_at, guard)) = result else {
                continue;
            };
            probe_fallbacks.remove(&device.udid);
            // Keep raw metadata separate from live readiness and setup errors.
            // Partial or failed enrichment is displayed, but never buys a TTL.
            let metadata_complete = device.last_error.is_none()
                && !device.os_version.is_empty()
                && !device.model.is_empty()
                && device.battery.is_some();
            if metadata_complete {
                self.inventory_metadata_at.lock().insert(device.udid.clone(), observed_at);
            } else {
                self.inventory_metadata_at.lock().remove(&device.udid);
            }
            self.inventory_cache.lock().insert(device.udid.clone(), device.clone());
            readiness.push((device, Some(metadata_complete), Some(guard)));
        }
        for (serial, fallback) in probe_fallbacks {
            self.inventory_metadata_at.lock().remove(&serial);
            self.inventory_cache.lock().remove(&serial);
            devices.push(fallback);
        }
        for (device, guard) in cached_devices {
            // A row whose setup holds the writer is shown as it is, unasked.
            let may_be_ready = guard.is_some().then_some(device.last_error.is_none());
            readiness.push((device, may_be_ready, guard));
        }
        // One slow agent must not hold every phone behind it: the reads overlap, capped, and
        // each row keeps its own answer.
        let mut reads = Vec::with_capacity(readiness.len());
        for (device, may_be_ready, _) in &readiness {
            let udid = device.udid.as_str();
            let ask = may_be_ready.is_some();
            reads.push(async move { ask && self.agent_ready(udid).await });
        }
        let ready = futures_util::stream::iter(reads)
            .buffered(AGENT_READY_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        for ((mut device, may_be_ready, _guard), ready) in readiness.into_iter().zip(ready) {
            if let Some(may_be_ready) = may_be_ready {
                device.wda_ready = ready;
                if ready && may_be_ready {
                    device.status = DeviceStatus::Ready;
                }
            }
            devices.push(device);
        }
        devices.extend(unreachable_devices);
        while let Some(result) = wifi_reads.join_next().await {
            let Ok((serial, read)) = result else {
                continue;
            };
            self.record_wifi_ip(serial, read);
        }
        {
            let wifi_ips = self.wifi_ips.lock();
            for device in &mut devices {
                device.wifi_ip = connected
                    .contains(&device.udid)
                    .then(|| wifi_ips.get(&device.udid).and_then(|(_, ip)| ip.clone()))
                    .flatten();
            }
        }
        devices.sort_by_key(|device| {
            serial_order
                .get(&device.udid)
                .copied()
                .unwrap_or(usize::MAX)
        });
        // Consume failure already observed by an admitted operation. Inventory never
        // waits for helper HTTP/lifecycle locks, and ordinary busy is not failure.
        for device in &devices {
            if !connected.contains(&device.udid) { continue; }
            let Ok(_guard) = self.helper_inventory_lock(&device.udid).try_read_owned() else { continue; };
            let helper = self.helpers.lock().get(&device.udid).cloned();
            if helper.is_some_and(|helper| helper.cached_acquisition_failed()) {
                self.record_helper_runtime_result(&device.udid, false);
            }
        }
        for device in &mut devices {
            if connected.contains(&device.udid) {
                if let Some(error) = self.helper_setup_errors.lock().get(&device.udid).cloned() {
                    device.wda_ready = false;
                    device.status = DeviceStatus::Error;
                    if device.last_error.is_none() { device.last_error = Some(error); }
                }
            }
        }
        if reading.stable && reading.failure.is_none() {
            *self.helper_inventory_snapshot.lock() = Some(devices.clone());
        }
        // One line per scan: the total and the part that took longest.
        let enrichment = started.elapsed();
        let scan = clock.started.elapsed();
        let adb_devices = scan.saturating_sub(clock.lock_wait + enrichment);
        let (largest, _) = [
            ("lock_wait", clock.lock_wait),
            ("adb_devices", adb_devices),
            ("enrichment", enrichment),
        ]
        .into_iter()
        .max_by_key(|(_, elapsed)| *elapsed)
        .unwrap_or(("enrichment", enrichment));
        tracing::info!(
            cached, probed, offline, setup_busy,
            enrichment_elapsed_ms = enrichment.as_millis() as u64,
            lock_wait_ms = clock.lock_wait.as_millis() as u64,
            scan_elapsed_ms = scan.as_millis() as u64,
            largest,
            "Android inventory metadata enrichment"
        );
        devices
    }

    /// The caller owns a Repair lease. Inventory returns the last known row while this
    /// holds the writer, so a slow package install never queues an entire fleet scan.
    pub async fn ensure_helper_installed(&self, serial: &str) -> anyhow::Result<()> {
        let _inventory = self.helper_inventory_lock(serial).write_owned().await;
        let result = async {
            anyhow::ensure!(self.automatic_setup_allowed, "helper setup disabled in diagnostic mode");
            anyhow::ensure!(!self.helpers.lock().contains_key(serial),
                "live helper owner retained; package preparation requires owner settlement");
            crate::riviu_agent::HelperClient::fence_package_preparation(
                &self.adb, serial, self.helper_state_dir.as_deref()).await?;
            prepare_package(&AdbHelperPackage {
            driver: self,
            serial,
        })
            .await
        }.await;
        // Package presence alone is never qualified runtime success.
        if result.is_err() { self.record_helper_runtime_result(serial, false); }
        result
    }

    pub(super) fn helper_inventory_lock(&self, serial: &str) -> Arc<tokio::sync::RwLock<()>> {
        self.helper_inventory_locks
            .lock()
            .entry(serial.to_owned())
            .or_default()
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Fixture {
        reads: Mutex<VecDeque<anyhow::Result<HelperPackage>>>,
        installs: AtomicU32,
        fail_install: bool,
        launcher: bool,
    }
    #[async_trait]
    impl HelperPackageIo for Fixture {
        async fn installed(&self) -> anyhow::Result<HelperPackage> {
            self.reads
                .lock()
                .pop_front()
                .expect("unexpected extra inventory request")
        }
        async fn install(&self) -> anyhow::Result<()> {
            self.installs.fetch_add(1, Ordering::Relaxed);
            anyhow::ensure!(!self.fail_install, "INSTALL_FAILED_USER_RESTRICTED");
            Ok(())
        }
        async fn launcher(&self) -> anyhow::Result<bool> {
            Ok(self.launcher)
        }
    }
    fn fixture(reads: Vec<HelperPackage>) -> Fixture {
        Fixture {
            reads: Mutex::new(reads.into_iter().map(Ok).collect()),
            installs: AtomicU32::new(0),
            fail_install: false,
            launcher: true,
        }
    }

    #[tokio::test]
    async fn installs_missing_and_upgrades_old_helper_once_then_verifies() {
        for first in [HelperPackage::Missing, HelperPackage::Installed(4), HelperPackage::Installed(5), HelperPackage::Installed(6)] {
            let io = fixture(vec![first, HelperPackage::Installed(7)]);
            prepare_package(&io).await.unwrap();
            assert_eq!(io.installs.load(Ordering::Relaxed), 1);
        }
    }
    #[tokio::test]
    async fn current_and_newer_helpers_are_not_reinstalled() {
        for version in [7, 8] {
            let io = fixture(vec![HelperPackage::Installed(version)]);
            prepare_package(&io).await.unwrap();
            assert_eq!(io.installs.load(Ordering::Relaxed), 0);
        }
    }
    #[tokio::test]
    async fn failed_read_is_not_absence_and_never_installs() {
        let io = fixture(vec![]);
        io.reads
            .lock()
            .push_back(Err(anyhow!("device unauthorized")));
        assert!(prepare_package(&io).await.is_err());
        assert_eq!(io.installs.load(Ordering::Relaxed), 0);
    }
    #[tokio::test]
    async fn rejected_install_is_not_retried() {
        let mut io = fixture(vec![HelperPackage::Missing]);
        io.fail_install = true;
        assert!(prepare_package(&io)
            .await
            .unwrap_err()
            .to_string()
            .contains("USER_RESTRICTED"));
        assert_eq!(io.installs.load(Ordering::Relaxed), 1);
    }
    #[tokio::test]
    async fn install_needs_readback_and_a_launcher() {
        let io = fixture(vec![HelperPackage::Missing, HelperPackage::Installed(4)]);
        assert!(prepare_package(&io).await.is_err());
        let mut io = fixture(vec![HelperPackage::Installed(7)]);
        io.launcher = false;
        assert!(prepare_package(&io).await.is_err());
        assert_eq!(io.installs.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn package_path_refusals_are_not_missing_packages() {
        let output = |exit_code, stdout: &str, stderr: &str| adb::ShellOutput {
            exit_code,
            stdout: stdout.into(),
            stderr: stderr.into(),
        };
        assert!(!checked_package_path(&output(1, "", "")).unwrap());
        assert!(checked_package_path(&output(0, "package:/data/app/base.apk\n", "")).unwrap());
        assert!(checked_package_path(&output(1, "", "device unauthorized")).is_err());
        assert!(
            checked_package_path(&output(0, "Error: package manager unavailable", "")).is_err()
        );
    }

    fn inventory_driver() -> AndroidDriver {
        AndroidDriver::with_adb(
            AdbProgram::unrunnable_for_test(PathBuf::from("missing-test-adb")),
            adb::AdbOrigin::Path,
            &AndroidDriverConfig::default(),
        )
    }

    fn inventory_reading(
        serials: &[&str],
        stable: bool,
        failure: Option<&str>,
    ) -> adb::DeviceListReading {
        adb::DeviceListReading {
            devices: serials
                .iter()
                .map(|serial| adb::AdbDeviceLine {
                    serial: (*serial).to_owned(),
                    state: AdbDeviceState::Device,
                    model: Some("fixture model".into()),
                })
                .collect(),
            stable,
            attempts: 2,
            failure: failure.map(str::to_owned),
        }
    }

    async fn inventory_probe(
        _adb: AdbProgram,
        serial: String,
        model: Option<String>,
    ) -> DeviceInfo {
        let mut row = unusable_device(&serial, model, AdbDeviceState::Device);
        row.name = format!("Phone {serial}");
        row.status = DeviceStatus::Connected;
        row.last_error = None;
        row.battery = Some(74);
        row
    }

    fn serials(rows: &[DeviceInfo]) -> Vec<&str> {
        rows.iter().map(|row| row.udid.as_str()).collect()
    }

    fn scan_clock() -> InventoryScanClock {
        InventoryScanClock {
            started: std::time::Instant::now(),
            lock_wait: Duration::ZERO,
        }
    }

    #[tokio::test]
    async fn slow_agents_are_asked_side_by_side_not_one_after_another() {
        // Every phone's forwarded agent answers `/status` only after 300 ms, as a busy
        // agent does while a session holds it.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => request.extend_from_slice(&chunk[..read]),
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                        )
                        .await;
                });
            }
        });
        let driver = inventory_driver();
        let phones = ["a", "b", "c", "d", "e", "f"];
        for serial in phones {
            driver.ports.lock().insert(serial.into(), port);
            driver.forwarded.lock().insert(serial.into());
        }

        let started = std::time::Instant::now();
        let rows = driver
            .inventory_from_reading(
                scan_clock(),
                inventory_reading(&phones, true, None),
                inventory_probe,
            )
            .await;
        let elapsed = started.elapsed();

        assert_eq!(serials(&rows), phones);
        assert!(
            rows.iter().all(|row| row.wda_ready),
            "every phone asked its own agent"
        );
        // One after another this is 6 x 300 ms; side by side it is one slow answer.
        assert!(
            elapsed < Duration::from_millis(1_200),
            "scan took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn inventory_during_install_keeps_cached_identity_and_adb_order() {
        let driver = inventory_driver();
        driver
            .inventory_from_reading(
                scan_clock(),
                inventory_reading(&["a", "b"], true, None),
                inventory_probe,
            )
            .await;
        let _install_a = driver.helper_inventory_lock("a").write_owned().await;
        let _attach_c = driver.helper_inventory_lock("c").write_owned().await;
        let mut reading = inventory_reading(&["a", "b", "offline", "c"], true, None);
        reading.devices[2].state = AdbDeviceState::Offline;

        let rows = tokio::time::timeout(
            Duration::from_secs(1),
            driver.inventory_from_reading(scan_clock(), reading, |adb, serial, model| {
                assert_eq!(
                    serial, "b",
                    "installing and unreachable phones must not be probed"
                );
                inventory_probe(adb, serial, model)
            }),
        )
        .await
        .expect("a held install writer must not block the inventory scan");

        assert_eq!(serials(&rows), ["a", "b", "offline", "c"]);
        assert_eq!(rows[0].name, "Phone a");
        assert_eq!(rows[0].battery, Some(74));
        assert_eq!(rows[2].status, DeviceStatus::Disconnected);
        assert_eq!(rows[3].status, DeviceStatus::Connected);
        assert!(rows[3].last_error.is_none());
        assert_eq!(
            serials(&driver.helper_connection_snapshot().unwrap()),
            ["a", "b", "offline", "c"]
        );
    }

    #[tokio::test]
    async fn untrusted_partial_inventory_keeps_failure_across_reconnection() {
        let driver = inventory_driver();
        let failure = "Riviu Helper: INSTALL_FAILED_USER_RESTRICTED";
        driver
            .helper_setup_errors
            .lock()
            .insert("a".into(), failure.into());
        driver
            .inventory_from_reading(
                scan_clock(),
                inventory_reading(&["a", "b"], true, None),
                inventory_probe,
            )
            .await;

        for scan_failure in [None, Some("adb server unavailable")] {
            let partial = driver
                .inventory_from_reading(
                    scan_clock(),
                    inventory_reading(&["b"], false, scan_failure),
                    inventory_probe,
                )
                .await;
            assert_eq!(serials(&partial), ["b"]);
            let connections = driver.helper_connection_snapshot().unwrap();
            assert_eq!(serials(&connections), ["a", "b"]);
            assert_eq!(connections[0].last_error.as_deref(), Some(failure));
            assert!(driver.inventory_cache.lock().contains_key("a"));

            let recovered = driver
                .inventory_from_reading(
                    scan_clock(),
                    inventory_reading(&["a", "b"], true, None),
                    inventory_probe,
                )
                .await;
            assert_eq!(recovered[0].last_error.as_deref(), Some(failure));
        }

        driver
            .inventory_from_reading(
                scan_clock(),
                inventory_reading(&["b"], true, None),
                inventory_probe,
            )
            .await;
        assert_eq!(
            serials(&driver.helper_connection_snapshot().unwrap()),
            ["b"]
        );
        assert!(!driver.inventory_cache.lock().contains_key("a"));
        assert!(driver.helper_setup_errors.lock().contains_key("a"));
        let reconnected = driver
            .inventory_from_reading(
                scan_clock(),
                inventory_reading(&["a", "b"], true, None),
                inventory_probe,
            )
            .await;
        assert_eq!(reconnected[0].last_error.as_deref(), Some(failure));
        assert_eq!(reconnected[0].status, DeviceStatus::Error);
        assert!(!reconnected[0].wda_ready);
    }
}
