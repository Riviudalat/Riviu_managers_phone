//! What the fleet is: which phones are there, what each can do, and getting one ready.
//!
//! Reads and preparation, with no lease taken and nothing left running.

use super::*;

impl DeviceControlPlane {
    pub fn agent_settings(&self) -> AgentSettings {
        self.driver.agent_settings()
    }
    pub fn set_agent_settings(&self, settings: AgentSettings) {
        self.driver.set_agent_settings(settings);
    }
    pub fn cached_agent_status(&self, udid: &str) -> AgentStatus {
        self.driver.cached_agent_status(udid)
    }
    pub fn supports_text_comments(&self, udid: &str) -> bool {
        self.driver.supports_text_comments(udid)
    }
    /// Pre-flight prediction; the session stays the runtime authority. See
    /// `DeviceDriver::reports_element_bounds`.
    pub fn reports_element_bounds(&self, udid: &str) -> bool {
        self.driver.reports_element_bounds(udid)
    }
    /// Which TikTok build this device can be driven against.
    pub async fn resolve_tiktok_package(&self, udid: &str) -> Result<String, DeviceControlError> {
        self.driver
            .resolve_tiktok_package(udid)
            .await
            .map_err(|error| driver_error(udid, "resolveTikTokPackage", error))
    }
    /// The `(package, versionName, locale)` a label lookup is keyed by — see
    /// [`DeviceDriver::tiktok_build`]. Lease-free for the same reason as the line above:
    /// it reads and changes nothing, and a readiness panel must not be able to evict a
    /// running session.
    pub async fn tiktok_build(
        &self,
        udid: &str,
    ) -> Result<(String, String, String), DeviceControlError> {
        self.driver
            .tiktok_build(udid)
            .await
            .map_err(|error| driver_error(udid, "tiktokBuild", error))
    }
    /// Every app the phone reports as present.
    ///
    /// Lease-free on purpose, following `resolve_tiktok_package` directly above: this
    /// reads and changes nothing, and taking an exclusive lease to answer it would let a
    /// panel refresh evict a running session. The interaction path already relies on that
    /// property — it resolves a package *before* acquiring anything so a phone with no
    /// drivable build refuses without consuming a lease or a capacity slot.
    pub async fn list_installed_apps(
        &self,
        udid: &str,
    ) -> Result<Vec<crate::types::InstalledApp>, DeviceControlError> {
        self.driver
            .list_installed_apps(udid)
            .await
            .map_err(|error| driver_error(udid, "listInstalledApps", error))
    }
    pub fn driver_contract_ids(&self, udid: &str) -> BTreeSet<String> {
        let mut contracts = BTreeSet::new();
        if self.driver.supports_verified_app_termination(udid) {
            contracts.insert("verifiedProcessControl".to_string());
        }
        contracts
    }
    pub fn requires_fresh_text_session(&self, udid: &str) -> bool {
        self.driver.requires_fresh_text_session(udid)
    }
    pub async fn list_devices(&self) -> Result<Vec<DeviceInfo>, DeviceControlError> {
        self.driver
            .list_devices()
            .await
            .map_err(|error| driver_error("fleet", "listDevices", error))
    }
    pub async fn refresh_device(&self, udid: &str) -> Result<DeviceInfo, DeviceControlError> {
        self.driver
            .refresh_device(udid)
            .await
            .map_err(|error| driver_error(udid, "refreshDevice", error))
    }
    pub async fn inspect_interaction_device(
        &self,
        context: &DeviceExclusiveContext,
    ) -> Result<DeviceCapabilitySnapshot, DeviceControlError> {
        let lease = self.validate_exclusive(context)?;
        self.driver
            .set_negotiated_interaction_capabilities(lease.udid(), Default::default())
            .await
            .map_err(|error| {
                driver_error(
                    lease.udid(),
                    "clearNegotiatedInteractionCapabilities",
                    error,
                )
            })?;
        let snapshot = self
            .driver
            .inspect_interaction_device(lease.udid())
            .await
            .map_err(|error| driver_error(lease.udid(), "inspectInteractionDevice", error))?;
        Ok(snapshot)
    }
    pub async fn inspect_flow_device(
        &self,
        context: &DeviceExclusiveContext,
        target_bundle_id: &str,
    ) -> Result<DeviceCapabilitySnapshot, DeviceControlError> {
        let lease = self.validate_exclusive(context)?;
        if target_bundle_id.is_empty() || target_bundle_id.trim() != target_bundle_id {
            return Err(DeviceControlError::InvalidContext {
                reason: "Flow target bundle ID must be non-empty and exact",
            });
        }
        self.driver
            .inspect_device_for_target(lease.udid(), target_bundle_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "inspectFlowDevice", error))
    }
    /// Applies capabilities only from a complete runtime snapshot collected
    /// while this same exclusive context is held. Metadata-only inspection is
    /// intentionally insufficient because it carries neither protected auth
    /// nor live frame geometry proof.
    pub async fn negotiate_interaction_capabilities(
        &self,
        context: &DeviceExclusiveContext,
        snapshot: &DeviceCapabilitySnapshot,
    ) -> Result<DeviceControllerCapabilities, DeviceControlError> {
        let lease = self.validate_exclusive(context)?;
        let negotiated = self.capability_registry.negotiate(snapshot);
        self.driver
            .set_negotiated_interaction_capabilities(lease.udid(), negotiated.ui.clone())
            .await
            .map_err(|error| {
                driver_error(lease.udid(), "setNegotiatedInteractionCapabilities", error)
            })?;
        Ok(negotiated)
    }
    pub async fn repair_agent_install_only(
        &self,
        context: &DeviceExclusiveContext,
    ) -> Result<AgentInstallProof, DeviceControlError> {
        let lease = self.validate_exclusive(context)?;
        self.driver
            .repair_agent_install_only(lease.udid())
            .await
            .map_err(|error| driver_error(lease.udid(), "repairAgentInstallOnly", error))
    }
    pub async fn preflight_agent(
        &self,
        context: &DeviceExclusiveContext,
    ) -> Result<AgentStatus, DeviceControlError> {
        let lease = self.validate_exclusive(context)?;
        self.driver
            .repair_agent_install_only(lease.udid())
            .await
            .map_err(|error| driver_error(lease.udid(), "preflightAgentInstallOnly", error))?;
        Ok(self.driver.cached_agent_status(lease.udid()))
    }
    pub async fn repair_agent(
        &self,
        context: &DeviceExclusiveContext,
    ) -> Result<AgentStatus, DeviceControlError> {
        let lease = self.validate_exclusive(context)?;
        self.driver
            .repair_agent_install_only(lease.udid())
            .await
            .map_err(|error| driver_error(lease.udid(), "repairAgentInstallOnly", error))?;
        Ok(self.driver.cached_agent_status(lease.udid()))
    }
    pub async fn prepare_device(
        &self,
        context: &DeviceExclusiveContext,
    ) -> Result<(), DeviceControlError> {
        let lease = self.validate_exclusive(context)?;
        self.driver
            .prepare_device(lease.udid())
            .await
            .map_err(|error| driver_error(lease.udid(), "prepareDevice", error))
    }
}
