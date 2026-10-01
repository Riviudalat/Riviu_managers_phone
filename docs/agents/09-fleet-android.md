## 9. Android: contract driver và fleet

ADB giữ lifecycle, UiAutomator2 giữ UI/hierarchy, scrcpy giữ H.264 view, minicap giữ
ảnh, Riviu Helper giữ clipboard/media/native. [Android tools](../../sidecars/android/README.md)
sở hữu inventory/hash; không nhân bản danh sách pin.

- Helper và hai APK UiAutomator2 đã đóng gói và có install/repair path. USB debugging,
  authorization và Install via USB theo ROM vẫn cần operator.
- Clipboard đã có qua helper với khôi phục IME. Không dùng response rỗng của API
  thiếu quyền như capability; input/effect vẫn cần readback và ledger.
- content-desc/text/resource-id theo package/build/locale; snapshot chung tránh ghép
  identity/geometry khác generation. Unknown không là false/absent.
- Legacy observation mặc định, enriched opt-in sau phép đo. Không cấm hierarchy
  chung theo số đo lịch sử hoặc bật enriched toàn fleet vì fixture xanh.
- Foreground dùng nhiều nguồn dumpsys và package inventory; hai TikTok cần binding.
  Không chọn package đầu tiên khi focus mơ hồ.
- Override size thắng Physical nếu có. Byte output/PNG magic không đi qua decoder text.
- Retry ADB opt-in theo idempotency, không phát lại input/install/launch mất ACK.
- Không kill adb-server. Discovery/route5037/5038 theo serial; scan lỗi không là roster
  rỗng đáng tin. Một backend lỗi không được xóa thiết bị của backend kia.
- Các engine đi qua control plane. IdleSweep thấp ưu tiên, không tranh foreground.
  Không dùng direct probe khi desktop giữ cùng USB.

[Snapshot đo Android](../archive/technical-snapshots-2026-10-01/09-fleet-android.md)
không là danh sách thiếu chức năng hiện tại hoặc chứng nhận mọi ROM/build.
