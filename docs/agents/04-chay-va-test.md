## 4. Chạy và kiểm chứng

Dùng [testing runbook](../development/testing.md) và [build/release](../development/build-release.md).
Pin nằm trong rust-toolchain.toml, lockfiles và CI, không theo tool từng cài ở một PC.
CI hiện chạy thủ công và có e2e; browser e2e không là USB test.

- UI smoke cô lập scratch/credential/worker theo StartupPolicy. Mock driver đơn lẻ
  không cô lập DB. Không mở app thật cạnh controller đang giữ máy.
- Diagnostic binary cần feature `diagnostics`; build không cho phép run.
  Harness Nuôi/Interaction có thể Like/Comment/Follow thật, không là smoke chỉ đọc.
- Canary chỉ exact device/effect đã duyệt, đúng controller. Đọc flags, giới hạn,
  cleanup và evidence trước invocation; không copy serial/tọa độ lịch sử.
- Kill agent/ADB/reboot không phải bước dọn chung. Chẩn đoán lớp lỗi trước;
  recovery cần scope riêng, không cắt upload/effect chưa rõ.
- iOS signing/provisioning phải kiểm đúng artifact/UDID; hạn cert không suy từ số
  ngày của lần đo cũ. Full agent repair khác stock WDA re-sign legacy.

[Snapshot môi trường iOS](../archive/technical-snapshots-2026-10-01/04-chay-va-test.md)
chỉ để điều tra lịch sử, không dùng như recipe onboarding hiện tại.
