use std::{collections::HashMap, future::Future};
use tokio::task::{Id, JoinSet};

/// One observer per phone or retained metadata assignment; rows remain durable.
pub(crate) struct VerificationQueue {
    tasks: JoinSet<anyhow::Result<bool>>,
    devices: HashMap<Id, String>,
}

impl Default for VerificationQueue {
    fn default() -> Self {
        Self {
            tasks: JoinSet::new(),
            devices: HashMap::new(),
        }
    }
}
impl VerificationQueue {
    pub fn observer_key(row: &riviu_core::db::PendingPublishVerification) -> anyhow::Result<String> {
        Ok(if row.retained_metadata_candidate().map_or(true, |candidate| candidate.is_some()) {
            format!("publish-metadata:{}", row.assignment_id)
        } else { row.udid.clone() })
    }

    pub fn can_observe(
        row: &riviu_core::db::PendingPublishVerification,
        device: Option<&riviu_core::DeviceInfo>,
        phone_busy: bool,
    ) -> anyhow::Result<bool> {
        Ok(match row.retained_metadata_candidate() {
            // Admit only to persist a controlled terminal diagnostic, without IO.
            Err(_) => true,
            Ok(Some(_)) => row.automatic_metadata_observation_allowed(chrono::Utc::now()),
            Ok(None) => !phone_busy && device.is_some_and(Self::device_can_observe),
        })
    }

    pub fn device_can_observe(device: &riviu_core::DeviceInfo) -> bool {
        use riviu_core::{DevicePlatform, DeviceStatus};
        device.status == DeviceStatus::Ready
            || (device.platform == DevicePlatform::Android
                && device.status == DeviceStatus::Connected
                && device.wda_ready)
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// True when this UDID is not already running an observer.
    pub fn available(&self, udid: &str) -> bool {
        !self.devices.values().any(|id| id == udid)
    }

    pub fn push(
        &mut self,
        udid: String,
        task: impl Future<Output = anyhow::Result<bool>> + Send + 'static,
    ) {
        assert!(self.available(&udid));
        let id = self.tasks.spawn(task).id();
        self.devices.insert(id, udid);
    }

    pub async fn next(&mut self) -> Option<(String, anyhow::Result<bool>)> {
        let (id, result) = match self.tasks.join_next_with_id().await? {
            Ok((id, result)) => (id, result),
            Err(error) => (error.id(), Err(anyhow::anyhow!(error))),
        };
        Some((
            self.devices.remove(&id).expect("owned verification task"),
            result,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reconnected_android_can_verify_without_opening_manual_control_first() {
        let mut device: riviu_core::DeviceInfo = serde_json::from_value(serde_json::json!({
            "udid":"phone", "name":"SM G955F", "model":"SM G955F", "platform":"android",
            "osVersion":"9", "connection":"usb", "status":"connected", "battery":78,
            "wdaReady":true, "wdaExpiresAt":null, "streamUrl":null, "tileStreamState":"live", "lastError":null
        })).unwrap();
        assert!(VerificationQueue::device_can_observe(&device));
        device.wda_ready = false;
        assert!(!VerificationQueue::device_can_observe(&device));
        device.wda_ready = true;
        for status in [
            riviu_core::DeviceStatus::Disconnected,
            riviu_core::DeviceStatus::Pairing,
            riviu_core::DeviceStatus::Preparing,
            riviu_core::DeviceStatus::Busy,
            riviu_core::DeviceStatus::Error,
        ] {
            device.status = status;
            assert!(!VerificationQueue::device_can_observe(&device));
        }
        device.platform = riviu_core::DevicePlatform::Ios;
        device.status = riviu_core::DeviceStatus::Connected;
        assert!(!VerificationQueue::device_can_observe(&device));
        device.status = riviu_core::DeviceStatus::Ready;
        assert!(VerificationQueue::device_can_observe(&device));

        let intent = serde_json::json!({
            "effectIntent":"post","verificationContractVersion":1,"expectedAccount":"fixture",
            "submittedAt":"2026-10-06T01:00:00Z","package":"com.zhiliaoapp.musically",
            "version":"45.7.3","locale":"en","captionSha256":"a".repeat(64),"bundleId":"bundle","mediaKind":"image"
        }).to_string();
        let evidence = serde_json::json!({"verificationDiagnostic":{"pendingMetadataCandidate":{
            "schemaVersion":1,"campaignId":"campaign","assignmentId":"assignment","bundleId":"bundle",
            "intentSha256":riviu_core::frame_sha256(intent.as_bytes()),"captionSha256":"a".repeat(64),
            "captured":{"canonicalUrl":"https://www.tiktok.com/@fixture/photo/7550000000000000000",
                "postId":"7550000000000000000","expectedAccount":"fixture","normalizedCaptionSha256":"a".repeat(64),
                "preparedAt":null,"submittedAt":"2026-10-06T01:00:00Z","capturedAt":"2026-10-06T01:01:00Z",
                "provenance":"measuredViewerClipboard","metadata":{"state":"unavailable","attempts":1,
                    "checkedAt":"2026-10-06T01:01:00Z","stage":"httpStatus","httpStatus":400}}
        }}});
        let mut row = riviu_core::db::PendingPublishVerification {
            assignment_id:"assignment".into(),campaign_id:"campaign".into(),bundle_id:"bundle".into(),
            udid:"phone".into(),scheduled:false,revision:1,effect_intent:Some(intent),evidence_json:None,stop_marker:None,
        };
        assert!(!VerificationQueue::can_observe(&row, None, false).unwrap());
        assert!(!VerificationQueue::can_observe(&row, Some(&device), true).unwrap());
        row.evidence_json = Some(evidence.to_string());
        assert!(VerificationQueue::can_observe(&row, None, true).unwrap(), "offline/busy phone does not block metadata");
        assert_ne!(VerificationQueue::observer_key(&row).unwrap(), row.udid);
        for (field, value) in [("schemaVersion", serde_json::json!(2)), ("captured", serde_json::Value::Null)] {
            let mut invalid = evidence.clone();
            invalid["verificationDiagnostic"]["pendingMetadataCandidate"][field] = value;
            row.evidence_json = Some(invalid.to_string());
            assert!(VerificationQueue::can_observe(&row, Some(&device), false).unwrap(), "invalid candidate must reach diagnostic commit, never phone fallback");
        }
        row.evidence_json = Some(evidence.to_string().replace("unavailable", "rejected"));
        assert!(VerificationQueue::can_observe(&row, Some(&device), false).unwrap(), "Rejected rechecks the same candidate within budget");
        let mut exhausted = evidence.clone();
        exhausted["verificationBudget"] = serde_json::json!({"metadataObservations":12});
        row.evidence_json = Some(exhausted.to_string());
        assert!(!VerificationQueue::can_observe(&row, None, false).unwrap());
        row.evidence_json = Some(evidence.to_string());
        assert!(VerificationQueue::can_observe(&row, None, false).unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn slow_device_does_not_hold_a_completed_phone_or_the_next_phone() {
        let mut queue = VerificationQueue::default();
        queue.push("slow".into(), async {
            tokio::time::sleep(Duration::from_secs(120)).await;
            Ok(false)
        });
        assert!(!queue.available("slow"));
        for ordinal in 0..65 {
            let name = format!("fast-{ordinal}");
            assert!(queue.available(&name), "{name}");
            queue.push(name.to_string(), async { Ok(true) });
        }
        assert!(!queue.available("fast-0"));
        let start = tokio::time::Instant::now();
        let mut finished: std::collections::HashSet<_> =
            (0..65).map(|ordinal| format!("fast-{ordinal}")).collect();
        for _ in 0..65 {
            let (device, result) = queue.next().await.unwrap();
            assert!(result.unwrap());
            assert!(finished.remove(device.as_str()), "{device}");
            assert!(start.elapsed() < Duration::from_secs(120));
        }
        assert_eq!(queue.next().await.unwrap().0, "slow");
        assert!(queue.is_empty());
        assert!(queue.available("slow"));
    }

    #[tokio::test]
    async fn a_failed_observer_releases_only_its_own_device() {
        let mut queue = VerificationQueue::default();
        queue.push("broken".into(), async {
            panic!("fixture observer");
        });
        let (device, result) = queue.next().await.unwrap();
        assert_eq!(device, "broken");
        assert!(result.is_err());
        assert!(queue.available("broken"));
    }
}
