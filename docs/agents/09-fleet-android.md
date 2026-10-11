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
- Script ngoài controller (`scripts/device-acceptance/`) phải gọi đúng adb mà controller
  dùng (`RIVIU_ADB_PATH`, rồi PATH). Gọi adb client khác version sẽ restart adb-server
  dưới controller.
- Các engine đi qua control plane. IdleSweep thấp ưu tiên, không tranh foreground.
  Không dùng direct probe khi desktop giữ cùng USB.

### UiAutomator của công cụ khác và quyền TikTok

- Số đo: campaign `6e0bfb7c` (12/10/2026 02:28, 20 máy, 0 bài đăng) có UiAutomator của
  GenFarmer (`com.genfarmer.uiautomator.test/androidx.test.runner.AndroidJUnitRunner`)
  còn chạy trên 15/20 máy và giành UiAutomation của Riviu.
- Khi khởi động hoặc restart agent, nếu mọi instrumentation lạ đều thuộc
  `KNOWN_CONFLICTING_AUTOMATION` (`crates/android-driver/src/driver/agent.rs`: hai gói
  GenFarmer) thì Riviu `am force-stop` hai gói đó một lần, chờ ≤5 s đến khi ActivityManager
  không còn ghi nhận, rồi chạy tiếp. Chủ lạ khác vẫn bị từ chối. Phiên chẩn đoán không dừng gì.
  Không bao giờ dừng TikTok, Riviu Helper hay gói hệ thống.
- Mỗi phiên app TikTok, Riviu tự `pm grant --user 0` quyền ảnh/video: `READ_/WRITE_EXTERNAL_STORAGE`
  (Android ≤ 12) hoặc `READ_MEDIA_IMAGES`/`READ_MEDIA_VIDEO` (Android ≥ 13). Lý do: hộp thoại
  "Allow TikTok to access photos, media, and files" mở trên thư viện và làm hỏng bài đăng.
  Bỏ qua lỗi "not a changeable permission"/"has not requested". Không bao giờ revoke hay
  `pm clear`.
- Đẩy media lúc chuẩn bị Đăng bài (trước mọi Post) gặp `cannot connect to daemon` / `10060`
  thì thử lại đúng lần đẩy đó một lần sau 2 s. Riviu không restart adb-server.

### Sức khỏe máy trước code

Khi chỉ một máy có root không đọc được hoặc tap bị nuốt, so máy đó với một máy cùng
đời trước khi sửa code: `top -b -n 1` và `/proc/meminfo` (swap). Choreographer
"Skipped NN frames" trong logcat là dấu hiệu cùng loại.

- Số đo: máy 44 (`ce031713d903001b01`, SM-G955F, 10/10/2026) có root bảng nhạc không
  đọc được và tap chỉ hiện sau ≥110 s. surfaceflinger ~92% CPU trong khi máy cùng đời
  ~3%, swap dùng 1,1 GB, Choreographer báo bỏ khung. `adb reboot` hết lỗi.
- Chỉ reboot khi máy đó không còn Post/Send nào ở trạng thái chưa rõ.
- Riviu tự đo việc này (`crates/core/src/device_control/load.rs`, mẫu đo ở
  `crates/android-driver/src/load.rs`): một lượt shell ≤2 s gồm `top -b -n 1 -m 10`,
  `/proc/meminfo`, `/proc/uptime`, nhiệt độ pin; không dùng `uiautomator dump`. Không đọc
  được là chưa rõ, không bao giờ là quá tải.
- Ngưỡng quá tải (đo trên máy 44 và máy cùng đời 47/39, 10/10/2026): surfaceflinger
  ≥ 60% CPU trong 2 lần đo liên tiếp; hoặc MemAvailable < 600 MB; hoặc swap dùng > 1,0 GB
  cùng MemAvailable < 900 MB.
- Lúc đo: cổng nhận việc ở `DeviceControlPlane::ensure_load_admissible` (khi mở phiên app
  cho owner Script/Interaction/Nurture, và trước khi chuyển media của Đăng bài), và mỗi
  ~60 s trên máy đang giữ việc qua vòng quét roster (`device_load.rs`). Lượt đo chờ slot
  adb như mọi lệnh khác.
- Tầng 1, dọn nhẹ: `am kill-all`, `am force-stop` cho danh sách đã duyệt
  (`com.facebook.services`, `com.facebook.appmanager`), `pm trim-caches 2G`. Không bao giờ
  `pm clear`, TikTok/Trill, Riviu Helper, UiAutomator2 hay gói hệ thống. Chỉ chạy khi máy
  không có bước effect đang chạy: dưới lease của job lúc nhận việc, hoặc lease Repair khi
  máy rảnh và sổ ghi không còn việc chưa rõ/đang chạy (tối đa một lần mỗi 30 phút).
- Tầng 2, `adb reboot`: chỉ khi máy rảnh, `app_completion_block_reason` không còn việc
  chưa rõ hoặc đang chạy, và chưa reboot trong 6 giờ. Chờ `sys.boot_completed` (≤120 s,
  boot id đổi), áp lại baseline, giữ lease Repair suốt lúc chờ; helper được chuẩn bị lại
  khi máy kết nối lại. Không được reboot thì đánh dấu "quá tải — chờ người vận hành".
- Đăng bài bị từ chối ở cổng này settle `failedBeforeDispatch` với `device_overloaded`
  (retryable). Nuôi/Tương tác chạy tiếp nếu tầng 1 đạt.
- Baseline giữ auto-rotate tắt (`accelerometer_rotation=0`). `monkey -p … 1` bật lại
  auto-rotate (đo trên SM-G955F, Android 9, 10/10/2026), nên Riviu mở app bằng
  `am start` trên component launcher đã resolve, không dùng `monkey`.
- Sau reboot: kiểm lại auto-rotate (đã bật lại sau reboot) và khóa màn hình, rồi chạy
  "Khôi phục helper" (pending → prepare → execute). Bỏ bước này thì job kế tiếp lỗi
  `media_transfer_failed` / `recovery_required`.

[Snapshot đo Android](../archive/technical-snapshots-2026-10-01/09-fleet-android.md)
không là danh sách thiếu chức năng hiện tại hoặc chứng nhận mọi ROM/build.
