use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use serde::Deserialize;

const MAX_SCOPE_BYTES: u64 = 64 * 1024;
const MAX_SCOPE_DEVICES: usize = 100;
const MAX_SCOPE_CAMPAIGNS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AcceptanceCapability {
    PublishVerification,
    SheetDelivery,
    PublishSchedule,
    PublishCleanup,
}

#[derive(Clone, Debug)]
pub(crate) struct DevAcceptancePolicy {
    active: bool,
    scope_path: Option<PathBuf>,
    activation_id: Option<Arc<str>>,
    approved_device_ids: Option<Arc<[String]>>,
    pinned_campaign_ids: Arc<OnceLock<Vec<String>>>,
    pinned_capabilities: Arc<OnceLock<(bool, bool, bool, bool)>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AcceptanceScope {
    activation_id: String,
    campaign_ids: Vec<String>,
    #[serde(alias = "udids")]
    device_ids: Vec<String>,
    #[serde(default)]
    capabilities: AcceptanceCapabilities,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AcceptanceCapabilities {
    #[serde(default)]
    publish_verification: bool,
    #[serde(default)]
    sheet_delivery: bool,
    #[serde(default)]
    publish_schedule: bool,
    #[serde(default)]
    publish_cleanup: bool,
}

impl DevAcceptancePolicy {
    pub(crate) fn from_process() -> Self {
        let active = cfg!(debug_assertions)
            && std::env::var("RIVIU_DEV_MANUAL_ACCEPTANCE").as_deref() == Ok("1");
        let scope_path = std::env::var_os("RIVIU_DEV_ACCEPTANCE_SCOPE").map(PathBuf::from);
        let activation_id = std::env::var("RIVIU_DEV_ACCEPTANCE_ACTIVATION")
            .ok()
            .filter(|value| valid_id(value))
            .map(Arc::<str>::from);
        Self::from_parts(active, scope_path, activation_id)
    }

    pub(crate) fn from_parts(
        active: bool,
        scope_path: Option<PathBuf>,
        activation_id: Option<Arc<str>>,
    ) -> Self {
        let pinned_campaign_ids = Arc::new(OnceLock::new());
        let pinned_capabilities = Arc::new(OnceLock::new());
        let approved_device_ids = active
            .then(|| scope_path.as_deref().and_then(read_scope_file))
            .flatten()
            .filter(|scope| {
                activation_id.as_deref() == Some(scope.activation_id.as_str())
                    && valid_ids(&scope.device_ids, MAX_SCOPE_DEVICES)
                    && (if scope.campaign_ids.is_empty() {
                        !scope.capabilities.publish_verification
                            && !scope.capabilities.sheet_delivery
                            && !scope.capabilities.publish_schedule
                            && !scope.capabilities.publish_cleanup
                    } else {
                        valid_ids(&scope.campaign_ids, MAX_SCOPE_CAMPAIGNS)
                    })
            })
            .map(|scope| {
                if !scope.campaign_ids.is_empty() {
                    let _ = pinned_campaign_ids.set(scope.campaign_ids.clone());
                    let _ = pinned_capabilities.set((
                        scope.capabilities.publish_verification,
                        scope.capabilities.sheet_delivery,
                        scope.capabilities.publish_schedule,
                        scope.capabilities.publish_cleanup,
                    ));
                }
                Arc::<[String]>::from(scope.device_ids)
            });
        Self {
            active,
            scope_path,
            activation_id,
            approved_device_ids,
            pinned_campaign_ids,
            pinned_capabilities,
        }
    }

    pub(crate) fn active(&self) -> bool {
        self.active
    }

    pub(crate) fn schedules_frozen(&self) -> bool {
        self.active
    }

    pub(crate) fn automatic_device_workers_frozen(&self) -> bool {
        self.active
    }

    pub(crate) fn allows_publish_dispatch(&self, campaign_id: &str, udid: &str) -> bool {
        !self.active || self.scope_contains(campaign_id, udid, |_| true)
    }

    pub(crate) fn allows(
        &self,
        capability: AcceptanceCapability,
        campaign_id: &str,
        udid: &str,
    ) -> bool {
        !self.active
            || self.scope_contains(campaign_id, udid, |capabilities| match capability {
                AcceptanceCapability::PublishVerification => capabilities.publish_verification,
                AcceptanceCapability::SheetDelivery => capabilities.sheet_delivery,
                AcceptanceCapability::PublishSchedule => capabilities.publish_schedule,
                AcceptanceCapability::PublishCleanup => capabilities.publish_cleanup,
            })
    }

    pub(crate) fn scoped_campaign_ids(&self) -> Vec<String> {
        if !self.active {
            return Vec::new();
        }
        self.read_scope()
            .map(|scope| scope.campaign_ids)
            .unwrap_or_default()
    }

    pub(crate) fn allows_any(&self, capability: AcceptanceCapability) -> bool {
        if !self.active {
            return true;
        }
        self.read_scope().is_some_and(|scope| {
            !scope.campaign_ids.is_empty()
                && !scope.device_ids.is_empty()
                && match capability {
                    AcceptanceCapability::PublishVerification => {
                        scope.capabilities.publish_verification
                    }
                    AcceptanceCapability::SheetDelivery => scope.capabilities.sheet_delivery,
                    AcceptanceCapability::PublishSchedule => scope.capabilities.publish_schedule,
                    AcceptanceCapability::PublishCleanup => scope.capabilities.publish_cleanup,
                }
        })
    }

    fn scope_contains(
        &self,
        campaign_id: &str,
        udid: &str,
        capability: impl FnOnce(&AcceptanceCapabilities) -> bool,
    ) -> bool {
        let Some(scope) = self.read_scope() else {
            return false;
        };
        scope.campaign_ids.iter().any(|id| id == campaign_id)
            && scope.device_ids.iter().any(|id| id == udid)
            && capability(&scope.capabilities)
    }

    fn read_scope(&self) -> Option<AcceptanceScope> {
        let scope = read_scope_file(self.scope_path.as_deref()?)?;
        if self.activation_id.as_deref() != Some(scope.activation_id.as_str())
            || self.approved_device_ids.as_deref() != Some(scope.device_ids.as_slice())
            || !valid_ids(&scope.campaign_ids, MAX_SCOPE_CAMPAIGNS)
            || !valid_ids(&scope.device_ids, MAX_SCOPE_DEVICES)
        {
            return None;
        }
        let campaign_ids = &scope.campaign_ids;
        if let Some(pinned) = self.pinned_campaign_ids.get() {
            if pinned != campaign_ids {
                return None;
            }
        } else {
            let _ = self.pinned_campaign_ids.set(campaign_ids.clone());
            if self.pinned_campaign_ids.get() != Some(campaign_ids) {
                return None;
            }
        }
        let capabilities = (
            scope.capabilities.publish_verification,
            scope.capabilities.sheet_delivery,
            scope.capabilities.publish_schedule,
            scope.capabilities.publish_cleanup,
        );
        if let Some(pinned) = self.pinned_capabilities.get() {
            if *pinned != capabilities {
                return None;
            }
        } else {
            let _ = self.pinned_capabilities.set(capabilities);
            if self.pinned_capabilities.get() != Some(&capabilities) {
                return None;
            }
        }
        Some(scope)
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n'))
}

fn valid_ids(ids: &[String], maximum: usize) -> bool {
    !ids.is_empty()
        && ids.len() <= maximum
        && ids.iter().collect::<std::collections::HashSet<_>>().len() == ids.len()
        && ids.iter().all(|id| valid_id(id))
}

fn read_scope_file(path: &Path) -> Option<AcceptanceScope> {
    if !path.is_absolute() {
        return None;
    }
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_SCOPE_BYTES
    {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_SCOPE: AtomicU64 = AtomicU64::new(1);
    const ACTIVATION: &str = "fixture-activation";

    struct ScopeFile(PathBuf);

    impl Drop for ScopeFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.parent().expect("scope parent"));
        }
    }

    fn write_scope(mut value: serde_json::Value) -> ScopeFile {
        let directory = std::env::temp_dir().join(format!(
            "riviu-acceptance-policy-{}-{}",
            std::process::id(),
            NEXT_SCOPE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).expect("scope tempdir");
        let file = directory.join("scope.json");
        value
            .as_object_mut()
            .expect("scope object")
            .entry("activationId")
            .or_insert_with(|| serde_json::json!(ACTIVATION));
        std::fs::write(&file, serde_json::to_vec(&value).expect("scope json")).expect("scope file");
        ScopeFile(file)
    }

    fn active(path: Option<PathBuf>) -> DevAcceptancePolicy {
        let approved_device_ids = path
            .as_deref()
            .and_then(read_scope_file)
            .map(|scope| Arc::<[String]>::from(scope.device_ids));
        DevAcceptancePolicy {
            active: true,
            scope_path: path,
            activation_id: Some(Arc::from(ACTIVATION)),
            approved_device_ids,
            pinned_campaign_ids: Arc::new(OnceLock::new()),
            pinned_capabilities: Arc::new(OnceLock::new()),
        }
    }

    #[test]
    fn manual_acceptance_without_a_valid_scope_fails_closed() {
        let policy = active(None);
        assert!(policy.schedules_frozen());
        assert!(policy.automatic_device_workers_frozen());
        assert!(!policy.allows_publish_dispatch("campaign-a", "phone-a"));
        assert!(!policy.allows(AcceptanceCapability::SheetDelivery, "campaign-a", "phone-a"));
        assert!(!policy.allows(
            AcceptanceCapability::PublishCleanup,
            "campaign-a",
            "phone-a"
        ));
    }

    #[test]
    fn exact_campaign_and_device_scope_only_opens_publish_dispatch() {
        let directory = write_scope(serde_json::json!({
            "campaignIds": ["campaign-a"],
            "deviceIds": ["phone-a"]
        }));
        let policy = active(Some(directory.0.clone()));

        assert!(policy.allows_publish_dispatch("campaign-a", "phone-a"));
        assert!(!policy.allows_publish_dispatch("campaign-a", "phone-b"));
        assert!(!policy.allows_publish_dispatch("campaign-b", "phone-a"));
        assert!(!policy.allows(
            AcceptanceCapability::PublishVerification,
            "campaign-a",
            "phone-a"
        ));
        assert!(!policy.allows(AcceptanceCapability::SheetDelivery, "campaign-a", "phone-a"));
    }

    #[test]
    fn remote_workers_require_their_own_capability_and_exact_scope() {
        let directory = write_scope(serde_json::json!({
            "campaignIds": ["campaign-a"],
            "deviceIds": ["phone-a"],
            "capabilities": {
                "publishVerification": true,
                "sheetDelivery": true
            }
        }));
        let policy = active(Some(directory.0.clone()));

        assert!(policy.allows(
            AcceptanceCapability::PublishVerification,
            "campaign-a",
            "phone-a"
        ));
        assert!(policy.allows(AcceptanceCapability::SheetDelivery, "campaign-a", "phone-a"));
        assert!(!policy.allows(AcceptanceCapability::SheetDelivery, "campaign-a", "phone-b"));
        assert!(!policy.allows(
            AcceptanceCapability::PublishCleanup,
            "campaign-a",
            "phone-a"
        ));
    }

    #[test]
    fn verified_cleanup_requires_a_separate_pinned_capability() {
        let scope = write_scope(serde_json::json!({
            "campaignIds": ["campaign-a"], "deviceIds": ["phone-a"],
            "capabilities": {"publishCleanup": true}
        }));
        let policy = active(Some(scope.0.clone()));
        assert!(policy.allows(
            AcceptanceCapability::PublishCleanup,
            "campaign-a",
            "phone-a"
        ));
        assert!(!policy.allows(
            AcceptanceCapability::PublishCleanup,
            "campaign-a",
            "phone-b"
        ));
        assert!(!policy.allows(
            AcceptanceCapability::PublishCleanup,
            "campaign-b",
            "phone-a"
        ));
        std::fs::write(
            &scope.0,
            serde_json::to_vec(&serde_json::json!({
                "activationId": ACTIVATION,
                "campaignIds": ["campaign-a"], "deviceIds": ["phone-a"],
                "capabilities": {"publishCleanup": false}
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(!policy.allows(
            AcceptanceCapability::PublishCleanup,
            "campaign-a",
            "phone-a"
        ));
    }

    #[test]
    fn frozen_fleet_scope_allows_only_named_campaigns_and_explicit_schedules() {
        let devices: Vec<_> = (0..30).map(|n| format!("phone-{n}")).collect();
        let scope = write_scope(serde_json::json!({
            "campaignIds": ["batch-a", "batch-b"], "deviceIds": devices,
            "capabilities": {"publishVerification": true, "sheetDelivery": true, "publishSchedule": true}
        }));
        let policy = DevAcceptancePolicy::from_parts(
            true,
            Some(scope.0.clone()),
            Some(Arc::from(ACTIVATION)),
        );
        assert!(
            policy.schedules_frozen(),
            "Unrelated schedules remain frozen"
        );
        for campaign in ["batch-a", "batch-b"] {
            for device in &devices {
                assert!(policy.allows_publish_dispatch(campaign, device));
                assert!(policy.allows(AcceptanceCapability::PublishSchedule, campaign, device));
            }
        }
        assert!(!policy.allows(
            AcceptanceCapability::PublishSchedule,
            "old-schedule",
            "phone-0"
        ));
        assert!(!policy.allows_publish_dispatch("batch-a", "phone-30"));
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&scope.0).unwrap()).unwrap();
        value["campaignIds"]
            .as_array_mut()
            .unwrap()
            .push("late-campaign".into());
        std::fs::write(&scope.0, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(!policy.allows_publish_dispatch("late-campaign", "phone-0"));
        assert!(!policy.allows_publish_dispatch("batch-a", "phone-0"));
    }

    #[test]
    fn dispatch_and_sheet_permission_do_not_enable_scheduled_posting() {
        let scope = write_scope(serde_json::json!({
            "campaignIds": ["batch"], "deviceIds": ["phone"],
            "capabilities": {"publishVerification": true, "sheetDelivery": true}
        }));
        let policy = active(Some(scope.0.clone()));
        assert!(policy.allows_publish_dispatch("batch", "phone"));
        assert!(!policy.allows_any(AcceptanceCapability::PublishSchedule));
        assert!(!policy.allows(AcceptanceCapability::PublishSchedule, "batch", "phone"));
    }

    #[test]
    fn production_policy_keeps_existing_background_behaviour() {
        let policy = DevAcceptancePolicy {
            active: false,
            scope_path: None,
            activation_id: None,
            approved_device_ids: None,
            pinned_campaign_ids: Arc::new(OnceLock::new()),
            pinned_capabilities: Arc::new(OnceLock::new()),
        };
        assert!(!policy.schedules_frozen());
        assert!(!policy.automatic_device_workers_frozen());
        assert!(policy.allows_publish_dispatch("any", "any"));
        assert!(policy.allows(AcceptanceCapability::SheetDelivery, "any", "any"));
    }

    #[test]
    fn malformed_or_oversized_scope_is_never_treated_as_permission() {
        let unknown = write_scope(serde_json::json!({
            "campaignIds": ["campaign-a"],
            "deviceIds": ["phone-a"],
            "allowEverything": true
        }));
        let policy = active(Some(unknown.0.clone()));
        assert!(!policy.allows_publish_dispatch("campaign-a", "phone-a"));

        for invalid in [
            serde_json::json!({"campaignIds":["a","a"],"deviceIds":["phone-a"]}),
            serde_json::json!({"campaignIds":["a"],"deviceIds":(0..101).map(|i|format!("phone-{i}")).collect::<Vec<_>>()}),
            serde_json::json!({"campaignIds":["a"],"deviceIds":["phone-a","phone-a"]}),
        ] {
            let scope = write_scope(invalid);
            let policy = active(Some(scope.0.clone()));
            assert!(!policy.allows_publish_dispatch("a", "phone-a"));
        }

        let oversized = write_scope(serde_json::json!({
            "campaignIds": ["campaign-a"],
            "deviceIds": ["x".repeat(70_000)]
        }));
        let policy = active(Some(oversized.0.clone()));
        assert!(!policy.allows_publish_dispatch("campaign-a", "phone-a"));
    }

    #[test]
    fn first_campaign_is_pinned_and_scope_identity_cannot_be_swapped() {
        let scope = write_scope(serde_json::json!({
            "campaignIds": ["campaign-a"],
            "deviceIds": ["phone-a"]
        }));
        let policy = active(Some(scope.0.clone()));
        assert!(policy.allows_publish_dispatch("campaign-a", "phone-a"));
        std::fs::write(
            &scope.0,
            serde_json::to_vec(&serde_json::json!({
                "activationId": ACTIVATION,
                "campaignIds": ["campaign-b"],
                "deviceIds": ["phone-a"]
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(!policy.allows_publish_dispatch("campaign-b", "phone-a"));
        assert!(!policy.allows_publish_dispatch("campaign-a", "phone-a"));
    }

    #[test]
    fn capability_grant_cannot_change_after_first_active_scope_read() {
        let scope = write_scope(serde_json::json!({
            "campaignIds": ["campaign-a"],
            "deviceIds": ["phone-a"],
            "capabilities": {"publishVerification": true, "sheetDelivery": false}
        }));
        let policy = active(Some(scope.0.clone()));
        assert!(policy.allows(
            AcceptanceCapability::PublishVerification,
            "campaign-a",
            "phone-a"
        ));
        std::fs::write(
            &scope.0,
            serde_json::to_vec(&serde_json::json!({
                "activationId": ACTIVATION,
                "campaignIds": ["campaign-a"],
                "deviceIds": ["phone-a"],
                "capabilities": {"publishVerification": true, "sheetDelivery": true}
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(!policy.allows(AcceptanceCapability::SheetDelivery, "campaign-a", "phone-a"));
        assert!(!policy.allows_publish_dispatch("campaign-a", "phone-a"));
    }

    #[test]
    fn restart_pins_an_already_active_campaign_without_opening_other_work() {
        let scope = write_scope(serde_json::json!({
            "campaignIds": ["campaign-a"],
            "deviceIds": ["phone-a"],
            "capabilities": {"publishVerification": true, "sheetDelivery": false}
        }));
        let policy = DevAcceptancePolicy::from_parts(
            true,
            Some(scope.0.clone()),
            Some(Arc::from(ACTIVATION)),
        );
        assert!(policy.allows_publish_dispatch("campaign-a", "phone-a"));
        assert!(policy.allows(
            AcceptanceCapability::PublishVerification,
            "campaign-a",
            "phone-a"
        ));
        assert!(!policy.allows_publish_dispatch("campaign-b", "phone-a"));
        assert!(!policy.allows_publish_dispatch("campaign-a", "phone-b"));
    }
}
