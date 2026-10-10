const statusLabels: Record<string, string> = {
  // Roster `ready` alone is not proved control; "Sẵn sàng" belongs to deviceOperationalView.
  ready: "Đã chuẩn bị",
  busy: "Đang bận",
  disconnected: "Mất kết nối",
  pairing: "Đang ghép nối",
  connected: "Chưa chuẩn bị",
  preparing: "Đang chuẩn bị",
  error: "Cần kiểm tra",
};

export function machineStatusLabel(status: string) {
  return statusLabels[status] ?? "Chưa rõ trạng thái";
}
