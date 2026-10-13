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
- adb-server chỉ được khởi động lại theo mục "ADB server treo (HX-ADB)" bên dưới; người
  vận hành chỉ restart tay khi không có controller chạy. Discovery/route5037/5038 theo
  serial; scan lỗi không là roster rỗng đáng tin. Một backend lỗi không được xóa thiết bị của backend kia.
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
  thì thử lại đúng lần đẩy đó một lần sau 2 s. Server treo hẳn thì theo HX-ADB bên dưới.

### Giảm tải máy khi Riviu tự động (HX-QUIET)

- Số đo 13–14/10/2026, 30 máy SM-G955F/N Android 9 (`target/device-acceptance/
  fleet-health-20261014-*.tsv`, `controllers/controller-r105.stdout.txt`): khi 30 máy cùng
  soạn, UiAutomator2 `/source` p50 6,7 s, p90 12,7 s, max 18,3 s (bình thường <1 s);
  `surfaceflinger` 61 % CPU; load average 7–8 trên máy 8 nhân; lượt đo tải `adb shell`
  timeout 2 s; "waited for an adb slot" liên tục (cap 12 slot, `adb.rs`; nhãn cũ gộp ba
  tầng chờ, xem mục HX-TELEMETRY). Host không nghẽn (20 nhân, Riviu ~10 % CPU): nghẽn ở
  máy và adb server. Trên mỗi máy lúc soạn chạy đồng
  thời TikTok, UiAutomator2, scrcpy (Tile 480 px/1,2 Mbps/10 fps), có lúc minicap/screencap.
- Hai cơ chế trong code: (1) job đăng bài giữ view scrcpy của chính máy đó tắt suốt lượt
  (`QuietView`, `AndroidDriver::hold_view`; tile hiện "Đang đăng bài, tạm dừng hình");
  (2) một ngân sách `PUBLISH_PHONE_WORK_CAP` = 10 máy cho mọi pha đăng bài trên máy, kể cả
  quan sát xác minh. Chi tiết ở `docs/development/contracts.md` (tìm "HX-QUIET").
- Kiểm trên máy thật sau khi đổi: cùng số máy và build TikTok, so với r105 (a) `/source` ms
  p50/p90 từ log driver, (b) tỉ lệ dòng đăng được ở lượt đầu (không qua recovery), (c)
  `surfaceflinger` CPU và load average từ lượt đo tải. Phải thấy tile của máy đang đăng dừng
  ở khung cuối và bật lại sau khi lượt xong; máy rảnh vẫn Tile 480.
- Tham chiếu GenFarmer 2.6.2: stream mặc định 176 px/25–50 Kbps, High 288 px/1 Mbps, có
  `autoScreenOff`, `ADB_GLOBAL_CONCURRENCY = 16`. Riviu không hạ Tile xuống dưới 480 px vì
  176 px bị Redmi API 35 từ chối `MediaCodec.configure` và 320 px ra SPS `avc1.42000D`
  WebView2 có thể từ chối (`scrcpy.rs`).
- Đề xuất chưa bật (chỉ ghi để cân nhắc): tắt màn hình vật lý khi máy đang tự động
  (scrcpy `power_off_on_close`/`SET_DISPLAY_POWER`) để giảm nhiệt và throttle. Rủi ro: trên
  Android 9 màn hình tắt làm UiAutomator2 `/source` và `screencap` có thể trả cây/ảnh rỗng
  hoặc đen, `minicap` ngừng ra khung; TikTok có thể tạm dừng video/upload khi màn hình tắt;
  bật lại màn hình đi qua `KEYCODE_POWER` có thể dựng khóa màn hình (baseline giữ lock
  screen tắt nhưng phải kiểm). Chỉ thử trên 1–2 máy canary với bằng chứng `/source`,
  `screencap` và một lượt đăng đầy đủ trước khi xét bật rộng.

### Đọc số liệu chờ adb/agent (HX-TELEMETRY)

Mục đích: tách host nghẽn (hàng đợi adb trên PC) khỏi máy chậm (lệnh chạy lâu trên máy).
Code: `crates/android-driver/src/telemetry.rs`, `enter_adb_slot` trong `adb.rs`,
`send_once_with_timeout`/`observation_request` trong `agent.rs`.

- Mỗi lệnh adb đi qua ba tầng theo thứ tự, mỗi tầng đo riêng: `adb.serial_queue` (hàng
  đợi riêng của máy, một lệnh mỗi lúc), `adb.transfer_cap` (chỉ pull/push, tối đa 4),
  `adb.global_slot` (cap chung 12). Sau đó `adb.exec <verb>` là thời gian chạy thật, tính từ
  lúc giữ đủ slot.
- Agent: mọi call theo route `agent <METHOD> <route>`, không chỉ call chậm. UUID/số trong
  route thành `:id`; không ghi serial, payload, URL hay session. Kết quả: `ok`, `chậm`
  (quá ngân sách warning của route: 5 s cho cây, 500 ms cho thao tác), `hết_giờ`, `lỗi`
  (transport/spawn/adb exit ≠ 0), `hủy` (future bị bỏ giữa chừng).
- Dòng INFO "Độ trễ adb/agent N s gần nhất …" in mỗi ~60 s khi có lưu lượng: mỗi series
  `n`, `p50≤`, `p90≤` (cận trên ô histogram, không vượt max), `max` đúng, và số kết quả
  khác `ok`. Cửa sổ reset sau mỗi dòng, nên so đợt bằng cách so các dòng cùng giờ chạy.
  Tối đa 64 series mỗi cửa sổ; tên mới sau đó gộp vào `(khác)`.
- Warning chờ ≥500 ms ghi `lane=` (tầng chiếm nhiều thời gian nhất), `serial_queue_ms`,
  `transfer_cap_ms`, `global_slot_ms`, `global_in_use`, `transfers_in_use`. Chỉ
  `lane=adb.global_slot` mới là "host đang chạy đủ số lệnh adb cùng lúc". Log trước bản
  này ghi "waited for an adb slot; the host is running its cap" cho tổng ba tầng, nên số
  3.438 warning của r105 không chứng minh cap 12 đầy.
- Cách đọc: `adb.global_slot` p90 cao → host/adb server nghẽn, hạ admission trước, không
  tăng cap mù. `adb.serial_queue` cao mà `global_slot` thấp → nhiều lệnh dồn vào cùng
  máy. `adb.exec`/`agent GET /source` cao khi các hàng đợi thấp → máy chậm; kiểm tải/
  hierarchy trên canary (mục "Sức khỏe máy trước code").
- Chưa có số đo đợt chạy nào bằng bản này; ngưỡng ô histogram và cap 64 series chưa đo.

### ADB server treo (HX-ADB)

Chủ sở hữu bỏ quy tắc "không kill adb-server" ngày 14/10/2026. Bằng chứng r107 (`fd05d8ee`,
30 máy trên adb 34.0.5 của GenFarmer, client Riviu 37.x): năm máy hết hạn quan sát trong 8 s,
`adb shell timed out after 2s`, `could not read ok from ADB Server` khi tiến trình server vẫn
sống; vài phút sau `adb shell echo` lại 230–290 ms.

- Phát hiện (`adb_server.rs`): lỗi phía máy tính (`could not read ok`, `cannot connect to
  daemon`, `daemon not running`, `failed to start daemon`, `timed out after`) trên ≥3 máy khác
  nhau trong 20 s. Một máy offline/chậm chỉ lỗi trên serial của nó nên không tính. Nghi ngờ
  chưa đủ: ba lần đọc `host:devices-l` cách nhau 1 s đều không trả lời mới gọi là treo.
- Khởi động lại (`AdbProgram::recover_hung_server`, chạy bởi
  `apps/desktop/src-tauri/src/adb_server_recovery.rs` mỗi 5 s, không chạy khi worker tự động
  bị đóng băng): `kill-server` rồi `start-server` bằng chính adb Riviu đang dùng (bundled ở
  bản đóng gói), cùng cổng `-P`. Server bỏ qua `kill-server` thì chỉ tắt pid đang nghe cổng
  đó (`netstat`), không tắt theo tên `adb.exe`. Cooldown 45 s (như GenFarmer
  `MIN_MS_BETWEEN_KILLS`), tối đa 4 lần mỗi giờ (cap chưa đo). Nhật ký: intent
  `adb.server.restart` trước lệnh, rồi `.done`/`.failed`; `.deferred`/`.refused` ghi một
  lần mỗi đợt.
- Ranh giới an toàn (`Database::adb_restart_blockers`): chờ khi còn publish `posting` có
  `effect_intent` (đã claim Post), interaction `sending`, action `armed`, hoặc Flow
  `externalEffect` ở `intentCommitted`/`effectDispatched`, cập nhật trong 10 phút (cap: dòng
  cũ hơn là worker đã chết, không được chặn mãi). Đọc sổ lỗi = chờ. Sheet là HTTP, không đi
  qua adb, nên restart không cắt được lần ghi Sheet.
- Sau restart: mọi forward/stream/UiAutomator session cũ mất. Đó là lỗi transport như khi
  phần mềm khác restart server: helper nối lại forward (`reconnect_runtime`, `agent_ready`),
  view keeper mở lại stream, không phát lại thao tác; xác minh chỉ đọc lại. Trong lúc restart
  inventory không báo "phần mềm khác", background start chờ; UI nhận
  `adbServerNotice` `restarted`.
- Lúc khởi động controller (`replace_foreign_server_at_bootstrap`, trước mọi job): server
  đang chạy khác bản `Version` với client (đọc đường dẫn tiến trình nghe cổng rồi chạy
  `<server> version`) thì ghi cảnh báo và thay bằng adb của Riviu, thay vì giữ "selected from
  existing transport inventory". Không xác định được server thì giữ nguyên và ghi log.
- Cổng tường minh (`RIVIU_ADB_SERVER_PORT`, `ADB_SERVER_SOCKET`, ...) không có watch nên
  không tự restart.

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
  ≥ 60% CPU trong 2 lần đo liên tiếp khi swap dùng > 1,0 GB; hoặc MemAvailable < 600 MB;
  hoặc swap dùng > 1,0 GB cùng MemAvailable < 900 MB. surfaceflinger một mình không đủ:
  ngày 12/10/2026 máy `ce041714d44264230d` ở composer, đang bị Riviu chụp màn hình, đo 82%
  rồi 62% với RAM trống 3087 MB, swap 468 MB, không có lệnh helper chậm nào, vẫn bị từ chối.
  Các máy bị từ chối vì surfaceflinger mà swap > 1 GB thì có 7–21 lệnh helper chậm (tới 15–28 s).
- Lúc đo: cổng nhận việc ở `DeviceControlPlane::ensure_load_admissible` (khi mở phiên app
  cho owner Script/Interaction/Nurture, và trước khi chuyển media của Đăng bài), và mỗi
  ~60 s trên máy đang giữ việc qua vòng quét roster (`device_load.rs`). Lượt đo chờ slot
  adb như mọi lệnh khác.
- Tầng 1, dọn nhẹ: `am kill-all`, `am force-stop` cho danh sách đã duyệt
  (`com.facebook.services`, `com.facebook.appmanager`), `pm trim-caches 2G`. Không bao giờ
  `pm clear`, TikTok/Trill, Riviu Helper, UiAutomator2 hay gói hệ thống. Chỉ chạy khi máy
  không có bước effect đang chạy: dưới lease của job lúc nhận việc, hoặc lease Repair khi
  máy rảnh và sổ ghi không còn việc chưa rõ/đang chạy (tối đa một lần mỗi 30 phút).
- Tầng 2, `adb reboot`: khi `load_reboot_block_reason` không còn việc chưa rõ hoặc đang
  chạy và chưa reboot trong 6 giờ. Việc Đăng bài chỉ đang chờ máy (queued, đến giờ,
  transferring, imported: chưa có ý định Post) không chặn reboot; bài đang tải/chưa rõ,
  Tương tác/Flow/Nuôi/job/điều phối còn sống vẫn chặn. Hai nơi chạy tầng 2:
  - ngay ở cổng nhận việc của job (`reboot_for_job`, dưới lease của chính job, trước mọi
    effect): tầng 1 không hết, hoặc đã dọn nhẹ trong 30 phút, thì reboot luôn rồi đo lại,
    trong vài phút chứ không chờ 30 phút. Chạy cả khi chế độ nghiệm thu đóng băng worker nền
    (12/10/2026: monitor nền bị đóng băng ở mọi controller r91/r93 nên không máy nào được
    reboot);
  - monitor nền khi máy rảnh, giữ lease Repair suốt lúc chờ.
  Chờ `sys.boot_completed` (≤120 s, boot id đổi), áp lại baseline, khởi động lại helper
  (bước chuyển media vẫn tự kiểm helper). Không được reboot thì đánh dấu "quá tải — chờ
  người vận hành".
- Đăng bài vẫn bị từ chối ở cổng này settle `failedBeforeDispatch` với `device_overloaded`
  (retryable). Nuôi/Tương tác chạy tiếp nếu tầng 1 đạt; sổ ghi của chính chúng chặn tầng 2.
- Baseline giữ auto-rotate tắt (`accelerometer_rotation=0`). `monkey -p … 1` bật lại
  auto-rotate (đo trên SM-G955F, Android 9, 10/10/2026), nên Riviu mở app bằng
  `am start` trên component launcher đã resolve, không dùng `monkey`.
- Sau reboot: kiểm lại auto-rotate (đã bật lại sau reboot) và khóa màn hình. Helper được
  tự đối soát khi máy kết nối lại hoặc ở admission của job (pending → prepare → execute
  dưới lease Repair, không cần nút); "Khôi phục helper" chỉ là đường dự phòng khi kế hoạch
  tự từ chối. Mỗi lần máy từ offline/lỗi trở lại `device` mà lỗi helper còn chốt được đối
  soát lại, tối đa `MAX_RECONNECT_RECONCILES` = 3 lần (chưa đo; helper đạt thì đếm lại)
  (`android_helper_setup.rs`).
- Bàn phím gốc là intent bền: `with_ime_mode` ghi `pending.previousIme` và
  `helperWasEnabled` vào `ime-checkpoint.json` trước `ime set`. Khi bàn phím mặc định vẫn
  là `RiviuIme`, maintenance prepare lấy bàn phím gốc từ checkpoint của đúng owner (plan
  có thêm `imeRestore`), không từ máy và không đoán. Execute ghi intent, trả bàn phím
  (`ime set`, đọc lại; `ime disable` nếu trước đó helper IME chưa bật) **trước**
  `force-stop`, vì Android bind lại IME đang chọn và sẽ khởi động lại helper. Job
  clipboard cũ (helper trả 400 sau khi chạy lại) được lưu vào `maintenance/<id>/` ở trạng
  thái `archivedUnresolved`, không replay. Với client còn cache, nợ clipboard chỉ mở
  maintenance khi `owner_restart_proof` chứng minh tiến trình helper đã mất (boot id,
  vắng tiến trình hoặc pid khác). Không có proof thì vẫn `cleanup_pending`. Checkpoint cũ
  không có `previousIme` vẫn là `ime_unproved`, và thông báo nêu serial cùng lệnh
  `adb -s <serial> shell ime set …`. Đường dự phòng "bàn phím duy nhất còn bật" chỉ dành
  cho dữ liệu đó (13/10/2026: máy 23, 32, 38).
- Lỗi adb mất máy lúc lấy helper (`device offline`, `device '…' not found`, daemon không
  kết nối được, timeout: `adb::is_transport_lost`) là lỗi kết nối `category=adb_unreachable`,
  thử lại được, kể cả khi lỗi mang fence `HelperRecoveryRequired` của lần reconnect owner
  cũ. Nó không chứng minh gì về owner cũ nên không reclaim; owner chỉ được bỏ khi máy online
  và `owner_restart_proof` thấy boot id đổi/tiến trình mất.
- Máy kẹt `offline` sau reboot do Riviu (`issue_load_reboot`: tải, ngôn ngữ). Số đo:
  13/10/2026 SM-G955F (`ce051715e15b2c2e02`, Android 9) boot xong rồi ~10 s sau báo offline
  ~20 phút, đến khi chạy tay `adb reconnect offline`. Offline liên tục ≥60 s trong 15 phút
  sau reboot (cả hai là cap chưa đo) thì Riviu chạy một lần `adb -s <serial> reconnect` cho
  lần reboot đó: ghi intent `device.load.reconnect` trước, rồi `.done`/`.failed`. Một máy
  offline không bao giờ là lý do `kill-server` hay `reconnect` toàn bộ. Nơi đếm: vòng quét roster (chạy cả khi worker
  nền bị đóng băng), chờ boot, và `wait_for_adb_online_after_reboot` (≤100 s) trong hai
  đường reboot của `device_load.rs`. Đường của job thử lấy helper thêm một lần khi máy
  online lại.
- adbd chạy root (`id -u` = 0, thường do GenFarmer `adb root`) được tự sửa: một
  `adb -s <serial> unroot` mỗi boot id dưới lease Repair, ghi intent trước, chờ ≤45 s
  (chưa đo trên máy). Root không phải lý do restart adb-server; vẫn root thì báo tắt GenFarmer và
  khởi động lại máy. Lúc nhận phiên, `recovery_required` trên máy root từng che
  `shell_root` (bản 0.2.82, PC thứ hai, 12/10/2026).

[Snapshot đo Android](../archive/technical-snapshots-2026-10-01/09-fleet-android.md)
không là danh sách thiếu chức năng hiện tại hoặc chứng nhận mọi ROM/build.
