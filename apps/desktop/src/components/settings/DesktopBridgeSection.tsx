/** Which driver mode the desktop bridge came up in. */
export function DesktopBridgeSection({ mode }: { mode: string }) {
  const label = mode === "full"
    ? "Agent hợp nhất"
    : mode === "mock"
      ? "Dữ liệu mô phỏng"
      : mode === "stock"
        ? "Agent iOS dự phòng"
        : "Trạng thái chưa nhận diện";
  return (
    <section className="settings-section">
      <h3>Kết nối thiết bị</h3>
      <p className="hint">Đường điều khiển đang dùng: {label}.</p>
      <details className="settings-details" aria-label="Chi tiết kết nối thiết bị">
        <summary>Chi tiết kết nối thiết bị</summary>
        <p className="hint">Mã trạng thái: <code>{mode}</code></p>
      </details>
    </section>
  );
}
