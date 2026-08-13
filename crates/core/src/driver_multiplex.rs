//! One `DeviceDriver` over several backends, routed per device.
//!
//! A fleet can hold iPhones and Android phones at once, and every layer above
//! this - `DeviceControlPlane`, `DeviceWorkCoordinator`, `StreamBudgetManager`,
//! the registry, the event bus - is written in terms of one driver and an
//! opaque `udid`. Rather than teach each of them about platforms, this presents
//! the several backends as one.
//!
//! Two rules the implementation keeps, both learned the hard way elsewhere in
//! this project:
//!
//! 1. **Routes come from `list_devices` and nothing else.** The udid is never
//!    parsed, sniffed or pattern-matched to guess a platform. That the repo has
//!    no udid format validation anywhere is an asset: an ADB serial and an iOS
//!    UDID are both opaque keys, and they should stay that way.
//! 2. **A backend that fails does not take the others down.** `list_devices`
//!    returns what the healthy backends know plus a per-backend reason for the
//!    one that did not answer, because a phone missing from the grid looks
//!    unplugged rather than undiagnosed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use crate::device_capabilities::{
    AgentInstallProof, ClipboardAccessMode, DeviceCapabilitySnapshot, UiCapabilities,
};
use crate::driver::{
    AppProcessState, DeviceDriver, GuardedClipboardOperation, GuardedClipboardProgress,
    GuardedClipboardTransition, ProcessAbsenceProof, UiSession,
};
use crate::stream_budget::StreamStopProof;
use crate::types::{
    AgentSettings, AgentStatus, DeviceInfo, InteractionSessionKind, StreamHandoffProof,
    StreamStartProof,
};

/// A backend plus how its last listing went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendHealth {
    /// Human-readable name for the operator, e.g. `"ios"` or `"android"`.
    pub name: String,
    /// `None` when the last `list_devices` succeeded.
    pub degraded_reason: Option<String>,
}

struct Backend {
    name: String,
    driver: Arc<dyn DeviceDriver>,
}

pub struct MultiplexDriver {
    backends: Vec<Backend>,
    /// udid -> index into `backends`. Rebuilt from every successful listing.
    routes: RwLock<HashMap<String, usize>>,
    health: RwLock<Vec<BackendHealth>>,
}

impl MultiplexDriver {
    pub fn new(backends: Vec<(String, Arc<dyn DeviceDriver>)>) -> Self {
        let health = backends
            .iter()
            .map(|(name, _)| BackendHealth {
                name: name.clone(),
                degraded_reason: None,
            })
            .collect();
        Self {
            backends: backends
                .into_iter()
                .map(|(name, driver)| Backend { name, driver })
                .collect(),
            routes: RwLock::new(HashMap::new()),
            health: RwLock::new(health),
        }
    }

    /// Per-backend health from the last listing.
    pub fn health(&self) -> Vec<BackendHealth> {
        self.health.read().clone()
    }

    /// Which backend owns this device, if any has claimed it.
    pub fn backend_name(&self, udid: &str) -> Option<String> {
        let index = *self.routes.read().get(udid)?;
        self.backends.get(index).map(|backend| backend.name.clone())
    }

    fn route(&self, udid: &str) -> anyhow::Result<&Arc<dyn DeviceDriver>> {
        let index = self.routes.read().get(udid).copied().ok_or_else(|| {
            // Deliberately the same shape as an unplugged device: a udid we
            // have no route for is one no backend listed.
            anyhow::anyhow!("device is not connected: {udid}")
        })?;
        self.backends
            .get(index)
            .map(|backend| &backend.driver)
            .ok_or_else(|| anyhow::anyhow!("device is not connected: {udid}"))
    }

    /// Route without an error, for the `bool`-returning capability questions.
    fn try_route(&self, udid: &str) -> Option<&Arc<dyn DeviceDriver>> {
        let index = *self.routes.read().get(udid)?;
        self.backends.get(index).map(|backend| &backend.driver)
    }
}

#[async_trait]
impl DeviceDriver for MultiplexDriver {
    async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
        let mut devices = Vec::new();
        let mut routes = HashMap::new();
        let mut health = Vec::with_capacity(self.backends.len());
        let mut failures = 0usize;
        for (index, backend) in self.backends.iter().enumerate() {
            match backend.driver.list_devices().await {
                Ok(listed) => {
                    for device in &listed {
                        routes.insert(device.udid.clone(), index);
                    }
                    devices.extend(listed);
                    health.push(BackendHealth {
                        name: backend.name.clone(),
                        degraded_reason: None,
                    });
                }
                Err(error) => {
                    failures += 1;
                    health.push(BackendHealth {
                        name: backend.name.clone(),
                        degraded_reason: Some(error.to_string()),
                    });
                }
            }
        }
        // Only give up when nothing answered. One sick backend must not hide a
        // healthy one's devices.
        if failures == self.backends.len() && !self.backends.is_empty() {
            let reasons = health
                .iter()
                .filter_map(|entry| {
                    entry
                        .degraded_reason
                        .as_ref()
                        .map(|reason| format!("{}: {reason}", entry.name))
                })
                .collect::<Vec<_>>()
                .join("; ");
            *self.health.write() = health;
            anyhow::bail!("no device backend answered: {reasons}");
        }
        *self.routes.write() = routes;
        *self.health.write() = health;
        Ok(devices)
    }

    fn agent_settings(&self) -> AgentSettings {
        // Settings are one shared configuration, so the first backend owns the
        // canonical copy and `set` fans out to keep them equal.
        self.backends
            .first()
            .map(|backend| backend.driver.agent_settings())
            .unwrap_or_default()
    }

    fn set_agent_settings(&self, settings: AgentSettings) {
        for backend in &self.backends {
            backend.driver.set_agent_settings(settings.clone());
        }
    }

    fn cached_agent_status(&self, udid: &str) -> AgentStatus {
        match self.try_route(udid) {
            Some(driver) => driver.cached_agent_status(udid),
            None => AgentStatus::unknown(udid),
        }
    }

    async fn preflight_agent(&self, udid: &str) -> anyhow::Result<AgentStatus> {
        self.route(udid)?.preflight_agent(udid).await
    }

    async fn repair_agent(&self, udid: &str) -> anyhow::Result<AgentStatus> {
        self.route(udid)?.repair_agent(udid).await
    }

    async fn inspect_interaction_device(
        &self,
        udid: &str,
    ) -> anyhow::Result<DeviceCapabilitySnapshot> {
        self.route(udid)?.inspect_interaction_device(udid).await
    }

    async fn inspect_device_for_target(
        &self,
        udid: &str,
        target_bundle_id: &str,
    ) -> anyhow::Result<DeviceCapabilitySnapshot> {
        self.route(udid)?
            .inspect_device_for_target(udid, target_bundle_id)
            .await
    }

    async fn set_negotiated_interaction_capabilities(
        &self,
        udid: &str,
        capabilities: UiCapabilities,
    ) -> anyhow::Result<()> {
        self.route(udid)?
            .set_negotiated_interaction_capabilities(udid, capabilities)
            .await
    }

    async fn repair_agent_install_only(&self, udid: &str) -> anyhow::Result<AgentInstallProof> {
        self.route(udid)?.repair_agent_install_only(udid).await
    }

    async fn stop_owned_stream(&self, udid: &str) -> anyhow::Result<StreamStopProof> {
        self.route(udid)?.stop_owned_stream(udid).await
    }

    async fn park_owned_stream(&self, udid: &str) -> anyhow::Result<StreamStopProof> {
        self.route(udid)?.park_owned_stream(udid).await
    }

    async fn start_stream_after_session(&self, udid: &str) -> anyhow::Result<StreamStartProof> {
        self.route(udid)?.start_stream_after_session(udid).await
    }

    async fn confirm_interaction_stream_stopped(
        &self,
        udid: &str,
    ) -> anyhow::Result<StreamHandoffProof> {
        self.route(udid)?
            .confirm_interaction_stream_stopped(udid)
            .await
    }

    async fn read_active_app_bundle(&self, udid: &str) -> anyhow::Result<String> {
        self.route(udid)?.read_active_app_bundle(udid).await
    }

    async fn start_interaction_session(
        &self,
        udid: &str,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        self.route(udid)?
            .start_interaction_session(udid, bundle_id, kind)
            .await
    }

    async fn foreground_target_app_and_start_interaction_session(
        &self,
        udid: &str,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        // Forwarded rather than left to the default, because a backend may
        // override it to foreground the target exactly once.
        self.route(udid)?
            .foreground_target_app_and_start_interaction_session(udid, bundle_id, kind)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn guarded_clipboard_transition(
        &self,
        udid: &str,
        agent_bundle_id: &str,
        target_bundle_id: &str,
        final_session_kind: InteractionSessionKind,
        mode: ClipboardAccessMode,
        operation: GuardedClipboardOperation,
        progress: GuardedClipboardProgress,
    ) -> anyhow::Result<GuardedClipboardTransition> {
        self.route(udid)?
            .guarded_clipboard_transition(
                udid,
                agent_bundle_id,
                target_bundle_id,
                final_session_kind,
                mode,
                operation,
                progress,
            )
            .await
    }

    fn supports_text_comments(&self, udid: &str) -> bool {
        self.try_route(udid)
            .is_some_and(|driver| driver.supports_text_comments(udid))
    }

    fn supports_verified_app_termination(&self, udid: &str) -> bool {
        self.try_route(udid)
            .is_some_and(|driver| driver.supports_verified_app_termination(udid))
    }

    fn reports_element_bounds(&self, udid: &str) -> bool {
        self.try_route(udid)
            .is_some_and(|driver| driver.reports_element_bounds(udid))
    }

    async fn resolve_tiktok_package(&self, udid: &str) -> anyhow::Result<String> {
        self.route(udid)?.resolve_tiktok_package(udid).await
    }

    fn supports_push_media(&self, udid: &str) -> bool {
        self.try_route(udid)
            .is_some_and(|driver| driver.supports_push_media(udid))
    }

    fn requires_fresh_text_session(&self, udid: &str) -> bool {
        self.try_route(udid)
            .is_some_and(|driver| driver.requires_fresh_text_session(udid))
    }

    async fn inspect_app_process(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<AppProcessState> {
        self.route(udid)?.inspect_app_process(udid, bundle_id).await
    }

    async fn backup_device(&self, udid: &str, dest: &Path) -> anyhow::Result<()> {
        self.route(udid)?.backup_device(udid, dest).await
    }

    async fn restore_device(&self, udid: &str, src: &Path) -> anyhow::Result<()> {
        self.route(udid)?.restore_device(udid, src).await
    }

    async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo> {
        self.route(udid)?.refresh_device(udid).await
    }

    async fn install_app(&self, udid: &str, path: &Path) -> anyhow::Result<()> {
        self.route(udid)?.install_app(udid, path).await
    }

    async fn stage_publish_media(
        &self,
        udid: &str,
        agent_bundle_id: &str,
        campaign_id: &str,
        source_root: &Path,
    ) -> anyhow::Result<serde_json::Value> {
        self.route(udid)?
            .stage_publish_media(udid, agent_bundle_id, campaign_id, source_root)
            .await
    }

    async fn prepare_publish_media(
        &self,
        udid: &str,
        campaign_id: &str,
        manifest_sha256: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.route(udid)?
            .prepare_publish_media(udid, campaign_id, manifest_sha256)
            .await
    }

    async fn import_publish_media(
        &self,
        udid: &str,
        campaign_id: &str,
        manifest_sha256: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.route(udid)?
            .import_publish_media(udid, campaign_id, manifest_sha256)
            .await
    }

    async fn cleanup_publish_media(
        &self,
        udid: &str,
        import_id: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.route(udid)?
            .cleanup_publish_media(udid, import_id)
            .await
    }

    async fn uninstall_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()> {
        self.route(udid)?.uninstall_app(udid, bundle_id).await
    }

    async fn screenshot(&self, udid: &str, dest: &Path) -> anyhow::Result<PathBuf> {
        self.route(udid)?.screenshot(udid, dest).await
    }

    async fn syslog_tail(&self, udid: &str, lines: usize) -> anyhow::Result<String> {
        self.route(udid)?.syslog_tail(udid, lines).await
    }

    async fn launch_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()> {
        self.route(udid)?.launch_app(udid, bundle_id).await
    }

    async fn terminate_app(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<ProcessAbsenceProof> {
        self.route(udid)?.terminate_app(udid, bundle_id).await
    }

    async fn reboot(&self, udid: &str) -> anyhow::Result<()> {
        self.route(udid)?.reboot(udid).await
    }

    async fn start_ui_session(&self, udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
        self.route(udid)?.start_ui_session(udid).await
    }

    async fn open_control_session(&self, udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
        self.route(udid)?.open_control_session(udid).await
    }

    async fn start_fresh_text_session(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        self.route(udid)?
            .start_fresh_text_session(udid, bundle_id)
            .await
    }

    async fn ui_session_cached(&self, udid: &str) -> bool {
        match self.try_route(udid) {
            Some(driver) => driver.ui_session_cached(udid).await,
            None => false,
        }
    }

    fn invalidate_ui_session(&self, udid: &str) {
        if let Some(driver) = self.try_route(udid) {
            driver.invalidate_ui_session(udid);
        }
    }

    async fn recycle_ui_transport(&self, udid: &str) {
        if let Some(driver) = self.try_route(udid) {
            driver.recycle_ui_transport(udid).await;
        }
    }

    async fn ensure_stream(&self, udid: &str) -> anyhow::Result<String> {
        self.route(udid)?.ensure_stream(udid).await
    }

    async fn prepare_device(&self, udid: &str) -> anyhow::Result<()> {
        self.route(udid)?.prepare_device(udid).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ConnectionKind, DeviceStatus};

    struct StubDriver {
        udids: Vec<String>,
        text_comments: bool,
        fail_listing: bool,
    }

    impl StubDriver {
        fn attached(udids: &[&str], text_comments: bool) -> Arc<dyn DeviceDriver> {
            Arc::new(Self {
                udids: udids.iter().map(|value| value.to_string()).collect(),
                text_comments,
                fail_listing: false,
            })
        }

        fn broken() -> Arc<dyn DeviceDriver> {
            Arc::new(Self {
                udids: Vec::new(),
                text_comments: false,
                fail_listing: true,
            })
        }
    }

    fn device(udid: &str) -> DeviceInfo {
        DeviceInfo {
            udid: udid.to_string(),
            name: udid.to_string(),
            model: String::new(),
            platform: crate::DevicePlatform::Ios,
            os_version: String::new(),
            connection: ConnectionKind::Usb,
            status: DeviceStatus::Connected,
            battery: None,
            wda_ready: false,
            wda_expires_at: None,
            stream_url: None,
            tile_stream_state: Default::default(),
            last_error: None,
        }
    }

    #[async_trait]
    impl DeviceDriver for StubDriver {
        async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
            if self.fail_listing {
                anyhow::bail!("backend is down");
            }
            Ok(self.udids.iter().map(|udid| device(udid)).collect())
        }
        async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo> {
            Ok(device(udid))
        }
        async fn install_app(&self, _udid: &str, _path: &Path) -> anyhow::Result<()> {
            Ok(())
        }
        async fn uninstall_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn screenshot(&self, _udid: &str, dest: &Path) -> anyhow::Result<PathBuf> {
            Ok(dest.to_path_buf())
        }
        async fn syslog_tail(&self, _udid: &str, _lines: usize) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn launch_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn terminate_app(
            &self,
            _udid: &str,
            bundle_id: &str,
        ) -> anyhow::Result<ProcessAbsenceProof> {
            Ok(ProcessAbsenceProof {
                bundle_id: bundle_id.to_string(),
                old_pid: None,
            })
        }
        async fn reboot(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn start_ui_session(&self, _udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
            anyhow::bail!("stub has no session")
        }
        async fn ensure_stream(&self, _udid: &str) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn prepare_device(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn supports_text_comments(&self, _udid: &str) -> bool {
            self.text_comments
        }
    }

    fn multiplex() -> MultiplexDriver {
        MultiplexDriver::new(vec![
            (
                "ios".into(),
                StubDriver::attached(&["ios-a", "ios-b"], false),
            ),
            ("android".into(), StubDriver::attached(&["droid-a"], true)),
        ])
    }

    #[tokio::test]
    async fn listing_joins_the_backends_and_builds_the_routes() {
        let driver = multiplex();
        let devices = driver.list_devices().await.expect("list");
        let udids: Vec<&str> = devices.iter().map(|d| d.udid.as_str()).collect();
        assert_eq!(udids, ["ios-a", "ios-b", "droid-a"]);
        assert_eq!(driver.backend_name("ios-b").as_deref(), Some("ios"));
        assert_eq!(driver.backend_name("droid-a").as_deref(), Some("android"));
        assert_eq!(driver.backend_name("unknown"), None);
    }

    #[tokio::test]
    async fn a_capability_is_answered_by_the_backend_that_owns_the_device() {
        // The whole point: one fleet, two different answers, neither borrowed.
        let driver = multiplex();
        driver.list_devices().await.expect("list");
        assert!(!driver.supports_text_comments("ios-a"));
        assert!(driver.supports_text_comments("droid-a"));
    }

    #[tokio::test]
    async fn an_unrouted_device_never_borrows_another_backends_answer() {
        let driver = multiplex();
        driver.list_devices().await.expect("list");
        assert!(!driver.supports_text_comments("never-listed"));
        let error = driver
            .refresh_device("never-listed")
            .await
            .expect_err("unrouted device must not resolve");
        assert!(error.to_string().contains("not connected"), "{error}");
    }

    #[tokio::test]
    async fn routes_are_built_only_from_listings_never_from_the_udid_string() {
        // Before any listing there is no route, however plausible the udid
        // looks. Guessing a platform from the key is what this must never do.
        let driver = multiplex();
        assert_eq!(driver.backend_name("droid-a"), None);
        assert!(driver.refresh_device("droid-a").await.is_err());
        driver.list_devices().await.expect("list");
        assert_eq!(driver.backend_name("droid-a").as_deref(), Some("android"));
    }

    #[tokio::test]
    async fn one_sick_backend_does_not_hide_the_healthy_one() {
        let driver = MultiplexDriver::new(vec![
            ("ios".into(), StubDriver::broken()),
            ("android".into(), StubDriver::attached(&["droid-a"], true)),
        ]);
        let devices = driver.list_devices().await.expect("list");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].udid, "droid-a");
        let health = driver.health();
        assert_eq!(health[0].name, "ios");
        assert!(health[0].degraded_reason.is_some());
        assert_eq!(health[1].degraded_reason, None);
    }

    #[tokio::test]
    async fn listing_fails_only_when_no_backend_answers() {
        let driver = MultiplexDriver::new(vec![
            ("ios".into(), StubDriver::broken()),
            ("android".into(), StubDriver::broken()),
        ]);
        let error = driver.list_devices().await.expect_err("all backends down");
        let text = error.to_string();
        assert!(text.contains("ios"), "{text}");
        assert!(text.contains("android"), "{text}");
    }

    #[tokio::test]
    async fn a_device_that_disappears_loses_its_route() {
        let driver = MultiplexDriver::new(vec![
            ("ios".into(), StubDriver::attached(&["ios-a"], false)),
            ("android".into(), StubDriver::attached(&[], true)),
        ]);
        driver.list_devices().await.expect("list");
        assert_eq!(driver.backend_name("ios-a").as_deref(), Some("ios"));

        // A second multiplexer standing in for the same fleet after the phone
        // was unplugged: the rebuilt table must not keep the stale entry.
        let driver = MultiplexDriver::new(vec![
            ("ios".into(), StubDriver::attached(&[], false)),
            ("android".into(), StubDriver::attached(&["droid-a"], true)),
        ]);
        driver.list_devices().await.expect("list");
        assert_eq!(driver.backend_name("ios-a"), None);
    }
}
