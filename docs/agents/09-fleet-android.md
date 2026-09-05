## 9. Fleet Android (09/08/2026)

Ổ cắm cho việc này được chừa từ ngày đầu — bản thiết kế gốc
(`docs/archive/specs/2026-07-25-riviu-managers-phone-design.md:7`) viết
*"…multiple iPhones, with Android deferred behind a `DeviceDriver` trait"*.
`crates/android-driver` lấp chỗ đó và **không phải sửa `DeviceDriver`/`UiSession`**.

**Không viết Riviu Agent APK trên Android để “giống iPhone”.** Agent iPhone là
XCTest runner, không phải admin; Android không root cũng không có “toàn quyền”.
Nuôi và tương tác đã chạy trên `adb` + uiautomator2 + scrcpy/minicap. Helper
`com.riviu.agent` (§9.52) chỉ bù clipboard / MediaStore — không thay server UI,
không phải bàn phím mặc định. (Mục này từng viết *"chưa pin binary cho tới khi có SDK
build"* — đã pin từ đó; xem sửa ở §9.52.)

Số đo đầy đủ ở `docs/archive/reports/2026-08-09-android-probe-report.md`. Những điều không được
đoán lại:

- **Hai tầng, chia theo số đo, không theo thẩm mỹ.** `adb.rs` chỉ dùng cho vòng
  đời (cài/mở/dừng/khởi động lại): mỗi lệnh tốn 1–2 giây trên fleet Galaxy S8+.
  `agent.rs` nói HTTP với `appium-uiautomator2-server` thường trú qua
  `adb forward`: click 130–280 ms, tìm element 609 ms, đọc thuộc tính 241 ms.
- **Đừng dump cả cây accessibility trong vòng điều khiển.** Agent **không** làm
  nó rẻ hơn (3403 ms so với 2693–4239 ms qua CLI) vì chi phí nằm ở duyệt và
  serialize cây, không ở khởi động công cụ. Hãy truy vấn đúng element cần — đó
  đã là hình dạng của `find_and_tap`/`assert_visible`/`read_text`.
- **Locator ưu tiên `content-desc`, KHÔNG phải `resource-id`.** `resource-id`
  của TikTok bị R8 obfuscate (`a1p`, `ty9`, `ebz`) và đổi theo bản build;
  `content-desc` thì ngữ nghĩa và **mã hoá cả trạng thái**:
  `Like` ⇄ `Video liked`, `Read or add comments. 15 comments`.

  > **SỬA 10/08/2026 — `content-desc` KHÔNG phải tiếng Anh bất kể ngôn ngữ UI.**
  > Câu đó đo trên bản TikTok *global* (`com.zhiliaoapp.musically`). Trên bản SEA
  > `com.ss.android.ugc.trill` với UI tiếng Việt (Redmi Note 12, Android 15),
  > dump hierarchy thật cho thấy nhãn **bị dịch**:
  >
  > | AGENTS.md ghi | Thực tế trên `trill` + tiếng Việt |
  > |---|---|
  > | `Like` | `Thích` và `Thích video. 1Tr lượt thích` |
  > | `Video liked` | *(chưa đo trạng thái đã-thích)* |
  > | `Read or add comments. N comments` | `Đọc hoặc viết bình luận. 21,1K bình luận` |
  > | `For You` | `Đề xuất` |
  > | `Tap to watch LIVE` | *(không có nhãn nào chứa)* |
  > | `Follow <name>` | `Follow Hoàng Sơn` — **chỉ cái này giữ tiếng Anh** |
  >
  > Nhãn khác đo được: `Chia sẻ video. 310,6K lượt chia sẻ`, `Thêm hoặc xóa video
  > này khỏi mục Yêu thích.`, `Chú thích`, `Hồ sơ <tên>`, `Bạn bè`, `Đã follow`,
  > `Tìm kiếm`, `Trang chủ`, `Cửa hàng`, `Quay`, `Hộp thư`, `Hồ sơ`.
  >
  > **Hệ quả:** mọi `Locator::Description` tiếng Anh trong nurture/interaction sẽ
  > **absent** trên máy UI tiếng Việt — G1 probe đo được `find("Like")` absent,
  > `find("Video liked")` absent, `assert_visible("For You")` fail, dù rail vẫn
  > hiển thị đầy đủ trên màn hình. Cách mã hoá trạng thái trong nhãn vẫn đúng,
  > chỉ là **theo locale**. Muốn chạy fleet trộn locale thì locator phải tra theo
  > (TikTok package × ngôn ngữ UI), không hard-code một chuỗi. `package.json` của
  > GenFarmer có cột `apps.locale_input` — họ gặp đúng vấn đề này.
  >
  > **Đã có catalog: `riviu_core::tiktok_labels`.** Nhãn là **dữ liệu đo**, khoá theo
  > `(package × ngôn ngữ UI)`, và `labels_for()` trả `None` cho cặp chưa đo — `None`
  > **phải nghĩa là từ chối**, đúng như `CALIBRATED_LAYOUTS` (§10). Đừng thay bằng
  > nhãn của ngôn ngữ khác: locator đó khớp rỗng và đọc thành "chưa thích", tức một
  > câu trả lời **sai**, không phải câu trả lời thiếu.
  > - Mỗi nhãn mang `Exact` hoặc `Contains` như đo được. `Contains` không phải cho
  >   tiện: nhãn bình luận nhúng số đếm — đo được `21,1K bình luận` rồi `697 bình
  >   luận` trên hai video khác nhau, nên exact **không bao giờ** khớp.
  > - Cái gì chưa đo thì để `None`, **không đoán**. Hiện `Liked` và `LiveRoom` của
  >   bản `vi` là `None`: probe thấy `Thích` present và `Đã thích` absent trên video
  >   *chưa* thích — điều đó vừa hợp với nhãn đúng vừa hợp với nhãn sai, nên chưa
  >   chứng minh được gì. Muốn chốt phải đo trên video **đã** thích.
  > - **Đọc ngôn ngữ UI từ `persist.sys.locale`, KHÔNG phải `ro.product.locale`.**
  >   Đo trên Redmi Note 12: `persist.sys.locale=vi-VN` còn
  >   `ro.product.locale=en-GB` (mặc định xuất xưởng). Đọc sai cái là chọn nhãn tiếng
  >   Anh cho máy UI tiếng Việt — **bẫy dòng-đầu thứ ba** cùng loại với `wm size` và
  >   `mCurrentFocus`. Đi qua `AndroidUiSession::ui_locale()` / `adb::parse_locale`.
  > - Verify trên máy thật: catalog tự chọn `vi`, rồi `Đề xuất`/`Thích`/`bình
  >   luận`/`Chia sẻ video`/`Yêu thích` đều PRESENT trong 135–185 ms.
  Vì thế `supports_accessibility_readback` ở đây là `true` — backend đầu tiên
  của dự án nói được câu đó, do iOS buộc giữ `snapshotMaxDepth = 1` (§2.3).
- **Không port `screen.rs` sang Android.** Nó là cách lách một API hỏng; Android
  không có lỗi đó.
- **Không cần IME riêng.** `ACTION_SET_TEXT` gõ tiếng Việt đủ dấu vào ô bình
  luận TikTok (xác minh bằng ảnh chụp), sau đó TikTok tự chuyển nút gửi sang đỏ.
  `adb shell input text` thì **giết tiến trình** khi gặp dấu.
- **`wm size` trả HAI dòng**; phải đọc **Override** (`1080x2220`), không phải
  Physical (`1440x2960`). Đọc nhầm là lệch 33% mọi toạ độ.
- **Bẫy hai `EditText`**: khay bình luận mở có một ô ẩn `focused=false` và một ô
  thật `focused=true`. Selector theo `class name` lấy nhầm cái ẩn, set text
  "thành công" ở tầng API mà màn hình trống. Dùng `Locator::focused()`.
- **Bài LIVE trong feed không có rail** like/comment. Vòng nurture phải nhận ra
  và vuốt qua; nhận ra được từ hierarchy.
- `find_and_tap` lấy bounds rồi **chạm thật** trong vùng đó, **không** dùng
  `ACTION_CLICK` của accessibility — click accessibility phân biệt được với
  người và đi vòng qua lớp cử chỉ mà chống phát hiện đang dựa vào.
- `AdbProgram::run` giải mã stdout thành text nên **phá dữ liệu nhị phân**; mọi
  thứ nhị phân (`exec-out screencap -p`) phải qua `run_bytes`/`device_bytes`, và
  cả hai đường screenshot đều kiểm magic PNG.
- Dấu phân cách trong lệnh shell gộp **chỉ được dùng chữ và gạch dưới**: một
  `--8<--` từng bị shell trên máy hiểu `<` là chuyển hướng input.
- `pidof` thoát khác 0 khi tiến trình vắng mặt. Đó là **câu trả lời**, không
  phải lỗi — đi qua `AndroidDriver::pid_of`.
- **Clipboard Android bị platform gác — ĐỪNG implement qua uiautomator2-server.**
  Đo trên Android 15: `POST /session/{id}/appium/device/set_clipboard` **có tồn
  tại** và trả 200 `value:null`, nhưng `get_clipboard` trả **rỗng**. Từ Android 10
  chỉ app đang giữ focus/quyền mới đọc được clipboard, và MIUI log rõ
  `ClipboardServiceI: checkProviderWakePathForClipboard: <pkg> is not a
  activePermissionOwner`. Server appium không có UI nên không bao giờ là chủ quyền
  đó. **Implement `get_clipboard` trên đường này sẽ tạo ra capability báo thành
  công mà trả về không gì** — đúng loại "HTTP 200 không phải bằng chứng" mà §3.9
  cấm. Vì vậy `AndroidUiSession` **cố ý** không implement clipboard, để rơi về
  `unsupported("getClipboard")` của trait.

  Ba đường khả dĩ, chưa chọn: (1) IME — GenFarmer dùng `AdbKeyboard` của openatx
  với broadcast `ADB_KEYBOARD_GET_CLIPBOARD`, vì **IME được phép đọc clipboard**;
  đổi default IME là xâm lấn và để lại dấu. (2) `io.appium.settings` — app helper
  mà Appium Node driver thực sự dùng cho clipboard, cần cài thêm một APK. (3) Với
  §3.12 Copy Link, đọc URL từ UI thay vì clipboard. Đây là **chặn thật cho
  Interaction trên Android**, không phải việc còn thiếu vài dòng code.
- **Pha 5 đã có câu trả lời đo được: minicap bản Java, KHÔNG phải scrcpy.**
  `noarch/minicap.apk` của `@devicefarmer/minicap-prebuilt` chạy qua
  `CLASSPATH=<apk> app_process / io.devicefarmer.minicap.Main`, **không cần cài** nên
  không vướng gate MIUI, và phát **JPEG** đúng contract `Frame` hiện có (không cần
  decoder H.264). Đã implement ở `crates/android-driver/src/frames.rs`; G1 probe đo
  qua chính code đó trên Redmi Note 12/Android 15 khi TikTok phát video,
  `-P 1080x2400@540x1200/0 -Q 70`: **155 frame trong 6,00 s = 25,8 FPS, 43,2
  KB/frame**, `forward tcp:0` đọc lại được port adb cấp (50784), banner
  `real=1080x2400 virtual=540x1200 quirks=2`. Một reader viết bằng PowerShell trên
  cùng socket chỉ đạt 11 FPS — **chênh lệch là do reader, không phải máy**; đừng lấy
  harness chậm làm giới hạn thiết bị.

  **Đã nối vào fleet, không còn là producer rời.** `riviu_core::FrameSink` là seam
  phía publish: `StreamHub` implement nó, `AndroidDriver::set_frame_sink` nhận nó ở
  composition root (`state.rs`), nên frame Android vào **cùng một hub** với iOS và
  giữ nguyên generation/sequence — không dựng hub thứ hai. `ensure_stream` trả
  `auto-stream://{udid}` như iOS. Kết quả trên máy thật: tile Android `● Live`,
  `Tổng quan 2/2`, badge "2 sẵn sàng".
  - Producer publish qua `publish_if_current`; **`false` là tín hiệu dừng, không
    phải lỗi** — stream mới đã sở hữu máy đó, byte còn trong buffer của reader cũ
    không được phép thành bằng chứng cho stream mới.
  - `ensure_stream` gọi lại thì **reuse** feed còn sống cùng generation, không
    restart.
  - **`adb forward tcp:0` cấp port MỚI mỗi lần gọi.** Retry cả forward lẫn connect
    trong một vòng lặp làm leak một port mỗi lần thử — đo được 4 forward mắc cạn
    sau một lần chạy. Forward đúng **một lần**, chỉ retry connect. Và vì teardown
    không bao giờ chắc chắn chạy, `frames::forward` **prune mọi forward cũ tới đúng
    socket của máy đó** trước khi tạo mới; socket mang tên serial nên cái gì còn
    bám vào đó đều là rác của mình. Đã verify: vào với 1 forward cũ, ra với đúng 1.
  minicap **native** thì đã chết trên Android nay (`.so` prebuilt chỉ tới android-30;
  trên SDK 35 lỗi `cannot locate symbol _ZN7android2ui4Size7INVALIDE`).
  minicap chỉ phát khi display đổi — đó là **ưu điểm**, khớp §3.4. `screencap -p` là
  512 ms/frame và `screencap` raw là 990 ms + 10 MB/frame: cả hai **không** làm được
  stream. Chi tiết và cách đo: `docs/re/genfarmer/README.md` §7.
- **`dumpsys window windows` không còn mang `mCurrentFocus` trên Android 15.** Đo
  được rỗng trên Redmi Note 12/HyperOS, trong khi đó lại là lệnh chạy được trên
  fleet S8+ Android 9 — nên `active_app_bundle` hỏi **cả hai** nguồn
  (`dumpsys window windows`, rồi `dumpsys window displays`) và coi việc grep thoát
  khác 0 vì không khớp là **câu trả lời**, không phải lỗi. Chính exit code đó làm
  G1 probe fail với thông báo rỗng. Đừng thay hẳn sang một lệnh.
- **`probe.rs` nhận `RIVIU_TIKTOK_PACKAGE`.** Package TikTok theo vùng: global là
  `com.zhiliaoapp.musically`, SEA là `com.ss.android.ugc.trill`. Máy mang bản kia
  làm probe fail ngay ở `launch_app` (`monkey -p … failed`) nên **không** đo được
  gì phía sau. Mặc định giữ nguyên giá trị cũ.
- **G1 probe đo lại trên Android 15 (10/08/2026, Redmi Note 12):** `list_devices`
  500 ms, `launch_app` 596 ms, `open_session` 2.160 ms, `window_size` 0 ms
  (1080x2400 Override), `active_app_bundle` 297 ms, `screenshot_png` 844 ms
  (571.271 byte), `inspect_app_process` 98 ms. Agent là
  `appium-uiautomator2-server` **10.4.0**, `/status` trả `ready:true`.
- **MIUI/HyperOS chặn cài app qua adb, và không có đường lách từ host.** Trên
  Redmi Note 12 (`ro.miui.ui.version.name=V816`, `OS2.0.207.0.VMTMIXM`,
  SDK 35) cả ba đường đều trả `INSTALL_FAILED_USER_RESTRICTED: Install canceled
  by user`: `adb install`, push rồi `pm install` trong shell máy, và session
  `pm install-create`/`install-write`/`install-commit` (create + stream **thành
  công**, chỉ **commit** bị chặn). Đừng thử lại ba đường này — nó là policy của
  PackageManager áp cho shell UID, không phải lỗi APK. Mở khoá bằng
  **Tuỳ chọn nhà phát triển → "Cài đặt qua USB"** (và trên MIUI thường cần thêm
  **"Gỡ lỗi USB (Cài đặt bảo mật)"**, đòi đăng nhập Mi account). Vì `ensure_agent`
  **không tự cài** agent, máy nào chưa bật cờ này thì Android control dừng ở đúng
  câu lỗi "the agent is not installed on {serial}".
- **Retry adb là opt-in, không phải mặc định.** `run_bytes` vẫn một phát;
  `run_bytes_idempotent` mới được retry. Lý do: `pm install`, `am start`,
  `am force-stop` và `input` không idempotent, nên retry sau một lần đã landing
  thật sẽ cài hai lần / mở hai lần mà **cả hai lần đều "thành công"** — lỗi vô
  hình. Đừng gộp retry vào `run_bytes` cho gọn.
- **`classify_fault` coi lỗi lạ là terminal, không phải transient.** Máy chưa bấm
  Allow (`Unauthorized`) fail y như vậy mãi mãi; retry nó chỉ làm chậm đúng thông
  báo operator cần. Chỉ `Transient` và `Timeout` được retry.
- **"adb còn sống" = danh sách device ổn định qua hai lần đọc**, không phải một
  lệnh trả về (`devices_stable`). Quan trọng vì `DeviceRegistry::upsert_many`
  thay cả vector: một snapshot xấu không chỉ sai, nó **xoá máy khỏi fleet**.
  `list_devices` hiện chỉ log khi không ổn định — registry giữ lại vector cũ khi
  lần đọc không đáng tin là việc **chưa làm**.
- **`adb kill-server` không bao giờ được gọi tự động.** Nó là hành động toàn cục:
  mọi tool khác trên máy mất kết nối adb và **mọi `adb forward` chết theo**, phải
  forward lại từng máy. Chỉ chạy khi người vận hành yêu cầu, và log lại.
  Nền tảng cho mấy điều trên: `docs/re/genfarmer/README.md`.

`MultiplexDriver` (`crates/core/src/driver_multiplex.rs`) gộp hai backend vào
**một** `DeviceControlPlane`. Không tách hai plane: `DeviceRegistry::upsert_many`
thay cả vector và phát `DevicesUpdated` mang toàn fleet, nên hai plane poll độc
lập sẽ luân phiên xoá máy của nhau; `DeviceExclusiveContext` còn mang `plane_id`
mà mọi transition đều kiểm. **Bảng route chỉ dựng từ `list_devices`** — không
bao giờ đoán nền tảng từ chuỗi udid; việc repo không validate định dạng udid là
tài sản, giữ nguyên. Một backend hỏng **không** che backend còn lại.

`supports_text_comments`, `supports_verified_app_termination`,
`requires_fresh_text_session`, `supports_push_media` giờ **nhận udid**: trên
fleet trộn, câu trả lời fleet-wide là lời nói dối về máy của nền tảng kia.

Backend Android chỉ tham gia khi `adb` thực sự dùng được (`detect_driver` chạy
`adb version` lúc khởi động), và lý do vắng mặt nằm ở
`android_unavailable_reason` **riêng** với `driver_degraded_reason` — "máy này
không có adb" và "sidecar iOS chết" là hai sự việc khác nhau.

`StreamHub` **dùng chung** cho cả hai backend qua `riviu_core::FrameSink` (xem ở
trên); `ensure_stream` của Android trả `auto-stream://<serial>` vì frame được
producer minicap publish thẳng vào hub, không có URL riêng cho session.
