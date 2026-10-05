use riviu_core::{
    db::DeviceActivityProgress,
    device_work::{DeviceWorkOwner, HeldDeviceWork},
    NurturePhase, NurtureSessionStatus, OperationRunKind, OperationRunState,
};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceActivity {
    pub operation_id: String,
    pub kind: OperationRunKind,
    pub label: &'static str,
    pub step: Option<&'static str>,
    pub state: OperationRunState,
    pub updated_at: Option<String>,
}

/// No captions, errors, authors, script titles or raw event text reach the grid.
pub(super) fn project_activity(
    udid: &str,
    before: Option<HeldDeviceWork>,
    after: Option<HeldDeviceWork>,
    nurture: &[NurtureSessionStatus],
    progress: &[DeviceActivityProgress],
) -> Option<DeviceActivity> {
    let held = before.filter(|held| Some(*held) == after)?;
    if held.owner == DeviceWorkOwner::Nurture {
        let mut matches = nurture.iter().filter(|status| {
            status.udid == udid
                && status.running
                && status.phase != NurturePhase::Queued
                && !status.phase.is_terminal()
                && status.run_id.is_some()
                && status.updated_at.is_some_and(|at| at >= held.acquired_at)
        });
        let status = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        let step = match status.last_message.as_str() {
            "thả tim" => "Đang thích bài",
            "tim thành công (xác nhận icon đỏ)" => "Đã thích bài",
            "lưu video" => "Đang lưu video",
            "bình luận" => "Đang bình luận",
            "đã gửi bình luận chữ (xác nhận nút gửi tắt)" => {
                "Đã xác nhận gửi bình luận"
            }
            "vuốt video tiếp" => "Đang chuyển video",
            _ => match status.phase {
                NurturePhase::Queued => return None,
                NurturePhase::Opening => "Đang mở TikTok",
                NurturePhase::AwaitingFeed => "Đang tìm bảng tin",
                NurturePhase::Watching => "Đang lướt video",
                NurturePhase::Recovering => "Đang khôi phục bảng tin",
                NurturePhase::Finished => return None,
            },
        };
        return Some(DeviceActivity {
            operation_id: format!("nurture:{}", status.run_id?),
            kind: OperationRunKind::Nurture,
            label: "Nuôi TikTok",
            step: Some(step),
            state: OperationRunState::Running,
            updated_at: status.updated_at.map(|at| at.to_rfc3339()),
        });
    }
    let mut matches = progress.iter().filter(|row| {
        row.udid == udid
            && matches!(
                (held.owner, row.kind),
                (DeviceWorkOwner::Interaction, OperationRunKind::Interaction)
                    | (
                        DeviceWorkOwner::Script,
                        OperationRunKind::Publish
                            | OperationRunKind::Flow
                            | OperationRunKind::Script
                    )
            )
            && chrono::DateTime::parse_from_rfc3339(&row.updated_at)
                .is_ok_and(|at| at >= held.acquired_at)
    });
    let row = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let label = match row.kind {
        OperationRunKind::Publish => "Đăng bài",
        OperationRunKind::Interaction => "Tương tác",
        OperationRunKind::Flow => "Flow thiết bị",
        OperationRunKind::Script => "Kịch bản",
        _ => return None,
    };
    Some(DeviceActivity {
        operation_id: row.operation_id.clone(),
        kind: row.kind,
        label,
        step: safe_step(row),
        state: if matches!(row.state.as_str(), "uncertain" | "post_uncertain") {
            OperationRunState::Uncertain
        } else {
            OperationRunState::Running
        },
        updated_at: Some(row.updated_at.clone()),
    })
}

fn safe_step(row: &DeviceActivityProgress) -> Option<&'static str> {
    if row.kind == OperationRunKind::Interaction {
        return match (row.action.as_str(), row.state.as_str()) {
            ("like", "confirmed") => Some("Đã thích bài"),
            ("save", "confirmed") => Some("Đã lưu bài"),
            ("follow", "confirmed") => Some("Đã theo dõi"),
            ("comment", "confirmed") => Some("Đã xác nhận bình luận"),
            ("like", "preparing" | "armed") => Some("Đang thích bài"),
            ("save", "preparing" | "armed") => Some("Đang lưu bài"),
            ("follow", "preparing" | "armed") => Some("Đang theo dõi"),
            ("comment", "preparing" | "armed") => Some("Đang bình luận"),
            (_, "uncertain") => Some("Chưa xác nhận kết quả"),
            ("interaction", "preparing" | "ready") => Some("Đang chuẩn bị tương tác"),
            ("interaction", "sending") => Some("Đang gửi bình luận"),
            _ => None,
        };
    }
    if row.kind == OperationRunKind::Publish && row.action == "publishStep" {
        return Some(match row.state.as_str() {
            "checking_device" => "Đang kiểm tra thiết bị",
            "transferring_media" => "Đang chuyển media",
            "opening_app" => "Đang mở TikTok",
            "opening_composer" => "Đang mở màn đăng bài",
            "opening_gallery" | "selecting_album" => "Đang mở thư viện",
            "selecting_media" => "Đang chọn media",
            "opening_editor" => "Đang mở trình chỉnh sửa",
            "opening_sounds" | "selecting_sound" => "Đang chọn nhạc",
            "sound_confirmed" => "Đã xác nhận nhạc",
            "opening_caption" | "entering_caption" => "Đang nhập nội dung",
            "checking_before_post" => "Đang kiểm tra trước khi đăng",
            "submitting_post" => "Đang bấm Đăng",
            "awaiting_post" | "post_submitted" => "Đang chờ xác minh bài đăng",
            "post_confirmed" => "Đã xác minh bài đăng",
            "capturing_link" | "checking_existing_post_link" => "Đang kiểm tra liên kết",
            "link_captured" => "Đã lấy liên kết",
            "link_pending" | "link_needs_review" => "Liên kết cần kiểm tra",
            "post_uncertain" => "Chưa xác nhận kết quả đăng",
            "read_recovery" | "retry_waiting" => "Đang kiểm tra lại giao diện",
            "finishing" => "Đang hoàn tất",
            _ => return None,
        });
    }
    if row.kind == OperationRunKind::Flow
        && matches!(
            row.state.as_str(),
            "intentCommitted" | "effectDispatched" | "verifying"
        )
    {
        let action = match row.action.as_str() {
            "launchApp" => "Đang mở ứng dụng",
            "terminateApp" => "Đang đóng ứng dụng",
            "tap" | "tapVision" => "Đang chạm màn hình",
            "swipe" | "autoSwipe" => "Đang vuốt màn hình",
            "wait" => "Đang chờ",
            "typeText" => "Đang nhập văn bản",
            "home" => "Đang về màn hình chính",
            "screenshot" => "Đang chụp màn hình",
            "readText" | "ocrReadText" => "Đang đọc màn hình",
            "assertVisible" | "ifVisible" | "ifVision" => "Đang kiểm tra màn hình",
            _ => return None,
        };
        return Some(action);
    }
    // These states describe execution, never an inferred public action result.
    match row.state.as_str() {
        "running" | "intentCommitted" => Some("Đang thực hiện bước"),
        "effectDispatched" | "verifying" => Some("Đang kiểm tra kết quả bước"),
        "uncertain" => Some("Chưa xác nhận kết quả bước"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_activity_requires_same_held_lease_and_current_nurture_status() {
        let at = "2026-10-05T00:00:00Z".parse().unwrap();
        let held = HeldDeviceWork {
            owner: DeviceWorkOwner::Nurture,
            token: uuid::Uuid::new_v4(),
            acquired_at: at,
        };
        let mut status = NurtureSessionStatus::new("phone");
        status.running = true;
        status.run_id = Some(uuid::Uuid::new_v4());
        status.phase = NurturePhase::Watching;
        status.updated_at = Some(at);
        status.last_message = "PRIVATE CAPTION".into();
        let statuses = [status.clone()];
        let read = |before, after, statuses: &[NurtureSessionStatus]| {
            project_activity("phone", before, after, statuses, &[])
        };
        let activity = read(Some(held), Some(held), &statuses).unwrap();
        assert_eq!(
            activity.operation_id,
            format!("nurture:{}", status.run_id.unwrap())
        );
        assert_eq!(activity.step, Some("Đang lướt video"));
        assert!(!serde_json::to_string(&activity)
            .unwrap()
            .contains("PRIVATE"));
        assert!(read(Some(held), None, &statuses).is_none());
        assert!(read(None, Some(held), &statuses).is_none());
        assert!(read(
            Some(held),
            Some(HeldDeviceWork {
                token: uuid::Uuid::new_v4(),
                ..held
            }),
            &statuses
        )
        .is_none());
        status.updated_at = Some(at - chrono::Duration::seconds(1));
        assert!(read(Some(held), Some(held), &[status.clone()]).is_none());
        status.updated_at = Some(at);
        status.phase = NurturePhase::Finished;
        assert!(read(Some(held), Some(held), &[status]).is_none());
    }

    #[test]
    fn device_activity_only_verified_interaction_claims_completion() {
        let held = HeldDeviceWork {
            owner: DeviceWorkOwner::Interaction,
            token: uuid::Uuid::new_v4(),
            acquired_at: "2026-10-05T00:00:00Z".parse().unwrap(),
        };
        let mut row = DeviceActivityProgress {
            udid: "phone".into(),
            operation_id: "interaction:run".into(),
            kind: OperationRunKind::Interaction,
            action: "like".into(),
            state: "armed".into(),
            updated_at: held.acquired_at.to_rfc3339(),
        };
        let project = |row| project_activity("phone", Some(held), Some(held), &[], &[row]).unwrap();
        assert_eq!(project(row.clone()).step, Some("Đang thích bài"));
        row.state = "confirmed".into();
        assert_eq!(project(row.clone()).step, Some("Đã thích bài"));
        row.state = "uncertain".into();
        assert_eq!(project(row).step, Some("Chưa xác nhận kết quả"));
    }

    #[test]
    fn device_activity_flow_uses_safe_action_kind_not_node_text() {
        let held = HeldDeviceWork {
            owner: DeviceWorkOwner::Script,
            token: uuid::Uuid::new_v4(),
            acquired_at: "2026-10-05T00:00:00Z".parse().unwrap(),
        };
        let mut row = DeviceActivityProgress {
            udid: "phone".into(),
            operation_id: "flow:run".into(),
            kind: OperationRunKind::Flow,
            action: "swipe".into(),
            state: "effectDispatched".into(),
            updated_at: held.acquired_at.to_rfc3339(),
        };
        let project = |row| project_activity("phone", Some(held), Some(held), &[], &[row]).unwrap();
        assert_eq!(project(row.clone()).step, Some("Đang vuốt màn hình"));
        row.action = "PRIVATE CUSTOM NODE".into();
        assert_eq!(project(row.clone()).step, None);
        row.action = "swipe".into();
        row.state = "uncertain".into();
        assert_eq!(project(row).step, Some("Chưa xác nhận kết quả bước"));
    }
}
