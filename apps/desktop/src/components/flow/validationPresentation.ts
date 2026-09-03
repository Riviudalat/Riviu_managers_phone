import type { FlowValidationIssue } from "../../types";

const ISSUE_LABELS: Record<string, string> = {
  ActionDisabled: "Hành động này đang bị tắt.",
  ConfigInvalid: "Giá trị cấu hình chưa hợp lệ.",
  ConfigOutOfRange: "Giá trị cấu hình vượt giới hạn.",
  Cycle: "Flow có vòng lặp không được hỗ trợ.",
  DisconnectedNode: "Bước này chưa được nối vào Flow.",
  DuplicateEdgeId: "Flow có đường nối bị trùng.",
  EndCount: "Flow phải có đúng một bước kết thúc.",
  EntryNodeInvalid: "Điểm bắt đầu của Flow chưa hợp lệ.",
  FeatureNotEnabled: "Hành động này chưa được bật.",
  InvalidDegree: "Số đường nối của bước chưa hợp lệ.",
  InvalidPort: "Cổng nối của bước chưa hợp lệ.",
  NonFiniteCoordinate: "Tọa độ phải là một số hữu hạn.",
  NonFiniteNumber: "Hãy nhập một số hữu hạn.",
  StartCount: "Flow phải có đúng một bước bắt đầu.",
  UiSessionTargetRequired: "Flow thao tác màn hình phải bắt đầu bằng mở ứng dụng.",
  UnknownEdgeNode: "Đường nối đang trỏ tới một bước không tồn tại.",
  ValidationTransportFailed: "Không nhận được kết quả kiểm tra Flow.",
  WaitOutOfRange: "Thời lượng chờ vượt giới hạn.",
};

export function flowValidationMessage(issue: FlowValidationIssue): string {
  return ISSUE_LABELS[issue.code] ?? (issue.field
    ? "Giá trị cấu hình chưa hợp lệ."
    : issue.nodeId
      ? "Bước này chưa hợp lệ."
      : "Flow chưa hợp lệ.");
}
