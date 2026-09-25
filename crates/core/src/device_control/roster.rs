//! What the fleet is: which phones are there, what each can do, and getting one ready.
//!
//! Reads and preparation, with no lease taken and nothing left running.

use super::*;

impl DeviceControlPlane {
    pub async fn verify_automation_readiness(&self, udid: &str) -> Result<(), DeviceControlError> {
        self.driver
            .verify_automation_readiness(udid)
            .await
            .map_err(|error| driver_error(udid, "automationReadiness", error))
    }

    /// The same non-mutating capability read model used by UI and native starts.
    pub async fn tiktok_action_capabilities(
        &self,
        udid: &str,
    ) -> crate::ipc_contract::DeviceActionCapabilities {
        use crate::ipc_contract::CapabilityEvidenceState;
        let (package, version, locale, mut refusal) = match self.tiktok_build(udid).await {
            Ok((package, version, locale)) => (package, version, locale, None),
            Err(error) => (
                String::new(),
                String::new(),
                String::new(),
                Some(error.to_string()),
            ),
        };
        if let Some(owner) = self.current_work_owner(udid) {
            refusal = Some(format!("Thiết bị đang được {owner:?} giữ"));
        } else if refusal.is_none() {
            if let Err(error) = self.verify_automation_readiness(udid).await {
                refusal = Some(error.to_string());
            }
        }
        let mut report = crate::app_automation::action_capabilities(
            udid,
            &package,
            &version,
            &locale,
            refusal.is_none(),
        );
        if let Some(reason) = refusal {
            for action in &mut report.actions {
                action.state = CapabilityEvidenceState::DeviceNotReady;
                action.reason = reason.clone();
            }
        }
        report
    }

    pub async fn preflight_tiktok_actions(
        &self,
        udid: &str,
        actions: &[&str],
    ) -> anyhow::Result<()> {
        let report = self.tiktok_action_capabilities(udid).await;
        crate::app_automation::require_actions(&report, actions)
    }
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
        let preferred = self.selected_app_package(udid, "tiktok").await?;
        self.driver
            .resolve_tiktok_package_with_preference(udid, preferred.as_deref())
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
        let preferred = self.selected_app_package(udid, "tiktok").await?;
        self.driver
            .tiktok_build_with_preference(udid, preferred.as_deref())
            .await
            .map_err(|error| driver_error(udid, "tiktokBuild", error))
    }
    /// Read-only Threads package/build/locale probe used by publish preflight.
    pub async fn threads_build(
        &self,
        udid: &str,
    ) -> Result<(String, String, String), DeviceControlError> {
        self.driver
            .threads_build(udid)
            .await
            .map_err(|error| driver_error(udid, "threadsBuild", error))
    }
    /// Read-only storage check used by publish preflight; it never evicts active work.
    pub async fn available_storage_bytes(&self, udid: &str) -> Result<u64, DeviceControlError> {
        self.driver
            .available_storage_bytes(udid)
            .await
            .map_err(|error| driver_error(udid, "availableStorageBytes", error))
    }
    /// Read-only check for another host competing for the device's UI session.
    pub async fn verify_automation_transport(&self, udid: &str) -> Result<(), DeviceControlError> {
        self.driver
            .verify_automation_transport(udid)
            .await
            .map_err(|error| driver_error(udid, "verifyAutomationTransport", error))
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
