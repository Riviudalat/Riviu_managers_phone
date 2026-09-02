//! Acting on a phone under an exclusive lease: its apps, its media, its screen, its power.

use super::*;

impl DeviceControlPlane {
    pub async fn install_app<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        path: &Path,
    ) -> Result<(), DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .install_app(lease.udid(), path)
            .await
            .map_err(|error| driver_error(lease.udid(), "installApp", error))
    }
    pub async fn install_app_set<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        paths: &[PathBuf],
    ) -> Result<(), DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .install_app_set(lease.udid(), paths)
            .await
            .map_err(|error| driver_error(lease.udid(), "installAppSet", error))
    }
    pub async fn android_install_device_spec<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
    ) -> Result<AndroidInstallDeviceSpec, DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .android_install_device_spec(lease.udid())
            .await
            .map_err(|error| driver_error(lease.udid(), "androidInstallDeviceSpec", error))
    }
    pub async fn extract_app_container_for_spec<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        path: &Path,
        spec: &AndroidInstallDeviceSpec,
        destination: &Path,
    ) -> Result<Vec<PathBuf>, DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .extract_app_container_for_spec(lease.udid(), path, spec, destination)
            .await
            .map_err(|error| driver_error(lease.udid(), "extractAppContainerForSpec", error))
    }
    pub async fn install_app_set_checked<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        request: &DeviceAppInstallRequest,
    ) -> Result<AppInstallResult, DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .install_app_set_checked(lease.udid(), request)
            .await
            .map_err(|error| driver_error(lease.udid(), "installAppSetChecked", error))
    }
    pub async fn install_app_container<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        path: &Path,
    ) -> Result<(), DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .install_app_container(lease.udid(), path)
            .await
            .map_err(|error| driver_error(lease.udid(), "installAppContainer", error))
    }
    pub async fn stage_publish_media<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        agent_bundle_id: &str,
        campaign_id: &str,
        source_root: &Path,
    ) -> Result<serde_json::Value, DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .stage_publish_media(lease.udid(), agent_bundle_id, campaign_id, source_root)
            .await
            .map_err(|error| driver_error(lease.udid(), "stagePublishMedia", error))
    }
    pub fn supports_push_media(&self, udid: &str) -> bool {
        self.driver.supports_push_media(udid)
    }
    pub async fn prepare_publish_media<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        campaign_id: &str,
        manifest_sha256: &str,
    ) -> Result<serde_json::Value, DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .prepare_publish_media(lease.udid(), campaign_id, manifest_sha256)
            .await
            .map_err(|error| driver_error(lease.udid(), "preparePublishMedia", error))
    }
    pub async fn import_publish_media<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        campaign_id: &str,
        manifest_sha256: &str,
    ) -> Result<serde_json::Value, DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .import_publish_media(lease.udid(), campaign_id, manifest_sha256)
            .await
            .map_err(|error| driver_error(lease.udid(), "importPublishMedia", error))
    }
    pub async fn cleanup_publish_media(
        &self,
        context: &DeviceExclusiveContext,
        import_id: &str,
    ) -> Result<serde_json::Value, DeviceControlError> {
        let lease = self.validate_exclusive(context)?;
        self.driver
            .cleanup_publish_media(lease.udid(), import_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "cleanupPublishMedia", error))
    }
    /// Remove native publish assets while the foreground UI context still owns
    /// the live Agent relay. Closing the stream first can make the sidecar
    /// recycle the XCTest process before this sessionless route is sent.
    pub async fn cleanup_publish_media_with_ui(
        &self,
        context: &UiWithStreamContext,
        import_id: &str,
    ) -> Result<serde_json::Value, DeviceControlError> {
        let lease = self.validate_stream(context)?;
        self.driver
            .cleanup_publish_media(lease.udid(), import_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "cleanupPublishMedia", error))
    }
    pub async fn uninstall_app<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        bundle_id: &str,
    ) -> Result<(), DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .uninstall_app(lease.udid(), bundle_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "uninstallApp", error))
    }
    /// Start one app on a leased phone, because an operator asked for it.
    ///
    /// The counterpart of [`Self::uninstall_app`] and deliberately *not* of
    /// [`Self::foreground_target_app`], which takes an exclusive context because it is a step
    /// of the interaction sequence and has a foreground *proof* to keep. This one is a menu
    /// row: it starts the app and reports whether the start command was accepted, nothing
    /// more. Promising a foreground proof here would be promising something no caller checks.
    pub async fn launch_app<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        bundle_id: &str,
    ) -> Result<(), DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .launch_app(lease.udid(), bundle_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "launchApp", error))
    }
    pub async fn screenshot<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        destination: &Path,
    ) -> Result<PathBuf, DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .screenshot(lease.udid(), destination)
            .await
            .map_err(|error| driver_error(lease.udid(), "screenshot", error))
    }
    pub async fn syslog_tail<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        lines: usize,
    ) -> Result<String, DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .syslog_tail(lease.udid(), lines)
            .await
            .map_err(|error| driver_error(lease.udid(), "syslogTail", error))
    }
    /// Run one operator-typed shell script on the device.
    ///
    /// Behind an exclusive lease on purpose, unlike the read-only queries above. An
    /// arbitrary script can reboot the phone, kill the app a session is driving, or
    /// change a setting under it — so it must not be possible to fire one at a device
    /// another piece of work is holding.
    pub async fn device_shell<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        script: &str,
    ) -> Result<crate::types::ShellOutcome, DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .device_shell(lease.udid(), script)
            .await
            .map_err(|error| driver_error(lease.udid(), "deviceShell", error))
    }
    /// Ask for a rotation and report what the device actually settled at.
    pub async fn set_screen_rotation<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        rotation: u8,
    ) -> Result<u8, DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .set_screen_rotation(lease.udid(), rotation)
            .await
            .map_err(|error| driver_error(lease.udid(), "setScreenRotation", error))
    }
    /// Copy the phone's photos and videos onto this host.
    ///
    /// Takes a lease like every other device action, but the caller is expected to have used
    /// the keeping-stream variant: an export can run for minutes on a full camera roll, and
    /// parking the operator's live tile for that long — while they watch — is the behaviour
    /// `device_shell` and `set_screen_rotation` were both deliberately moved off.
    pub async fn pull_media<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        dest_dir: &std::path::Path,
    ) -> Result<crate::driver::MediaPullReport, DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .pull_media(lease.udid(), dest_dir)
            .await
            .map_err(|error| driver_error(lease.udid(), "pullMedia", error))
    }
    pub async fn reboot<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
    ) -> Result<(), DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .reboot(lease.udid())
            .await
            .map_err(|error| driver_error(lease.udid(), "reboot", error))
    }
    pub async fn backup_device<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        dest: &std::path::Path,
    ) -> Result<(), DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .backup_device(lease.udid(), dest)
            .await
            .map_err(|error| driver_error(lease.udid(), "backupDevice", error))
    }
    pub async fn restore_device<'a>(
        &self,
        context: impl Into<DeviceLeaseRef<'a>>,
        src: &std::path::Path,
    ) -> Result<(), DeviceControlError> {
        let lease = self.validate_leased(context.into())?;
        self.driver
            .restore_device(lease.udid(), src)
            .await
            .map_err(|error| driver_error(lease.udid(), "restoreDevice", error))
    }
    pub async fn terminate_app(
        &self,
        context: &DeviceExclusiveContext,
        bundle_id: &str,
    ) -> Result<ProcessAbsenceProof, DeviceControlError> {
        let lease = self.validate_exclusive(context)?;
        self.driver
            .terminate_app(lease.udid(), bundle_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "terminateApp", error))
    }
    pub async fn inspect_app_process(
        &self,
        context: &DeviceExclusiveContext,
        bundle_id: &str,
    ) -> Result<AppProcessState, DeviceControlError> {
        let lease = self.validate_exclusive(context)?;
        self.driver
            .inspect_app_process(lease.udid(), bundle_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "inspectAppProcess", error))
    }
}
