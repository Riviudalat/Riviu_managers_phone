const statusLabels: Record<string, string> = {
  ready: "Sẵn sàng",
  busy: "Đang bận",
  disconnected: "Mất kết nối",
  pairing: "Đang ghép nối",
  connected: "Chưa chuẩn bị",
  preparing: "Đang chuẩn bị",
  error: "Cần kiểm tra",
};

export function machineStatusLabel(status: string) {
  return statusLabels[status] ?? "Chưa sẵn sàng";
}
