use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::types::{DeviceGroup, DeviceMeta};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationKind {
    Nurture,
    Interaction,
    Publish,
}

impl AutomationKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Nurture => "nurture",
            Self::Interaction => "interaction",
            Self::Publish => "publish",
        }
    }

    pub(crate) fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "nurture" => Ok(Self::Nurture),
            "interaction" => Ok(Self::Interaction),
            "publish" => Ok(Self::Publish),
            _ => anyhow::bail!("unknown automation kind `{value}`"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TargetRef {
    All,
    Group { group_id: String },
    Explicit { udids: Vec<String> },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExcludedDeviceReason {
    NotInRoster,
    DuplicateExplicit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTargetDevice {
    pub udid: String,
    pub alias: String,
    pub number: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTargetExclusion {
    pub device: ResolvedTargetDevice,
    pub reason: ExcludedDeviceReason,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTargetSnapshot {
    pub target_ref: TargetRef,
    pub included: Vec<ResolvedTargetDevice>,
    pub excluded: Vec<ResolvedTargetExclusion>,
    pub roster_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TargetResolutionError {
    #[error("automation target group `{group_id}` does not exist")]
    GroupNotFound { group_id: String },
}

pub fn resolve_target(
    target_ref: &TargetRef,
    fleet_order: &[String],
    device_meta: &[DeviceMeta],
    groups: &[DeviceGroup],
) -> Result<ResolvedTargetSnapshot, TargetResolutionError> {
    let roster: HashSet<&str> = fleet_order.iter().map(String::as_str).collect();
    let roster_projection: Vec<ResolvedTargetDevice> = fleet_order
        .iter()
        .map(|udid| device_projection(udid, device_meta))
        .collect();
    let mut excluded = Vec::new();
    let included = match target_ref {
        TargetRef::All => roster_projection.clone(),
        TargetRef::Group { group_id } => {
            let group = groups
                .iter()
                .find(|group| group.id == *group_id)
                .ok_or_else(|| TargetResolutionError::GroupNotFound {
                    group_id: group_id.clone(),
                })?;
            let members: HashSet<&str> = group.udids.iter().map(String::as_str).collect();
            for udid in &group.udids {
                if !roster.contains(udid.as_str()) {
                    excluded.push(ResolvedTargetExclusion {
                        device: device_projection(udid, device_meta),
                        reason: ExcludedDeviceReason::NotInRoster,
                    });
                }
            }
            roster_projection
                .iter()
                .filter(|device| members.contains(device.udid.as_str()))
                .cloned()
                .collect()
        }
        TargetRef::Explicit { udids } => {
            let mut seen = HashSet::new();
            let mut included = Vec::new();
            for udid in udids {
                if !seen.insert(udid.as_str()) {
                    excluded.push(ResolvedTargetExclusion {
                        device: device_projection(udid, device_meta),
                        reason: ExcludedDeviceReason::DuplicateExplicit,
                    });
                } else if roster.contains(udid.as_str()) {
                    included.push(device_projection(udid, device_meta));
                } else {
                    excluded.push(ResolvedTargetExclusion {
                        device: device_projection(udid, device_meta),
                        reason: ExcludedDeviceReason::NotInRoster,
                    });
                }
            }
            included
        }
    };

    Ok(ResolvedTargetSnapshot {
        target_ref: target_ref.clone(),
        included,
        excluded,
        roster_sha256: roster_sha256(&roster_projection),
    })
}

fn device_projection(udid: &str, device_meta: &[DeviceMeta]) -> ResolvedTargetDevice {
    let metadata = device_meta.iter().find(|metadata| metadata.udid == udid);
    ResolvedTargetDevice {
        udid: udid.to_owned(),
        alias: metadata
            .map(|metadata| metadata.alias.clone())
            .unwrap_or_default(),
        number: metadata.and_then(|metadata| metadata.number),
    }
}

fn roster_sha256(roster: &[ResolvedTargetDevice]) -> String {
    let bytes = serde_json::to_vec(roster).expect("serializing device projections cannot fail");
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationDefinition {
    pub id: Uuid,
    pub name: String,
    pub kind: AutomationKind,
    pub latest_revision: u64,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationDefinitionRevision {
    pub definition_id: Uuid,
    pub revision: u64,
    pub target_ref: TargetRef,
    pub config: Value,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationDefinitionRecord {
    pub definition: AutomationDefinition,
    pub revision: AutomationDefinitionRevision,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("automation definition {definition_id} does not exist")]
pub struct AutomationDefinitionNotFound {
    pub definition_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("automation definition {definition_id} is archived")]
pub struct AutomationDefinitionArchived {
    pub definition_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("automation revision conflict: expected {expected}, actual {actual}")]
pub struct AutomationRevisionConflict {
    pub expected: u64,
    pub actual: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationSchedule {
    pub id: Uuid,
    pub revision: u64,
    pub name: String,
    pub definition_id: Uuid,
    pub definition_revision: u64,
    pub enabled: bool,
    /// Raw read model kept for forward-compatible display. Mutating commands accept only the
    /// strict [`AutomationScheduleV1`] DTO.
    pub schedule: Value,
    pub next_due_at: Option<String>,
    pub last_error_code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub const AUTOMATION_SCHEDULE_SCHEMA_VERSION: u8 = 1;
pub const MIN_AUTOMATION_INTERVAL_MINUTES: u16 = 15;
pub const MAX_AUTOMATION_INTERVAL_MINUTES: u16 = 1_440;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AutomationScheduleKind {
    Interval,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomationScheduleV1 {
    #[serde(deserialize_with = "deserialize_schedule_schema_v1")]
    pub schema_version: u8,
    pub kind: AutomationScheduleKind,
    #[serde(deserialize_with = "deserialize_interval_minutes")]
    pub every_minutes: u16,
}

fn deserialize_interval_minutes<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let minutes = u16::deserialize(deserializer)?;
    if !(MIN_AUTOMATION_INTERVAL_MINUTES..=MAX_AUTOMATION_INTERVAL_MINUTES).contains(&minutes) {
        return Err(serde::de::Error::custom(format!(
            "automation interval must be {MIN_AUTOMATION_INTERVAL_MINUTES}..={MAX_AUTOMATION_INTERVAL_MINUTES} minutes"
        )));
    }
    Ok(minutes)
}

impl AutomationScheduleV1 {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == AUTOMATION_SCHEDULE_SCHEMA_VERSION,
            "automation schedule schemaVersion must be {AUTOMATION_SCHEDULE_SCHEMA_VERSION}"
        );
        anyhow::ensure!(
            (MIN_AUTOMATION_INTERVAL_MINUTES..=MAX_AUTOMATION_INTERVAL_MINUTES)
                .contains(&self.every_minutes),
            "automation interval must be {MIN_AUTOMATION_INTERVAL_MINUTES}..={MAX_AUTOMATION_INTERVAL_MINUTES} minutes"
        );
        Ok(())
    }
}

fn deserialize_schedule_schema_v1<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let version = u8::deserialize(deserializer)?;
    if version != AUTOMATION_SCHEDULE_SCHEMA_VERSION {
        return Err(serde::de::Error::custom(format!(
            "automation schedule schemaVersion must be {AUTOMATION_SCHEDULE_SCHEMA_VERSION}"
        )));
    }
    Ok(version)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationScheduleOccurrenceState {
    Queued,
    Dispatching,
    Running,
    Done,
    Partial,
    Failed,
    Uncertain,
}

impl AutomationScheduleOccurrenceState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Partial | Self::Failed | Self::Uncertain
        )
    }

    pub fn outcome(self) -> Option<crate::orchestration::ChildCampaignOutcome> {
        match self {
            Self::Queued | Self::Dispatching | Self::Running => None,
            Self::Done => Some(crate::orchestration::ChildCampaignOutcome::Done),
            Self::Partial => Some(crate::orchestration::ChildCampaignOutcome::Partial),
            Self::Failed => Some(crate::orchestration::ChildCampaignOutcome::Failed),
            Self::Uncertain => Some(crate::orchestration::ChildCampaignOutcome::Uncertain),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationScheduleOccurrence {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub schedule_revision: u64,
    pub scheduled_for: String,
    pub kind: AutomationKind,
    pub profile: crate::AutomationProfileRef,
    pub target: Option<ResolvedTargetSnapshot>,
    pub child_campaign_id: Uuid,
    pub idempotency_key: String,
    pub state: AutomationScheduleOccurrenceState,
    pub nurture_run_id: Option<Uuid>,
    pub nurture_started_udids: Vec<String>,
    pub error_code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("automation schedule {schedule_id} does not exist")]
pub struct AutomationScheduleNotFound {
    pub schedule_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("automation schedule revision conflict: expected {expected}, actual {actual}")]
pub struct AutomationScheduleConflict {
    pub expected: u64,
    pub actual: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("automation config contains secret field `{field}` at {path}")]
pub struct AutomationConfigSecret {
    pub field: String,
    pub path: String,
}

pub fn validate_automation_config(config: &Value) -> Result<(), AutomationConfigSecret> {
    fn is_secret_key(key: &str) -> bool {
        let normalized: String = key
            .bytes()
            .filter(|byte| !matches!(byte, b'_' | b'-'))
            .map(|byte| byte.to_ascii_lowercase() as char)
            .collect();
        matches!(
            normalized.as_str(),
            "apikey"
                | "token"
                | "accesstoken"
                | "refreshtoken"
                | "webhooktoken"
                | "password"
                | "secret"
                | "clientsecret"
        )
    }

    fn visit(value: &Value, path: &str) -> Result<(), AutomationConfigSecret> {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    let child_path = format!("{path}.{key}");
                    if is_secret_key(key) {
                        return Err(AutomationConfigSecret {
                            field: key.clone(),
                            path: child_path,
                        });
                    }
                    visit(child, &child_path)?;
                }
            }
            Value::Array(array) => {
                for (index, child) in array.iter().enumerate() {
                    visit(child, &format!("{path}[{index}]"))?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    visit(config, "$")
}

pub const AUTOMATION_PROFILE_CONFIG_SCHEMA_VERSION: u8 = 1;

fn deserialize_profile_schema_v1<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let version = u8::deserialize(deserializer)?;
    if version != AUTOMATION_PROFILE_CONFIG_SCHEMA_VERSION {
        return Err(serde::de::Error::custom(format!(
            "automation profile schemaVersion must be {AUTOMATION_PROFILE_CONFIG_SCHEMA_VERSION}"
        )));
    }
    Ok(version)
}

/// Immutable Nurture input stored in an automation profile revision.
///
/// `settings` intentionally omits `apiKey` and `hasApiKey`; the command boundary injects the
/// current credential after loading this secret-free snapshot. Missing settings use the same
/// defaults as the normal Nurture editor, which keeps old profiles readable as settings grow.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NurtureAutomationProfileConfigV1 {
    #[serde(deserialize_with = "deserialize_profile_schema_v1")]
    pub schema_version: u8,
    pub settings: crate::NurtureSettings,
    #[serde(default)]
    pub duration_minutes: Option<u32>,
}

/// Interaction fields an operator approves. Run identity and actors are deliberately absent:
/// orchestration injects both from the durable attempt and its resolved target snapshot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractionCampaignTemplateV1 {
    pub targets: Vec<crate::ResolvedTikTokTarget>,
    pub message_count: u8,
    pub instruction: String,
    pub max_words: u8,
    #[serde(default)]
    pub mode: crate::ThreadMode,
    #[serde(default)]
    pub shape: crate::ThreadShape,
    #[serde(default)]
    pub cohort_size: Option<u8>,
    #[serde(default)]
    pub manual_comments: Vec<String>,
    #[serde(default)]
    pub actions: crate::InteractionActionSet,
    #[serde(default)]
    pub mentions: Vec<String>,
    #[serde(default)]
    pub mention_parent: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractionAutomationProfileConfigV1 {
    #[serde(deserialize_with = "deserialize_profile_schema_v1")]
    pub schema_version: u8,
    pub request: InteractionCampaignTemplateV1,
}

impl InteractionAutomationProfileConfigV1 {
    pub fn into_campaign_request(
        self,
        request_id: String,
        actor_udids: Vec<String>,
    ) -> crate::ThreadCampaignRequest {
        crate::ThreadCampaignRequest {
            request_id,
            targets: self.request.targets,
            actor_udids,
            message_count: self.request.message_count,
            instruction: self.request.instruction,
            max_words: self.request.max_words,
            mode: self.request.mode,
            shape: self.request.shape,
            cohort_size: self.request.cohort_size,
            manual_comments: self.request.manual_comments,
            actions: self.request.actions,
            mentions: self.request.mentions,
            mention_parent: self.request.mention_parent,
        }
    }
}

/// Publish input stored before a child exists. The adapter stages the named source bundles,
/// injects the attempt idempotency key and target UDIDs, and always runs immediately.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishAutomationProfileConfigV1 {
    #[serde(deserialize_with = "deserialize_profile_schema_v1")]
    pub schema_version: u8,
    pub source_root: String,
    pub bundle_ids: Vec<String>,
    #[serde(default)]
    pub caption_overrides: BTreeMap<String, String>,
    pub sound_policy: crate::PublishSoundPolicy,
    pub execution_confirmed: bool,
}

/// Validates the immutable payload against the schema owned by its automation kind.
///
/// Keeping this at the persistence boundary prevents a pinned revision from looking valid until
/// the child is dispatched. The secret scan remains independent so callers retain its precise
/// JSON path in error messages.
pub fn validate_automation_profile_config(
    kind: AutomationKind,
    config: &Value,
) -> anyhow::Result<()> {
    validate_automation_config(config)?;
    match kind {
        AutomationKind::Nurture => {
            serde_json::from_value::<NurtureAutomationProfileConfigV1>(config.clone()).map_err(
                |error| anyhow::anyhow!("invalid nurture automation profile config: {error}"),
            )?;
        }
        AutomationKind::Interaction => {
            serde_json::from_value::<InteractionAutomationProfileConfigV1>(config.clone())
                .map_err(|error| {
                    anyhow::anyhow!("invalid interaction automation profile config: {error}")
                })?;
        }
        AutomationKind::Publish => {
            serde_json::from_value::<PublishAutomationProfileConfigV1>(config.clone()).map_err(
                |error| anyhow::anyhow!("invalid publish automation profile config: {error}"),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DeviceGroup, DeviceMeta};

    fn group(id: &str, udids: &[&str]) -> DeviceGroup {
        DeviceGroup {
            id: id.into(),
            name: id.into(),
            color: "#000000".into(),
            udids: udids.iter().map(|udid| (*udid).into()).collect(),
            created_at: "2026-09-03T00:00:00Z".into(),
        }
    }

    fn meta(udid: &str, alias: &str, number: Option<u32>) -> DeviceMeta {
        DeviceMeta {
            udid: udid.into(),
            notes: String::new(),
            tags: vec![],
            group_id: None,
            handle: String::new(),
            alias: alias.into(),
            number,
        }
    }

    #[test]
    fn all_targets_follow_stable_fleet_order_and_hash_the_roster() {
        let target = TargetRef::All;
        let metadata = [
            meta("phone-a", "Desk A", Some(10)),
            meta("phone-b", "Desk B", Some(2)),
        ];
        let first = resolve_target(
            &target,
            &["phone-b".into(), "phone-a".into()],
            &metadata,
            &[],
        )
        .expect("resolve all");
        let again = resolve_target(
            &target,
            &["phone-b".into(), "phone-a".into()],
            &metadata,
            &[],
        )
        .expect("resolve all again");

        assert_eq!(
            first.included,
            vec![
                ResolvedTargetDevice {
                    udid: "phone-b".into(),
                    alias: "Desk B".into(),
                    number: Some(2),
                },
                ResolvedTargetDevice {
                    udid: "phone-a".into(),
                    alias: "Desk A".into(),
                    number: Some(10),
                },
            ]
        );
        assert_eq!(first.roster_sha256, again.roster_sha256);
        assert_eq!(
            first.roster_sha256,
            "d5a3eff5a498f7be8788c497f5b455c769b9acdd7ae23845fe4b77b81360d54d"
        );

        let reordered = resolve_target(
            &target,
            &["phone-a".into(), "phone-b".into()],
            &metadata,
            &[],
        )
        .expect("resolve reordered roster");
        assert_ne!(first.roster_sha256, reordered.roster_sha256);

        let renamed = resolve_target(
            &target,
            &["phone-b".into(), "phone-a".into()],
            &[
                meta("phone-a", "Renamed", Some(10)),
                meta("phone-b", "Desk B", Some(2)),
            ],
            &[],
        )
        .expect("resolve renamed roster");
        assert_ne!(first.roster_sha256, renamed.roster_sha256);

        let renumbered = resolve_target(
            &target,
            &["phone-b".into(), "phone-a".into()],
            &[
                meta("phone-a", "Desk A", Some(11)),
                meta("phone-b", "Desk B", Some(2)),
            ],
            &[],
        )
        .expect("resolve renumbered roster");
        assert_ne!(first.roster_sha256, renumbered.roster_sha256);
    }

    #[test]
    fn a_group_is_resolved_against_each_current_roster() {
        let target = TargetRef::Group {
            group_id: "morning".into(),
        };
        let groups = [group("morning", &["phone-a", "phone-c"])];

        let metadata = [meta("phone-a", "A", Some(1)), meta("phone-c", "C", Some(3))];
        let first = resolve_target(
            &target,
            &["phone-a".into(), "phone-b".into()],
            &metadata,
            &groups,
        )
        .expect("first roster");
        assert_eq!(
            first.included,
            vec![ResolvedTargetDevice {
                udid: "phone-a".into(),
                alias: "A".into(),
                number: Some(1)
            }]
        );
        assert_eq!(
            first.excluded,
            vec![ResolvedTargetExclusion {
                device: ResolvedTargetDevice {
                    udid: "phone-c".into(),
                    alias: "C".into(),
                    number: Some(3),
                },
                reason: ExcludedDeviceReason::NotInRoster,
            }]
        );

        let second = resolve_target(
            &target,
            &["phone-c".into(), "phone-a".into(), "phone-b".into()],
            &metadata,
            &groups,
        )
        .expect("second roster");
        assert_eq!(
            second.included,
            vec![
                ResolvedTargetDevice {
                    udid: "phone-c".into(),
                    alias: "C".into(),
                    number: Some(3)
                },
                ResolvedTargetDevice {
                    udid: "phone-a".into(),
                    alias: "A".into(),
                    number: Some(1)
                },
            ]
        );
        assert!(second.excluded.is_empty());
        assert_ne!(first.roster_sha256, second.roster_sha256);
    }

    #[test]
    fn a_missing_group_is_a_typed_error() {
        let error = resolve_target(
            &TargetRef::Group {
                group_id: "missing".into(),
            },
            &["phone-a".into()],
            &[],
            &[],
        )
        .expect_err("missing group");

        assert_eq!(
            error,
            TargetResolutionError::GroupNotFound {
                group_id: "missing".into()
            }
        );
    }

    #[test]
    fn explicit_targets_keep_caller_order_and_report_duplicates_and_absence() {
        let snapshot = resolve_target(
            &TargetRef::Explicit {
                udids: vec![
                    "phone-c".into(),
                    "phone-a".into(),
                    "phone-c".into(),
                    "offline".into(),
                ],
            },
            &["phone-a".into(), "phone-c".into()],
            &[
                meta("phone-a", "Alpha", Some(1)),
                meta("offline", "Shelf 9", Some(9)),
            ],
            &[],
        )
        .expect("resolve explicit");

        assert_eq!(
            snapshot.included,
            vec![
                ResolvedTargetDevice {
                    udid: "phone-c".into(),
                    alias: String::new(),
                    number: None
                },
                ResolvedTargetDevice {
                    udid: "phone-a".into(),
                    alias: "Alpha".into(),
                    number: Some(1)
                },
            ]
        );
        assert_eq!(
            snapshot.excluded,
            vec![
                ResolvedTargetExclusion {
                    device: ResolvedTargetDevice {
                        udid: "phone-c".into(),
                        alias: String::new(),
                        number: None
                    },
                    reason: ExcludedDeviceReason::DuplicateExplicit,
                },
                ResolvedTargetExclusion {
                    device: ResolvedTargetDevice {
                        udid: "offline".into(),
                        alias: "Shelf 9".into(),
                        number: Some(9)
                    },
                    reason: ExcludedDeviceReason::NotInRoster,
                },
            ]
        );
    }

    #[test]
    fn nested_secret_field_names_are_rejected_case_insensitively() {
        for (config, path) in [
            (
                serde_json::json!({"outer": {"ApiKey": "secret"}}),
                "$.outer.ApiKey",
            ),
            (
                serde_json::json!({"items": [{"api_key": "credential"}]}),
                "$.items[0].api_key",
            ),
            (serde_json::json!({"Password": "secret"}), "$.Password"),
            (
                serde_json::json!({"access-token": "credential"}),
                "$.access-token",
            ),
            (
                serde_json::json!({"refreshToken": "credential"}),
                "$.refreshToken",
            ),
            (
                serde_json::json!({"webhook_token": "credential"}),
                "$.webhook_token",
            ),
            (serde_json::json!({"secret": "credential"}), "$.secret"),
            (
                serde_json::json!({"CLIENT_SECRET": "credential"}),
                "$.CLIENT_SECRET",
            ),
        ] {
            let error = validate_automation_config(&config).expect_err("secret field");
            assert_eq!(error.path, path);
        }
        validate_automation_config(&serde_json::json!({
            "tokenBudget": 3,
            "credentials": [{
                "label": "not a known secret field",
                "description": "this harmless value mentions api_key, accessToken and secret"
            }]
        }))
        .expect("ordinary config");
    }

    #[test]
    fn profile_execution_configs_are_versioned_and_exclude_runtime_identity() {
        let interaction: InteractionAutomationProfileConfigV1 =
            serde_json::from_value(serde_json::json!({
                "schemaVersion": 1,
                "request": {
                    "targets": [{
                        "originalUrl": "https://www.tiktok.com/@fixture/video/123",
                        "normalizedUrl": "https://www.tiktok.com/@fixture/video/123",
                        "targetKey": "fixture:123",
                        "author": "fixture",
                        "contentId": "123",
                        "kind": "video"
                    }],
                    "messageCount": 2,
                    "instruction": "Viet ngan",
                    "maxWords": 12,
                    "actions": { "like": true, "comment": false, "save": true }
                }
            }))
            .expect("typed interaction profile");
        let request = interaction.into_campaign_request(
            "attempt-key".into(),
            vec!["phone-1".into(), "phone-2".into()],
        );
        assert_eq!(request.request_id, "attempt-key");
        assert_eq!(request.actor_udids, ["phone-1", "phone-2"]);

        let publish: PublishAutomationProfileConfigV1 = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "sourceRoot": "C:/fixture",
            "bundleIds": ["bundle-a"],
            "soundPolicy": { "kind": "default" },
            "executionConfirmed": true
        }))
        .expect("typed publish profile");
        assert_eq!(publish.bundle_ids, ["bundle-a"]);

        let nurture: NurtureAutomationProfileConfigV1 = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "settings": { "numVideos": 3, "numRounds": 2 },
            "durationMinutes": 15
        }))
        .expect("typed nurture profile");
        assert_eq!(nurture.settings.num_videos, 3);
        assert_eq!(nurture.duration_minutes, Some(15));

        assert!(
            serde_json::from_value::<PublishAutomationProfileConfigV1>(serde_json::json!({
                "schemaVersion": 1,
                "sourceRoot": "C:/fixture",
                "bundleIds": ["bundle-a"],
                "executionConfirmed": true,
                "requestId": "must-not-be-stored"
            }))
            .is_err()
        );
    }
}
