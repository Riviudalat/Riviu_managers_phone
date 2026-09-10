import type {
  PublishExecutionIssue,
  PublishPreflightAssignmentReport,
} from "../../types";

export interface PublishPreflightProblem {
  title: string;
  action: string;
}

/** Labels only: this projection never changes the backend's canExecute decision. */
export function publishTikTokBuildLabel(
  row: Pick<
    PublishPreflightAssignmentReport,
    "packageName" | "version" | "locale"
  >,
): string {
  const app =
    row.packageName === "com.zhiliaoapp.musically"
      ? "TikTok quốc tế"
      : row.packageName === "com.ss.android.ugc.trill"
        ? "TikTok châu Á"
        : row.packageName
          ? "Ứng dụng TikTok khác"
          : "Chưa xác định bản TikTok";
  const version = row.version?.trim()
    ? `Phiên bản ${row.version.trim()}`
    : "Chưa đọc được phiên bản";
  const locale = row.locale?.trim();
  let language = "Chưa đọc được ngôn ngữ";
  if (locale) {
    try {
      const display = new Intl.DisplayNames(["vi"], { type: "language" }).of(
        locale.replaceAll("_", "-"),
      );
      language =
        display && display !== locale
          ? `${display} (${locale})`
          : `Ngôn ngữ ${locale}`;
    } catch {
      language = `Ngôn ngữ ${locale}`;
    }
  }
  return [app, version, language].join(" · ");
}

export function publishPreflightProblem(
  issue: PublishExecutionIssue,
): PublishPreflightProblem {
  switch (issue.code) {
    case "automation_transport_conflict":
      return {
        title: "Kết nối điều khiển cần kiểm tra",
        action: issue.message,
      };
    case "composer_unmeasured":
    case "video_composer_unmeasured":
      return {
        title:
          issue.code === "video_composer_unmeasured"
            ? "Chưa hỗ trợ đăng video trên bản TikTok này"
            : "Chưa hỗ trợ luồng đăng trên bản TikTok này",
        action:
          "Dùng máy có bản TikTok đã được hỗ trợ, hoặc gửi phiên bản và ngôn ngữ hiển thị để hiệu chỉnh. Sau đó kiểm tra lại.",
      };
    case "sound_picker_unmeasured":
      return {
        title: "Chưa hỗ trợ chọn nhạc trên bản TikTok này",
        action:
          "Dùng máy có bản TikTok đã được hỗ trợ, hoặc gửi phiên bản và ngôn ngữ hiển thị để hiệu chỉnh phần chọn nhạc. Chưa thể đăng trên máy này.",
      };
    case "device_missing":
      return {
        title: "Máy đã mất kết nối",
        action:
          "Kiểm tra nguồn và kết nối USB/Wi-Fi, chờ máy xuất hiện trong Thiết bị rồi kiểm tra lại. Có thể ghép bài sang máy khác.",
      };
    case "tiktok_build_unreadable":
      if (
        /more than one.*TikTok|ambiguous|nhiều.*TikTok|hai.*TikTok/i.test(
          issue.message,
        )
      ) {
        return {
          title: "Máy có nhiều bản TikTok, chưa xác định bản cần dùng",
          action:
            "Mở đúng bản TikTok muốn đăng trên máy, để ứng dụng ở màn hình trước rồi bấm Kiểm tra lại. Không cần gỡ bản còn lại.",
        };
      }
      return {
        title: "Chưa đọc được bản TikTok trên máy",
        action:
          "Mở khóa máy, kiểm tra kết nối và mở TikTok, sau đó kiểm tra lại. Nếu lỗi còn lặp, xem Chi tiết kỹ thuật.",
      };
    case "push_media_unavailable":
      return {
        title: "Máy chưa sẵn sàng nhận nội dung",
        action:
          "Vào Chẩn đoán để kiểm tra Riviu Helper và quyền truy cập ảnh, sau đó kiểm tra lại.",
      };
    case "storage_insufficient":
      return {
        title: "Máy không đủ dung lượng trống",
        action:
          "Giải phóng dung lượng hoặc ghép bài sang máy khác, rồi kiểm tra lại.",
      };
    case "storage_unreadable":
      return {
        title: "Chưa đọc được dung lượng trống",
        action: "Mở khóa máy và kiểm tra kết nối, sau đó kiểm tra lại.",
      };
    case "media_unready":
      return {
        title: "Nội dung bài chưa đạt yêu cầu",
        action:
          "Kiểm tra caption không rỗng; mỗi bài gồm ảnh hoặc một MP4 được hỗ trợ. Sửa nội dung rồi kiểm tra lại.",
      };
    case "android_required":
      return {
        title: "Cần máy Android cho lượt đăng này",
        action: "Ghép bài với một máy Android đang kết nối rồi kiểm tra lại.",
      };
    default:
      return {
        title: "Có điều kiện chưa đạt",
        action:
          "Xem Chi tiết kỹ thuật để xác định điều kiện cần xử lý, sau đó kiểm tra lại.",
      };
  }
}

export function publishPreflightRowPassed(
  row: PublishPreflightAssignmentReport,
): boolean {
  return (
    row.issues.length === 0 &&
    [row.media, row.composer, row.soundPicker, row.storage].every(
      (status) => status === "pass",
    )
  );
}
