//! Install the branded helper on arrival without opening it, switching IME, or starting a session.
use super::*;

const MIN_LAUNCHER_VERSION: u64 = 5;
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);

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

    /// Merge live probes with rows reserved by package setup, preserving the ADB order.
    /// The probe seam lets regression tests exercise this same path without a device.
    pub(super) async fn inventory_from_reading<F, Fut>(
        &self,
        reading: adb::DeviceListReading,
        mut probe: F,
    ) -> Vec<DeviceInfo>
    where
        F: FnMut(AdbProgram, String, Option<String>) -> Fut,
        Fut: std::future::Future<Output = DeviceInfo> + Send + 'static,
    {
        let lines = reading.devices;
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
            self.helper_setup_errors
                .lock()
                .retain(|serial, _| connected.contains(serial));
        }

        // Fan out: the fleet is 16 phones and every one of them costs a round
        // trip we would otherwise pay in series.
        let mut inflight = Vec::new();
        let mut unreachable_devices = Vec::new();
        let mut cached_devices = Vec::new();
        for line in lines {
            match line.state {
                AdbDeviceState::Device => {
                    let Ok(guard) = self.helper_inventory_lock(&line.serial).try_read_owned()
                    else {
                        let cached = self
                            .inventory_cache
                            .lock()
                            .get(&line.serial)
                            .cloned()
                            .unwrap_or_else(|| {
                                let mut device = unusable_device(
                                    &line.serial,
                                    line.model,
                                    AdbDeviceState::Device,
                                );
                                device.status = DeviceStatus::Connected;
                                device.last_error = None;
                                device
                            });
                        cached_devices.push(cached);
                        continue;
                    };
                    let probe = probe(self.adb.clone(), line.serial, line.model);
                    inflight.push(tokio::spawn(async move { (probe.await, guard) }));
                }
                // **Report it, do not hide it**, and that now covers every state rather
                // than one of them. A phone whose USB-debugging prompt has not been
                // accepted is a normal fleet state with an obvious fix; so is one that has
                // gone `offline` because its cable or hub dropped, or because it is
                // mid-reboot. Dropping those from the list makes them look unplugged, which
                // is the one thing they are not — adb can see them, and it can say why.
                //
                // `offline` in particular was silently discarded, so a phone that lost its
                // connection simply vanished from the grid with no row and no reason.
                state => unreachable_devices.push(unusable_device(&line.serial, line.model, state)),
            }
        }

        let mut devices = Vec::with_capacity(inflight.len() + unreachable_devices.len());
        for handle in inflight {
            let Ok((mut device, _guard)) = handle.await else {
                continue;
            };
            device.wda_ready = self.agent_ready(&device.udid).await;
            if device.wda_ready {
                device.status = DeviceStatus::Ready;
            }
            self.inventory_cache
                .lock()
                .insert(device.udid.clone(), device.clone());
            devices.push(device);
        }
        devices.extend(cached_devices);
        devices.extend(unreachable_devices);
        devices.sort_by_key(|device| {
            serial_order
                .get(&device.udid)
                .copied()
                .unwrap_or(usize::MAX)
        });
        for device in &mut devices {
            if device.last_error.is_none() && connected.contains(&device.udid) {
                device.last_error = self.helper_setup_errors.lock().get(&device.udid).cloned();
            }
        }
        if reading.stable && reading.failure.is_none() {
            *self.helper_inventory_snapshot.lock() = Some(devices.clone());
        }
        devices
    }

    /// The caller owns a Repair lease. Inventory returns the last known row while this
    /// holds the writer, so a slow package install never queues an entire fleet scan.
    pub async fn ensure_helper_installed(&self, serial: &str) -> anyhow::Result<()> {
        let _inventory = self.helper_inventory_lock(serial).write_owned().await;
        let result = prepare_package(&AdbHelperPackage {
            driver: self,
            serial,
        })
        .await;
        let mut errors = self.helper_setup_errors.lock();
        match &result {
            Ok(()) => {
                errors.remove(serial);
            }
            Err(error) => {
                errors.insert(serial.to_owned(), format!("Riviu Helper: {error:#}"));
            }
        }
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
        for first in [HelperPackage::Missing, HelperPackage::Installed(4)] {
            let io = fixture(vec![first, HelperPackage::Installed(5)]);
            prepare_package(&io).await.unwrap();
            assert_eq!(io.installs.load(Ordering::Relaxed), 1);
        }
    }
    #[tokio::test]
    async fn current_and_newer_helpers_are_not_reinstalled() {
        for version in [5, 6] {
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
        let mut io = fixture(vec![HelperPackage::Installed(5)]);
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

    #[tokio::test]
    async fn inventory_during_install_keeps_cached_identity_and_adb_order() {
        let driver = inventory_driver();
        driver
            .inventory_from_reading(inventory_reading(&["a", "b"], true, None), inventory_probe)
            .await;
        let _install_a = driver.helper_inventory_lock("a").write_owned().await;
        let _attach_c = driver.helper_inventory_lock("c").write_owned().await;
        let mut reading = inventory_reading(&["a", "b", "offline", "c"], true, None);
        reading.devices[2].state = AdbDeviceState::Offline;

        let rows = tokio::time::timeout(
            Duration::from_secs(1),
            driver.inventory_from_reading(reading, |adb, serial, model| {
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
    async fn untrusted_partial_inventory_keeps_failure_until_stable_departure() {
        let driver = inventory_driver();
        let failure = "Riviu Helper: INSTALL_FAILED_USER_RESTRICTED";
        driver
            .helper_setup_errors
            .lock()
            .insert("a".into(), failure.into());
        driver
            .inventory_from_reading(inventory_reading(&["a", "b"], true, None), inventory_probe)
            .await;

        for scan_failure in [None, Some("adb server unavailable")] {
            let partial = driver
                .inventory_from_reading(
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
                .inventory_from_reading(inventory_reading(&["a", "b"], true, None), inventory_probe)
                .await;
            assert_eq!(recovered[0].last_error.as_deref(), Some(failure));
        }

        driver
            .inventory_from_reading(inventory_reading(&["b"], true, None), inventory_probe)
            .await;
        assert_eq!(
            serials(&driver.helper_connection_snapshot().unwrap()),
            ["b"]
        );
        assert!(!driver.inventory_cache.lock().contains_key("a"));
        assert!(!driver.helper_setup_errors.lock().contains_key("a"));
        let reconnected = driver
            .inventory_from_reading(inventory_reading(&["a", "b"], true, None), inventory_probe)
            .await;
        assert!(reconnected[0].last_error.is_none());
    }
}
