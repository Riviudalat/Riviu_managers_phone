# GenFarmer 2.6.1 — khảo sát kiến trúc điều khiển thiết bị

> **Mục đích.** Trả lời một câu hỏi cụ thể: một sản phẩm farm Android đang chạy
> được thì điều khiển máy bằng gì, và Riviu học được gì **hợp pháp** từ đó.
> Không phải để copy code.
>
> **Ngày khảo sát:** 10/08/2026. **Máy khảo sát:** Windows 11 26200.
> **Cập nhật lần cuối:** 10/08/2026.

---

## 1. Phạm vi, phương pháp, và ranh giới tự đặt

GenFarmer là app Electron thương mại đã cài trên máy dev. Artifact được đọc tại
chỗ; không tải thêm gì, không chạy GenFarmer để quan sát hành vi, không đụng tới
server của họ.

**Ba thứ cố ý KHÔNG phân tích**, và đừng ai mở lại:

- Cơ chế licensing/DRM: `public-key.pem`, khối env đã mã hoá mà stream server
  nhận, `src/api/controllers/backend/{auth,payment,validate}.controller.ts`.
  Hiểu chúng không giúp Riviu làm gì hợp pháp cả.
- `main.jsc`, `log.worker.jsc` — V8 bytecode. Không dịch ngược.
- `genauto-agent` (binary Go riêng của họ) — chỉ ghi nhận vai trò và giao thức
  ở mức đủ để so sánh, không reverse.

**Redaction:** serial máy Android test ghi là `<REDACTED>`. Không có token,
UDID, hay dữ liệu tài khoản nào trong tài liệu này.

**Không có dòng code nào của GenFarmer được copy vào repo.** Mọi mô tả dưới đây
là cơ chế diễn đạt lại bằng lời, đúng tinh thần §3.8 của `AGENTS.md`: hiểu hợp
đồng hành vi, rồi tự viết — không rebrand binary người khác rồi gọi là của mình.

### Cách lấy chứng cứ

Main process là bundle esbuild **còn giữ comment `// src/...`**, nên khôi phục
được nguyên bản đồ 106 module mà không cần dịch ngược. Asset thiết bị nằm trong
`app.asar`, trích từng file bằng `@electron/asar extract-file` (lưu ý: trên
Windows path trong asar dùng dấu `\`).

## 2. SHA-256 các artifact đã đọc

| Artifact | Bytes | SHA-256 |
|---|---|---|
| `resources/app.asar` | 248,323,460 | `ba534abe96c8efcfd27d4dac28751cdefdb9b854b0f31bed47b1e746fae00658` |
| `GenFarmer.exe` | 190,592,592 | `33eb04c6967709a0ea72ec5e585980dea575f372eac5d60664c9c14fd7177d7f` |
| `dist/main/index.js` | 2,220,613 | `c1c7904280edb5f983cbadb451dc287c77f3f7d0ccb080a65e7799a9a65b3be5` |
| `package.json` | 2,284 | `fbc4cb8424286112f55ab5713de06c474802d2d0a8c079d9f3afb4fdf2e6fa57` |
| `build/common/scrcpy-server.jar` | 69,994 | `0846802f863cdd8c159bf20a14031305e3aed7b98d654d50e40f7da31fa06e18` |
| `build/common/app-uiautomator.apk` | 1,879,807 | `6fc4f018970adb6a282f1f49f4ca3f528ee2822e66ca0b4f37d8b35ec1f5d662` |
| `build/common/app-uiautomator-test.apk` | 1,121,597 | `50ee58c4cc8f07ba70f1858c4012bd10ed3744c228d49f902df2a0669e95c0c4` |
| `build/common/atx-agent-arm64` | 10,092,696 | `0195402052f351daaf910f276b5c7d9c9163f9046d90723fa4f7631d6b7dd014` |
| `build/common/genauto-agent-arm64` | 15,536,695 | `5909206e1a4710d4b8ddd29c2d2996e35163153f14f6e9fbafc61444ac7d66ba` |
| `build/common/genfarmer_util` | 2,538,428 | `6d9ab16b17a06aeb73adee5304fb60f993b270e70f656b213a051dbce9540974` |
| `build/common/genfarmer_install.sh` | 631 | `3635c4b5a0af9774568417fbfb738abb1fe51b1e2afadc6ea042b36085ac3316` |
| `build/common/main.py` | 4,915 | `9251ff2bc6e2f57c3bb912fc1239acf5c2ae9e957240f46ebcf55d8d615cf239` |
| `build/common/run.py` | 301 | `5eca725767c186f12fd374a7b656c017da8b2aab566d8dc911557957fb17f2a8` |

## 3. Kiến trúc: năm kênh, chia theo việc chứ không theo tầng

| Kênh | Ở đâu | Dùng để |
|---|---|---|
| adb protocol (`@devicefarmer/adbkit`, JS thuần) | host | lifecycle: install, shell, forward, list |
| `genauto-agent` (Go) tại `/data/local/tmp/genauto-agent`, `tcp:8912` | thiết bị | agent thường trú, host nói HTTP qua `adb forward` |
| uiautomator server (`com.genfarmer.uiautomator{,.test}`) | thiết bị | JSON-RPC `/jsonrpc/0`: element, hierarchy, gesture |
| `scrcpy-server` 2.4 → `adb-tools.exe` → WebSocket | cả hai | video H.264 để xem màn hình trong UI |
| Airtest (đổi tên `gentest`) sau uvicorn `127.0.0.1:58211` | host | template matching OpenCV: tìm ảnh rồi chạm |

Điểm quan trọng nhất của thiết kế này: **xem màn hình và tự động hoá đi hai
đường khác nhau**. scrcpy chỉ để người xem; máy thì nhìn qua minicap/screenshot
rồi so khớp ảnh. Riviu hiện gộp một đường (MJPEG vừa vẽ tile vừa làm bằng chứng)
— xem §7.

`~/.genfarmer/` là data dir: `db.sqlite`, `static/` (các APK + script cài),
`data/scrcpy-server.jar`, `app/adb-tools.exe`, `automation/`, `screenshot/`,
`logs/xml/`, `apks/`, `stream-port`.

## 4. Từng kênh, cơ chế đo được

### 4.1 adb: hàng đợi, retry, và một thang khôi phục riêng

`adb.service.ts` là module lớn nhất của họ (~5.500 dòng bundle). Mọi lệnh shell
đi qua một hàng đợi (`adb-queue.ts`) và **retry tối đa 5 lần**, chỉ retry khi lỗi
được phân loại là *transient* (adb/socket) — không retry mù.

`adb-recovery.service.ts` là một **thang khôi phục có thứ tự**, không phải một cú
kill:

1. `oneListOk()` / `waitForStableList(deadline)` — coi "adb còn sống" là *danh
   sách device ổn định*, không phải một lệnh trả về.
2. `tryStabilizeWithoutKill()` — thử cứu mà không giết server.
3. `killAndStartServer()` — chỉ khi bước 2 thất bại.
4. `chunkedReconnectNetworkDevices()` — reconnect device mạng theo lô, không
   đồng loạt.
5. `isRecovering()` / `waitUntilNotRecovering(maxWaitMs)` — mọi caller khác chờ,
   thay vì cùng nhau kích hoạt khôi phục.

Đây đúng bài học §2.7 của Riviu (đừng recycle transport vì một probe
false-negative), nhưng ở dạng có cấp bậc rõ ràng.

### 4.2 Bootstrap agent thường trú

Trình tự đo được:

1. Push binary theo đúng kiến trúc vào path tạm.
2. `sh -c "chmod 755 <tmp> && mv -f <tmp> <bin> && chmod 755 <bin>"` (deadline 30 s).
3. Kiểm lại `chmod 755 <bin> && sync` (20 s) → **fail closed** với đúng câu
   "binary is not executable (chmod did not apply)". Họ không tin chmod đã chạy.
4. `sh -c "<bin> server -d"` — daemon hoá.
5. `ensureForwarded()`.

APK khi cài được push vào `/data/local/tmp/<tên>.<timestamp>.apk` — timestamp để
hai lần cài song song không đè nhau.

**Forward:** `forward tcp:0 tcp:8912` rồi `listForwards()` **đọc lại port thật
mà adb cấp**; nếu không thấy hoặc local không hợp lệ thì throw. Có map
`agentForwardInFlight` dedupe theo `(serial, remote)` nên nhiều caller đồng thời
không tạo trùng forward. Khi adb server/client bị tạo lại thì **xoá cả cache
forward và cache HTTP client** — comment của họ nói thẳng: mọi `forward tcp:0`
trên host mất hiệu lực sau `adb kill-server`.

### 4.3 uiautomator session

`uiautomator-session.service.ts` thực chất là **port `python-uiautomator2` sang
TypeScript**: `callRpc(method, params, timeout, attempt)` tới `/jsonrpc/0`,
`dumpWindowHierarchy(compressed, max_depth, retry)`, `xpath(selector, xml)`,
`swipe`/`swipeExt(direction, scale, retry)`, `touchDown/Move/Up`, `click`,
`longClick`, `doubleClick`, `press`, `lockScreen`/`unlockScreen`,
`install`/`uninstall`/`isInstalled`, `pushFile`/`pullFile`,
`screenShot`/`screenShotToBuffer`, `windowSize`, `rotation`.

Ba chi tiết đáng ghi:

- **`getWindowSizeCached(maxAgeMs = 3000)`** — kích thước màn hình được cache 3 s
  thay vì hỏi lại mỗi lệnh.
- `parseWmSize` và `parseRotationFromDisplayDump` là hàm riêng có thể test —
  cùng bài học với `parse_wm_size` của Riviu.
- Phân loại lỗi tách bạch: `isNetworkError`, `isConnRefused`,
  `shouldRestartOnError`, `isTransientAdbError`, và `runWithAdbRetry(label, fn,
  maxAttempts = 3)`. "Kết nối bị từ chối" và "server chưa boot" là hai việc khác
  nhau.

Comment của họ ghi lại một bài học rất giống §2.2 của Riviu (stock WDA phải prime
session): **server JSON-RPC boot rất chậm, trong lúc chờ thì mọi `/jsonrpc/0`
trả 502** — nên "process còn sống" không đồng nghĩa "gọi được".

### 4.4 Gõ chữ Unicode và clipboard: qua IME

GenFarmer **thay bàn phím mặc định của máy**:

```
ime enable  com.genfarmer.uiautomator/.AdbKeyboard
ime set     com.genfarmer.uiautomator/.AdbKeyboard
settings put secure default_input_method com.genfarmer.uiautomator/.AdbKeyboard
```

rồi gửi chữ bằng broadcast với payload base64:

| Broadcast | Việc |
|---|---|
| `ADB_KEYBOARD_INPUT_TEXT` | chèn thêm chữ |
| `ADB_KEYBOARD_SET_TEXT` | xoá rồi đặt chữ |
| `ADB_KEYBOARD_GET_CLIPBOARD` | **đọc clipboard** |

Đây là AdbKeyboard đi kèm bundle uiautomator của openatx.

**Riviu KHÔNG nên đổi sang đường này để gõ chữ.** §9 đã đo được `ACTION_SET_TEXT`
gõ tiếng Việt đủ dấu, và đổi IME mặc định là thay đổi trạng thái máy — vừa xâm
lấn vừa là một dấu hiệu phân biệt được. Nhưng `ADB_KEYBOARD_GET_CLIPBOARD` là câu
trả lời cho một câu hỏi Riviu **sẽ** phải trả lời: §3.12 bắt buộc Copy Link phải
đọc lại clipboard, và trên Android chưa có đường nào. Ghi nhận là *một* phương
án; phương án còn lại là RPC clipboard của chính uiautomator server.

### 4.5 Stream: scrcpy 2.4 sau một stream server riêng

`adb-tools.exe` là process riêng, spawn với env:

```
SCRCPY_SERVER_SOURCE      = <appdata>/data/scrcpy-server.jar
SCRCPY_SERVER_DESTINATION = /data/local/tmp/genscrcpy.jar
SERVER_ADDRESS            = 0.0.0.0:<port>
LISTENER                  = 4
```

Nó push scrcpy-server lên máy (**đổi tên thành `genscrcpy.jar`** để không đụng
scrcpy của tool khác), chạy, rồi re-serve H.264 qua WebSocket; renderer decode
bằng `h264-converter` vào canvas. Đúng kiến trúc ws-scrcpy.

Version xác định từ dex: có `AudioCapture`, `CameraCapture`, `AsyncProcessor`,
`CameraFacing` (camera có từ scrcpy 2.2) và string `2.4` → **scrcpy-server 2.4**.
Bản thân `.jar` là APK đổi tên (`AndroidManifest.xml` + `classes.dex`).

Chi tiết vận hành đáng giá: họ có sẵn hộp thoại lỗi hướng dẫn người dùng
**whitelist `.genfarmer` trong antivirus**, vì AV hay chặn/xoá `adb-tools.exe`.

### 4.6 Sidecar Airtest cho template matching

`run.py` dựng `uvicorn main:app --host 127.0.0.1 --port 58211 --workers 4`.
`main.py` import `gentest.*` — API trùng khít Airtest (`Android`, `Template`,
`Settings`, `TargetPos`, `TargetNotFoundError`), tức **Airtest đổi tên**. Thiết
bị mở với `cap_method=MINICAP`, `touch_method=MINITOUCH`, `ori_method=MINICAP`.

Protocol: WebSocket `/ws`, message đầu là serial → `"OK"`, sau đó mỗi job

```
{ id, task: { file_path, click, timeout, index_pos, click_delay, threshold, method } }
→ { id, result: { success, data: [positions] } }
```

`loop_find` poll `device.snapshot()` rồi `Template.match_in(screen)` cho tới
timeout. Nếu `snapshot()` trả `None` thì họ log *"may be locked"* — màn hình tắt
là một trạng thái, không phải lỗi.

## 5. Provisioning và mô hình dữ liệu

`genfarmer_install.sh` chạy **trên máy** (`#!/system/bin/sh`), cài split-APK qua
session:

```
pm install-create -S <tổng bytes> -i <packageName> -r
pm install-write  -S <size> <session> <tên split> < <file>
pm install-commit <session>
```

Cờ `-i <packageName>` đặt **installer attribution** — app cài xong trông như do
installer đó cài.

Trước khi cài, họ tắt kiểm duyệt cài đặt:

```
settings put global verifier_verify_adb_installs 0
settings put global package_verifier_enable 0
settings put global package_verifier_user_consent -1
```

Đọc metadata APK **ngay trên máy** thay vì pull về host:
`CLASSPATH=/data/local/tmp/genfarmer_util app_process / net.dongliu.apk.parser.Main <apkPath>`
→ JSON có label + icon base64. `genfarmer_util` là dex chứa apk-parser.

Khác: `svc wifi <state>` để bật/tắt Wi-Fi; `wallpaper.apk` (bên thứ ba,
`org.mistkeith.setwallpaper`) để đặt hình nền cho từng máy — nhận diện máy trong
farm bằng mắt.

**Schema SQLite (Drizzle, 14 bảng):** `devices`, `device_groups`, `apks`, `apps`,
`accounts`, `table_account`, `table_tree`, `router_proxy`, `tasks`, `task_runs`,
`task_schedules`, `task_run_device_status`, `task_run_device_storages`,
`schedules`.

Ba bảng đáng chú ý vì Riviu có đúng nhu cầu tương ứng: `device_groups` (Riviu:
`565d71e feat(groups)`), `router_proxy` (Riviu §3.11), và
`task_run_device_status` + `task_run_device_storages` — trạng thái và storage
**theo từng máy trong một run**, giống assignment của Riviu.

Ngoài ra `package.json` cho thấy các mảng tính năng: `imapflow` + `mailparser` +
`node-2fa` + `@faker-js/faker` (nuôi/ tạo tài khoản), `openai` +
`@google/genai` (nội dung), `simple-proxy-agent` + `hpagent` (proxy),
`croner` (lịch), `xlsx` + `csv-parser` (nhập liệu khối lớn),
`codemirror` + `@vue-flow/*` + `dagre` (editor flow trực quan),
`socket.io` + `express` (có cả REST API và `cloudPhone.controller.ts`).

## 6. Đối chiếu với Riviu — chỗ nào Riviu đã hơn, chỗ nào nên học

**Riviu đang làm tốt hơn hoặc khác có chủ đích, đừng đổi:**

| Việc | GenFarmer | Riviu | Nhận xét |
|---|---|---|---|
| Agent trên máy | openatx JSON-RPC | `appium-uiautomator2-server` W3C | Riviu chọn cái được maintain rộng hơn, có W3C. Không thua. |
| Host port cho forward | `tcp:0` rồi đọc lại | port cố định theo serial + `HashSet` serial *do mình* forward | Riviu **chặt hơn**: không "mượn" forward của tool khác (đúng lo ngại xung đột ở §8). |
| Gõ chữ | đổi IME mặc định | `ACTION_SET_TEXT` | Riviu ít xâm lấn hơn, không để lại dấu. |
| Locator | XPath + content-desc | content-desc là chiến lược chính, `resource-id` bị loại vì R8 | Cùng kết luận; Riviu ghi rõ lý do hơn. |

**Nên học (là *kiến thức*, không phải code):**

1. **Thang khôi phục adb có cấp bậc** (§4.1) — hiện `adb.rs` của Riviu chưa có
   khái niệm "danh sách device ổn định" như tín hiệu sống, và chưa có trạng thái
   `isRecovering` để các caller khác chờ thay vì cùng kích hoạt.
2. **Chmod rồi kiểm lại, fail closed** khi push binary (§4.2).
3. **Cache kích thước màn hình có tuổi** (§4.3) thay vì hỏi lại mỗi lệnh.
4. **`pm install-create/write/commit`** cho split-APK — `adb install` thường thất
   bại với app phân tách; Riviu sẽ gặp khi cài TikTok từ APK/AAB.
5. **Đọc metadata APK trên máy** bằng apk-parser + `app_process` (§5) — rẻ hơn
   pull APK về host.
6. **Tách đường xem và đường bằng chứng** (§3) — xem §7.

## 7. Frame source cho Android — và vì sao nó KHÔNG phải việc gấp

Đọc GenFarmer dễ dẫn tới kết luận sai rằng Riviu đang thiếu stream nên phải làm
ngay. `session.rs:243` nói ngược lại, và nói có lý:

> *"No MJPEG producer yet. Frames are deliberately deferred: with hierarchy-based
> location they are corroboration, not the locator."*

Trên Android, **hierarchy chính là locator**. iOS phải dò pixel vì bị ép
`snapshotMaxDepth = 1` (§2.3), Android không có ràng buộc đó — nên frame chỉ là
bằng chứng bổ trợ. Đúng phân tầng của GenFarmer ở §3: họ cũng **không** tự động
hoá bằng scrcpy; scrcpy chỉ để người xem.

Vậy nên đây là việc *khi nào cần thì làm*, không phải lỗ hổng đang chảy máu. Khi
làm, khảo sát này chốt được lựa chọn công nghệ, và nó **không phải scrcpy**:

- `FrameSource` của Riviu định nghĩa `Frame` là **JPEG** (`frame_source.rs:18`),
  và `image` trong desktop chỉ bật feature `jpeg`.
- **scrcpy trả H.264.** Dùng nó cho frame source có nghĩa là thêm decoder video
  vào Rust — dependency lớn, và vẫn phải encode lại sang JPEG cho detector.
- **minicap trả JPEG trực tiếp** — khớp đúng contract hiện có. Đây chính là lý do
  GenFarmer dùng minicap cho tự động hoá và chỉ dùng scrcpy để *xem*.
- `adb exec-out screencap -p` trả **PNG**, mà đường decode hiện tại không đọc PNG.
  Nó là bậc thấp nhất (chậm, một ảnh một lần), không phải stream.

Vì vậy thứ tự đề xuất: **minicap (JPEG, stream) cho `FrameSource`**, scrcpy chỉ
xét sau nếu cần xem màn hình mượt trong UI. Cái giá của minicap là phải cung cấp
binary theo ABI × SDK cho từng máy — đúng lớp bài toán artifact mà Riviu đã có
kỷ luật xử lý với IPA (manifest + SHA-256 + gate), nên dùng lại được khuôn đó.

Hai thứ phải trả lời trước khi viết dòng code đầu tiên:

- `GenerationFrameSource` yêu cầu generation + sequence để bằng chứng không vượt
  qua một lần restart stream. Thiết kế minicap phải mang được ngữ nghĩa đó, không
  chỉ đẩy byte.
### 7.1 Số đo trên Redmi Note 12 / Android 15 (SDK 35), 10/08/2026

Đo trực tiếp, không đoán. Lưu ý: **gate cài app của MIUI không chặn đường này**,
vì minicap/scrcpy chỉ cần `adb push` + thực thi, không cần `pm install`.

| Đường | Kết quả đo |
|---|---|
| `screencap -p` (PNG) | min **512 ms**, median 529 ms, 15.580 byte → ~2 FPS |
| `screencap` (raw) | min **990 ms**, 10.368.016 byte = 1080×2400×4 + header 16 byte, `format=1` (RGBA_8888) |
| minicap **native** (.so android-30 trên SDK 35) | ❌ `CANNOT LINK EXECUTABLE … cannot locate symbol "_ZN7android2ui4Size7INVALIDE"` |
| minicap **Java** (`noarch/minicap.apk` qua `app_process`) | ✅ chạy, phát **JPEG hợp lệ** (9/9 và 7/7 magic `FF D8 FF`) |

**Raw chậm hơn PNG** vì 10 MB/frame qua USB áp đảo chi phí encode trên máy — nên
đừng lặp lại giả thuyết "raw nhanh hơn vì không phải encode".

**minicap native đã chết trên Android nay**: prebuilt của `@devicefarmer/minicap-prebuilt@2.7.3`
chỉ có `.so` tới **android-30**, và ABI nội bộ của platform đã đổi (thiếu
`android::ui::Size::INVALID`). Muốn dùng native thì phải build `.so` theo cây AOSP 15.

**Đường sống là minicap Java**, chạy `CLASSPATH=<apk> app_process / io.devicefarmer.minicap.Main`:

- Không cài gì → **miễn nhiễm gate MIUI** (§9).
- Cờ khớp đúng nhu cầu Riviu: `-Q` JPEG quality, `-P <w>x<h>@<w>x<h>/<rot>` (đã xác
  nhận `virtual=540x1200` khi projection nửa scale), `-S` bỏ frame khi consumer
  chậm (đúng ngữ nghĩa "coalesce rather than queue" của `FrameStream`), `-r` frame
  rate, `-n <socket>` phát qua abstract unix socket.
- Banner 24 byte đọc được: `version=1 real=1080x2400 virtual=<projected> orient=0 quirks=2`,
  sau đó mỗi frame là `u32 LE length` + JPEG. Host nối bằng
  `adb forward tcp:0 localabstract:<socket>`.
- **Nó chỉ phát khi display có frame mới.** `-r 10` **không** ép phát định kỳ.
  Đây là *ưu điểm* chứ không phải thiếu sót: khớp §3.4 ("chỉ decode khi digest byte
  frame đổi → feed đứng yên = 0 CPU").

**FPS thật, đo khi TikTok đang phát video** (máy mở khoá, `-P 1080x2400@540x1200/0
-Q 70`): **66 frame trong 6,02 s = 11,0 FPS, 66/66 JPEG hợp lệ, 55,9 KB/frame**
(≈615 KB/s). Watcher chỉ cần ≤3 FPS (§3.4) nên **minicap Java thừa sức**, và đây là
số chốt cho Pha 5.

Con số ~1 FPS đo trước đó là giới hạn của **nguồn chuyển động**, không phải của
minicap: máy đang khoá và `input swipe` tốn ~1,5 s/lần nên phần lớn cửa sổ đo không
có gì đổi. Bài học đo: **muốn số FPS thật thì phải có nội dung thật đang đổi.**

**Full-scale (1080x2400) vẫn chưa có số tin được.** Lần đo ra 0,4 FPS bị nhiễu vì
tap của người đo lại tạm dừng video (TikTok toggle play/pause khi tap giữa màn).
Đừng lấy 0,4 FPS làm năng lực full-scale. Half-scale là cấu hình thực dụng hơn và
đã có số; `session.rs` cũng đã tính tới ("A half-scale screenshot maps back to full
device pixels").

**Ngoài lề nhưng đáng biết:** trên máy test đã có sẵn
`/data/local/tmp/riviufarm/{scrcpy-server-v2.4, scrcpy-server-v2.4.sha256, scrcpy-server.jar}`
(69.007 byte, 19–20/07/2026) — tên thư mục và file `.sha256` kèm theo đúng kỷ luật
attestation của Riviu, nhưng **repo không có artifact hay code scrcpy nào**. Tức đã
từng có một lần staging scrcpy cho Riviu mà không để lại dấu trong repo. Bản đó
**khác** bản của GenFarmer (69.994 byte; MD5 `genscrcpy.jar` trên máy khớp chính xác
bản trong `app.asar`). Ai làm tiếp Pha 5 nên tìm hiểu lần staging đó trước.

Cũng đo được: `input keyevent` và `input swipe` đều exit 0 → **MIUI không chặn
input injection**, chỉ chặn cài app.

## 8. Đã triển khai từ khảo sát này

**Pha 5 — nguồn frame Android, đã implement và đo qua chính code Riviu.**
`crates/android-driver/src/frames.rs`: đẩy `noarch/minicap.apk` (bỏ qua nếu số byte
trên máy đã khớp), chạy qua `app_process` **không cần cài**, `adb forward tcp:0` rồi
**đọc lại port adb cấp** (đúng bài học §4.2 — port cố định sẽ đụng nhau trên fleet
20 máy), parse banner 24 byte, rồi đọc từng frame `u32 LE length` + JPEG với kiểm
magic từng frame. G1 probe đo được **155 frame / 6,00 s = 25,8 FPS, 43,2 KB/frame**
ở `1080x2400@540x1200` Q70, port adb cấp 50784.

Module này **cố ý không** sở hữu generation/stream-budget/ownership: `StreamHub` đã
làm rồi, dựng thêm một bộ nữa là tạo nguồn sự thật thứ hai cho thứ tự bằng chứng.

**Đã nối xong vào fleet.** Thêm `riviu_core::FrameSink` làm seam phía publish (đối
xứng với `FrameSource` phía đọc), `StreamHub` implement nó, và composition root
tiêm nó vào `AndroidDriver`. Nhờ vậy frame Android vào **cùng hub** với iOS, giữ
nguyên generation/sequence, và `ensure_stream` trả `auto-stream://{udid}` như iOS.
Trên máy thật: tile Android `● Live`, `Tổng quan 2/2`.

Bài học trả bằng bug thật trong lúc làm: **`adb forward tcp:0` cấp port mới mỗi lần
gọi**, nên retry cả forward lẫn connect làm mắc cạn 4 forward sau một lần chạy. Sửa
hai lớp: forward đúng một lần, và `forward()` **prune** mọi forward cũ tới socket của
máy đó trước khi tạo mới — vì teardown không bao giờ chắc chắn chạy.

Thứ chuyển giao được rõ nhất **ngoài stream** là §4.1: `adb.rs` của Riviu
trước đó không có một dòng nào về phân loại lỗi, retry hay health. Đã thêm:

- `AdbFault` + `classify_fault()` — tách `Transient` / `Timeout` /
  `Unauthorized` / `UnknownDevice` / `Terminal`. Lý do tách: một máy chưa bấm
  Allow sẽ fail **y như vậy mãi mãi**, retry nó chỉ làm chậm thông báo mà operator
  cần. Cái gì không nhận dạng được thì coi là terminal, không đoán là transient.
- `run_bytes_idempotent(args, timeout, attempts)` — retry có backoff
  250/500/1000 ms (clamp ở attempt 4). **Cố ý không** gộp vào `run_bytes`:
  `pm install`, `am start`, `am force-stop`, `input` không idempotent, và retry mù
  sau một lần đã thành công thật sẽ cài hai lần / mở hai lần mà không ai thấy.
  Retry là opt-in theo từng call site. GenFarmer retry `shell()` 5 lần một cách
  tổng quát — chỗ này Riviu cố tình bảo thủ hơn.
- `devices_stable(settle, deadline)` + `DeviceListReading { devices, stable,
  attempts }` + `same_fleet()` — "adb còn sống" = **danh sách device ổn định qua
  hai lần đọc liên tiếp**, không phải một lệnh trả về. Đã nối vào
  `AndroidDriver::list_devices`.
- `kill_server()` — có nhưng **không bao giờ tự gọi**, kèm doc nói rõ nó là hành
  động toàn cục: mọi tool khác trên máy mất kết nối adb (đúng §9) và mọi
  `adb forward` chết theo. GenFarmer coi đây là bậc 3 của thang khôi phục; với
  Riviu, nơi một máy có thể đang chạy tool khác, nó phải do người quyết định.

Vì sao `list_devices` chỉ *log* khi không ổn định chứ không raise: từ chối scan sẽ
làm trống nửa Android của fleet trộn — cùng một thiệt hại nhìn từ phía khác. Cách
sửa đúng là registry giữ lại vector cũ khi lần đọc không đáng tin, và **việc đó
chưa làm**. Đừng nhầm là đã xong.

6 unit test mới, không cần thiết bị: phân loại từng họ lỗi, `same_fleet` bỏ qua
thứ tự adb in ra, và trường hợp bẫy nhất — **tập serial giống nhau nhưng state
đổi** (`unauthorized` → `device`) phải bị coi là chưa ổn định.

## 9. Cảnh báo vận hành

- **Xung đột instrumentation.** Máy test hiện có `com.genfarmer.uiautomator{,.test}`
  và GenFarmer sẽ chạy agent riêng ở `tcp:8912`. Android chỉ cho một
  instrumentation UiAutomator giữ kết nối accessibility — **đúng lớp lỗi
  3uTools/XCTest ở §2.9**. Phải tắt GenFarmer trước khi chạy Riviu trên cùng máy.
- **Antivirus** hay xoá stream server dạng `.exe` (họ phải viết hộp thoại hướng
  dẫn whitelist). Nếu Riviu ship binary tương tự, tính trước việc này.
- **Đổi IME mặc định để lại dấu** trên máy; nếu vì clipboard mà phải làm, hãy
  hoàn nguyên sau khi xong.

## 10. Đã đọc bổ sung (17/08/2026) — kết quả ở hồ sơ riêng

Toàn bộ danh sách "chưa đọc" của bản trước đã được khảo sát xong trong đợt 2.
Vì khối lượng lớn, kết quả nằm ở hồ sơ riêng ngoài repo (không nhét source của
GenFarmer vào repo Riviu), cùng với **mã nguồn đã de-bundle** phục vụ tham khảo:

- Mã nguồn: `C:\Users\cattfan\Documents\All\genfarmer-src\debundled\` —
  tách `dist/main/index.js` bằng parser **acorn** (chính xác theo AST, đã kiểm
  chứng lossless): **106 file .ts** theo đúng cây `src/`, tổng **22.576 dòng
  source thật**; kèm `INDEX.md` (file → dòng bundle gốc), 105 file vendor gap
  (`_vendor/`), `_preamble.js`, `_tail.js`, `worker.log.worker.js`,
  `preload.index.js`. Script tái tạo: `genfarmer-src/_debundle2.js`.
- Báo cáo: `C:\Users\cattfan\Documents\All\genfarmer-explore\`
  `01-automation-runtime.md`, `02-api-ipc.md`, `03-data-services.md`,
  `04-renderer.md`, `05-main-performance.md`, `06-renderer-performance.md`.

Đính chính số dòng từ lần đọc thật: `adb.service.ts` chỉ ~238 dòng source thật
(khoảng marker-to-marker chứa vendor), `automationLogger.service.ts` ~95 dòng —
số dòng module thật nằm trong `debundled/INDEX.md`. Điểm quan trọng nhất chưa
có trong tài liệu này: **script runner không sandbox** — Action `Javascript`
dùng `eval` ngay trong main process (bài học ngược cho Riviu).

## 11. Thành phần upstream — cái Riviu dùng trực tiếp được

Gần như toàn bộ tầng Android của GenFarmer là open-source. Riviu **không cần** và
**không nên** lấy gì từ phần proprietary của họ:

| Thành phần | Upstream | License |
|---|---|---|
| uiautomator server + AdbKeyboard | `openatx/android-uiautomator-server` | MIT |
| agent thường trú | `openatx/atx-agent` | MIT |
| video | `Genymobile/scrcpy` (server 2.4) | Apache-2.0 |
| capture / touch | `openstf/minicap`, `openstf/minitouch` | Apache-2.0 |
| template matching | `AirtestProject/Airtest` | Apache-2.0 |
| adb client JS | `DeviceFarmer/adbkit` | Apache-2.0 |
| apk parser trên máy | `hsiafan/apk-parser` | Apache-2.0 |

Riviu đã dùng `appium-uiautomator2-server` (Apache-2.0) cho tầng agent, nên phần
còn thiếu thật sự chỉ là **nguồn frame** (§7).

Proprietary của GenFarmer — **không dùng, không reverse**: `dist/main/index.js`
và `*.jsc`, `genauto-agent`, `genfarmer_util` (bundle riêng), `adb-tools.exe`,
`public-key.pem` và toàn bộ luồng licensing/payment.

## 12. Hiệu suất — bản đồ cơ chế để tham khảo (17/08/2026)

Đợt 2 đã de-bundle toàn bộ main process (106 file, 22.576 dòng source thật) và
lập danh mục cơ chế hiệu suất. Chi tiết đầy đủ kèm bằng chứng file:dòng ở
`Documents/All/genfarmer-explore/05-main-performance.md` và
`06-renderer-performance.md`; dưới đây là phần đáng để Riviu soi.

### 12.1 Điều phối song song — nơi họ đặt các nút hạn chế

- Hàng đợi ADB **2 tầng**: mỗi serial một hàng đợi tuần tự (×1) rồi mới vào hàng
  đợi toàn cục **×16** — cùng ý tưởng "serialize theo máy + giới hạn toàn cục"
  mà Riviu đang thiếu cho adb. Với 200 máy đây chính là cổ chai chính của họ.
- Các TaskQueue theo ngữ cảnh: reconnect ×5, push APK ×5, tạo session ×8,
  warmup lô 20 máy (timeout 90s/lô), refresh device concurrency ×4.
- `deviceThreads` giới hạn worker mỗi run (free plan bị chặn xuống 2).
- Batch ghi DB: Queue gom **50 phần tử / 2000ms** cho device storage.

### 12.2 Throttle/cooldown — bài học quan trọng nhất cho fleet lớn

Họ gắn **cooldown có cửa sổ** vào mọi hành động phục hồi, không chỉ retry:

| Cơ chế | Tham số |
|---|---|
| Recreate/restart ADB client | cooldown 30s, dedupe qua promise đang chạy |
| Kill adb server (recovery) | tối thiểu 45s giữa 2 lần kill |
| Reconnect máy | 5 lần / cửa sổ 15 phút / cooldown 10 phút, backoff mũ |
| Restart tracker | 5 lần / 10 phút / cooldown 10 phút |
| Reload danh sách máy | tối thiểu 5s, dedupe in-flight |
| Emit log về UI | throttle 120ms/run |

Đây là cách họ chặn "storm tự nuôi" khi 200 máy fail đồng loạt — đúng tinh thần
phân loại lỗi của Riviu (§8) nhưng thêm lớp giới hạn tần suất ở phía hành động.

### 12.3 Timeout/Watchdog — không có chỗ nào chờ vô hạn

- adb shell retry ≤5 (chỉ lỗi transient) + backoff; getSession 3 lần × timeout 70s;
  heal lỗi hạ tầng tối đa 12 lần với backoff ≤4s; mỗi node script có nodeTimeout.
- Watchdog 60s cho AUTOMATION_STARTED (kẹt setup → fail, không treo run);
  `threadRuntime` cắt thời gian chạy mỗi máy; sau ADB recovery chờ máy online
  tối đa 120s rồi abort; RPC uiautomator hard-timeout timeout+400ms.

### 12.4 Bộ nhớ / log

- `--expose-gc` + `global.gc()` mỗi 10s (cả main lẫn renderer) — biện pháp thô
  nhưng cho thấy họ chấp nhận rò nhỏ thay vì OOM trên máy chạy lâu.
- Log run giữ trong bộ nhớ với cap (500 dòng/run, 300/máy, giữ 5 run) + file
  rotation 50MB qua worker riêng; app.log cắt giữ đuôi 200MB khi quá 2GB.

### 12.5 Điểm nóng đã xác định (để soi khi GenFarmer chậm — hoặc để tránh khi viết)

1. Hàng đợi ADB global ×16 là cổ chai với fleet lớn.
2. Warmup tuần tự lô 20 → có thể kéo hàng phút trước khi run bắt đầu.
3. getUiAutomatorClient 3×70s — một máy kẹt session treo tới ~3,5 phút.
4. Ghi SQLite **đồng bộ** trên main thread; cập nhật account/deviceStatus theo
   từng máy không batch.
5. Renderer: console-interceptor gửi stack qua IPC cho mỗi console.*; pinia
   persist ghi localStorage đồng bộ mỗi mutation; graph ScriptEditor không ảo
   hoá node; poll log run 2s.
6. Worker tìm ảnh Python cố định 4 luồng — nghẽn khi nhiều máy cùng IMAGE.

### 12.6 Áp cho Riviu

Chỗ đáng học trực tiếp: **mô hình cooldown có cửa sổ cho mọi hành động phục hồi**
(§12.2) và **timeout kín mọi đường** (§12.3). Chỗ đã thấy rõ cái giá: SQLite sync
trên main thread và console→IPC không tiết chế — Riviu (Rust + tách process) đang
ở thế tốt hơn, đừng bắt chước hai thứ này.
