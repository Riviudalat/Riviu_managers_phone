## 10. Thiết bị và phiên bản mới

Thiết bị mới cần discovery/transport/session proof, capability app build và verifier
của action. Cùng model hoặc độ phân giải không là chứng nhận.

- Pixel calibration và runtime qualification là hai allowlist khác nhau. Không scale
  tọa độ iPhone8 sang màn khác hoặc bịa geometry khi đọc lỗi.
- Android hierarchy không bị giới hạn bởi pixel profile iPhone; vẫn cần identity,
  bounds, freshness và capability. Action chưa đo từ chối riêng.
- iOS UI Flow/Interaction còn gate riêng; manifest features không chứng nhận iPhone mới.
- ADB resolution theo AdbOrigin trong driver; bundled là fallback, không restart server
  để ép dùng binary khác.
- Đo trên canary được phép, thêm sanitized fixture và negative cases rồi kiểm production
  path đúng scope. Unit/fixture không cấp quyền Post/Send hoặc promote IPA.

[Số đo ban đầu](../archive/technical-snapshots-2026-10-01/10-thiet-bi-moi.md) ·
[Chẩn đoán](../../.claude/skills/riviu-device-diagnostics/SKILL.md).
