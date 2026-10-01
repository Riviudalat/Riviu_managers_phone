## 3. Kiến trúc và ranh giới

Nguồn hiện hành: [developer guide](../developer-guide.md),
[contract thực thi](../development/contracts.md) và source tương ứng.

- UI/Local API/MCP đi qua commands và `DeviceControlPlane`; không mở driver thứ hai
  để né admission/ownership. Context/lease bind đúng UDID/token; cleanup không thu
  hồi effect đang dispatch.
- SQLite lưu intent trước effect, kết quả và receipt theo revision/CAS. Không giữ
  transaction qua HTTP/thiết bị; executor giới hạn reader/writer/queue trước spawn.
- Preview khác evidence: Android view H.264/scrcpy qua worker decoder; observation
  hierarchy và minicap giữ freshness/session/generation. iOS giữ session trước MJPEG.
- Observation thiếu dữ liệu là unknown, không là absent/false. Positive match đòi
  thuộc tính thật và target duy nhất; stale/ambiguous không cấp quyền tap.
- Nurture, Interaction và Publish giữ verifier riêng. Submitted khác publication
  verified; outbox Sheet/cleanup không quay lại Post. Không đếm theo ACK.
- Flow compiler/IR/runtime, revision/library snapshot và ledger đã triển khai;
  action vẫn phải preflight capability. iOS UI Flow chưa runtime-qualified như Android.
- `DevicePlatform` khác `SocialNetwork`. TikTok đã implement; Instagram/Threads là
  seam từ chối effect. My Apps khác Flow primitive và Điều phối.
- Process supervisor theo thiết bị, Windows Job Object và fingerprint bảo vệ cleanup.
  Không pkill theo tên, không cancel gesture đã phát chỉ để hết timeout.
- Capability catalog và geometry profile chỉ mở theo measurement/proof đúng tuple.
  Trần cấu hình concurrency không phải capacity đã nghiệm thu.

Roadmap/checkpoint/số đo được giữ trong
[snapshot kiến trúc](../archive/technical-snapshots-2026-10-01/03-kien-truc.md).
Không dùng trạng thái chưa triển khai/PASS trong snapshot để quyết định code hiện tại.
