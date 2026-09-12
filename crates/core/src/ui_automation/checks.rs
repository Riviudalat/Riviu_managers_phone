use super::{AutomationCheck, CheckStatus};
use crate::{PublishPreflightAssignmentReport, PublishPreflightCheck};

pub fn publish_checks(row: &PublishPreflightAssignmentReport) -> Vec<AutomationCheck> {
    let mut checks = Vec::new();
    for (id, label, status) in [
        ("media", "Nội dung", row.media),
        ("composer", "Luồng soạn bài", row.composer),
        ("sound", "Chọn nhạc", row.sound_picker),
        ("storage", "Dung lượng", row.storage),
    ] {
        checks.push(AutomationCheck {
            id: id.into(),
            label: label.into(),
            status: if status == PublishPreflightCheck::Pass {
                CheckStatus::Pass
            } else {
                CheckStatus::Blocked
            },
            reason: None,
        });
    }
    for (id, label, codes) in [
        (
            "transport",
            "Kết nối điều khiển",
            &[
                "device_missing",
                "automation_transport_conflict",
                "android_required",
            ][..],
        ),
        ("helper", "Chuyển nội dung", &["push_media_unavailable"][..]),
        (
            "link",
            "Nhận diện xác minh liên kết",
            &["link_verification_unmeasured", "tiktok_build_unreadable"][..],
        ),
        (
            "pending",
            "Lượt đăng trước",
            &["post_verification_pending"][..],
        ),
    ] {
        let issue = row.issues.iter().find(|i| codes.contains(&i.code.as_str()));
        checks.push(AutomationCheck {
            id: id.into(),
            label: label.into(),
            status: if issue.is_some() {
                CheckStatus::Blocked
            } else {
                CheckStatus::Pass
            },
            reason: issue.map(|i| i.message.clone()),
        });
    }
    for (id, label) in [
        ("account", "Tài khoản trước Đăng"),
        ("clipboard", "Đọc và khôi phục clipboard"),
        ("published", "Kết quả xuất bản"),
    ] {
        checks.push(AutomationCheck {
            id: id.into(),
            label: label.into(),
            status: CheckStatus::Unknown,
            reason: Some("Kiểm tra trong phiên thực thi; chưa đăng bài".into()),
        });
    }
    let adaptive = row
        .package_name
        .as_deref()
        .zip(row.locale.as_deref())
        .zip(row.version.as_deref())
        .is_some_and(|((p, l), v)| {
            crate::tiktok_labels::controls_for_runtime(p, l, v).is_some_and(|c| c.adaptive())
        });
    if adaptive {
        for check in &mut checks {
            if ["composer", "sound", "link"].contains(&check.id.as_str())
                && check.status == CheckStatus::Pass
            {
                check.status = CheckStatus::Unknown;
                check.reason = Some(
                    "Sẽ kiểm tra cấu trúc màn hình trong phiên; chưa nghiệm thu phiên bản này"
                        .into(),
                );
            }
        }
    }
    checks
}
