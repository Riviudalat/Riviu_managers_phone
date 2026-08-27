## 5. Trạng thái bình luận

### 5.1 Bình luận chữ qua RT-MMO: ĐÃ LIVE XÁC NHẬN

Ngày 28/07/2026 đã nối xong đường text end-to-end trong code:

- `wda.rs`: profile RT-MMO, header auth cho mọi request, logical size 375×667,
  không probe endpoint thiếu; text job dùng `POST /session` mới sau khi TikTok đã
  foreground. Nếu build không cho POST mới fallback session tự tạo từ `/status`.
- `pmd.rs` + `riviu_pmd.py`: chọn đúng một backend; launch app với đủ ba env;
  relay `8906`, stream `9093`; retry launch có giới hạn; token qua environment;
  kiểm tra/cài IPA từ `RIVIU_RTMMO_IPA` khi bundle thiếu. Chế độ
  `--bootstrap-only` restart agent text mà không sinh relay thứ hai.
- `nurture/actions.rs`: lấy contact sheet ba frame và chạy grounded generate +
  independent verify trước khi mở UI; nếu semantic gate không đạt thì bỏ lượt,
  không dùng pool comment chung;
  tap input `(120,640)` qua native tap, `/wda/keys` nhận một phần tử chứa cả câu;
  chỉ tap Gửi
  khi drawer ban đầu đúng `Open` và một **frame mới** chuyển `Open -> SendArmed`;
  chỉ tăng counter/ghi DB khi frame trở lại `CommentDrawer::Open` (nút đã tắt).
  Nếu drawer đã `SendArmed` trước lúc gõ thì đó là draft cũ: đóng và bỏ lượt,
  tuyệt đối không append/gửi rồi gán nhầm nội dung mới.
- `UiSession::supports_text_input()`: RT-MMO đi đường chữ; stock giữ nguyên emoji
  fallback. Một lượt RT đã gõ nhưng không armed sẽ đóng UI và bỏ lượt, không trộn
  thêm emoji vào draft có thể còn sót.
- Recovery cứng chỉ dành cho `UiErrorKind::Transport`; session/timeout chỉ thử
  session mềm, không recycle agent vì health probe. Với job comment, cả soft/hard
  recovery đều dựng fresh text session, **mở lại MJPEG thành công**, rồi mới thay
  đồng thời session của feed + watcher. Fresh RT session đã dừng stream cũ; bỏ
  bước `ensure_stream` ở recovery sẽ làm engine chạy tiếp mà không có frame producer.

Đã bundle/cài/trust release `idbagent.ipa` hợp lệ (SHA-256
`8A24847099495FF70B998522692C43F00DD16B90F698BDA6953A73F5D33002EA`). Hai IPA
trong `C:\RouterMMO iOS\resources\` có cert hết hiệu lực (`0xe8008018`), đừng thay
bản bundle bằng chúng.

Live proof 28/07/2026 trên máy test:

- probe ASCII: comment `Hay qua ban oi`, count `2 -> 3`, ảnh
  `%LOCALAPPDATA%\Temp\riviu-live\manual-comment-probe-20260728-1601\12-sent.png`;
- probe Unicode: comment `Hay quá 🔥`, count `6 -> 7`, ảnh
  `%LOCALAPPDATA%\Temp\riviu-live\ordered-unicode-probe\03-sent.png`;
- harness chạy đúng product path tự bootstrap: `comments=1`, log
  `đã gửi bình luận chữ (xác nhận nút gửi tắt)`, không request lỗi, artifact
  `%LOCALAPPDATA%\Temp\riviu-live\harness-comment-product-fix-20260728-172255`.
- sau hardening DVT env/strict MJPEG/recovery: harness 2 video gửi `comments=1`,
  fresh session 8 ms, `keys` 623 ms, 0 request lỗi, 0 recovery, artifact
  `%LOCALAPPDATA%\Temp\riviu-live\final-comment-20260728-182428`.
- bản release cuối sau generation/retry/stock-order hardening, chạy
  `--steady chatty`: 3 video gửi **2 bình luận chữ**, cả hai xác nhận nút Gửi tắt,
  `keys` 418–536 ms, 0 recovery, artifact
  `%LOCALAPPDATA%\Temp\riviu-live\final-chatty-20260728-185724`.

Comment thứ hai của vòng harness bị video đổi sang LIVE đúng lúc tap rồi bật modal
chính sách; frame dump xác nhận đây là chuyển màn hình, không phải text session
regression. Không nới invariant frame để đếm lượt đó.

#### Lịch sử điều tra stock WDA (giữ lại để không thử lại)

Đã chạy được: sinh comment vision (bám đúng nội dung video), pool fallback, mở
drawer, đóng drawer an toàn. **Chưa được**: tap vào ô "Thêm bình luận…" không bật
bàn phím, nên `/wda/keys` gõ vào hư không.

Đã thử và **đều thất bại** (đừng thử lại nếu không có ý tưởng mới):

| Cách | Kết quả |
|---|---|
| `/actions` tap 60 ms | không focus |
| `/actions` tap giữ 200 ms | không focus |
| `/wda/tap` (XCUICoordinate) tại x = 60/120/150/200 | không focus |
| `/wda/touchAndHold` 0.2 s | không focus |
| tap hai lần | không focus |
| tìm `XCUIElementTypeTextField` / `TextView` ở depth 10 | không tồn tại |
| nâng `snapshotMaxDepth` lên 20 / 50 rồi tap (feed foreground) | **runner treo ngay** |
| nâng depth lên 30 **khi drawer đang mở** + tìm `TextView` | query 10.4 s, tìm 0 element, rồi **treo** |
| tap nút "Bình luận" giữa drawer trống (187, 499) | không focus |
| tap icon ảnh / sticker / @ cạnh ô nhập (259/299/339, 639) | không focus |

#### Điều tra tiếp 2026-07-27 trên máy 05101fdb (TikTok 45.8.0) — 3 hướng mới, đều chặn

Chạy một fan-out nghiên cứu (5 agent) rồi test LIVE từng hướng. Kết quả dứt khoát:

| Hướng | Kết quả | Bằng chứng |
|---|---|---|
| **TrollStore + WDA vá** (idbagent, port 8906) | **bất khả trên máy này** | iOS 16.7.15 đã vá CoreTrust (CVE-2023-41991); TrollStore chỉ tới 16.6.1, và không hạ cấp được. Đây chính là thứ TOOL TIKTOK dùng — nhưng cần máy ≤16.6.1 |
| **Gõ từng phím bàn phím ảo** | **chặn** — bàn phím không lên | Panel đáy composer 45.8.0 là inputView emoji/sticker RIÊNG của TikTok, không phải bàn phím iOS. Thử mọi nút toggle + long-press + tap ô: xám vùng đáy giữ 0.012–0.047 (bàn phím thật = 0.39) |
| **Dán clipboard** (`setPasteboard` + long-press menu "Dán") | **chặn hai tầng** | (1) `setPasteboard` CHỈ ăn khi WDA foreground — đo được: TikTok foreground → đọc lại clipboard rỗng; WDA foreground → đọc lại đúng `'XINCHAO_FG_456'`. (2) Kể cả nạp clipboard trước rồi quay lại TikTok (nội dung persist), long-press ô **chỉ hiện kính lúp, KHÔNG bung menu "Dán"** — field là UITextInput chuẩn nhưng TikTok chặn edit menu |

**‼️ SỬA LẠI (2026-07-27, tối): TEXT COMMENT KHẢ THI — WDA vá đã có sẵn trên máy.**
Kết luận "text đóng cửa" bên dưới là SAI vì tôi chỉ test qua WDA STOCK của mình.
Thực tế **2/3 máy (05101fdb, e561b690) đã cài sẵn `com.mrph.svc` = "RT-MMO 1"** —
chính là WDA vá idbagent mà TOOL TIKTOK dùng, chạy trên **port 8906**, header
`X-RT-Token: <token>`. Đã xác nhận: nó nhận `POST /session`
và `POST /wda/keys` trả OK (WDA này khiến TikTok CHẤP NHẬN keystroke — khác stock).
Đối chiếu mã nguồn TOOL TIKTOK (`scripts/tiktok/shared/wda_interact.py`):
- tap ô input **(120,640)** — GIỐNG HỆT ta; gõ qua **`/wda/keys {value:list(text)}`**.
- tap dùng **native `/wda/tap` hoặc `/wda/swipe` 1px, KHÔNG dùng W3C `/actions`**
  (TikTok timeout sau touch đầu với /actions). Toạ độ = ĐIỂM (375×667).
- Session: RT-MMO tự tạo, client poll `GET /status` lấy `sessionId` (build mới
  chặn POST /session; build 15.1.4 này vẫn cho POST /session — dùng làm fallback).
- Layout A (đã có comment): tap ô input để mở bàn phím. Layout B (chưa có comment):
  bàn phím hiện luôn.
Phần engineering này đã hoàn tất ngày 28/07/2026 trong `wda.rs`, `pmd.rs`,
`riviu_pmd.py` và `nurture/actions.rs`; lịch sử dưới đây chỉ là bằng chứng dẫn tới
thiết kế hiện tại.

**Chi tiết RT-MMO binary (đo trực tiếp trên 05101fdb):** bundle `com.mrph.svc`,
app name `csc-native-ios.app`, executable `WebDriverAgentRunner-Runner`, ký
**iPhone Distribution: Wuhan Land Resource... (ENTERPRISE cert)**, entitlement
TIÊU CHUẨN (không private hiện qua installation_proxy). Launch bằng app-launch
thường (không cần `tidevice xctest`), tự bootstrap. Chập chờn: cần `tidevice
launch com.mrph.svc` + đợi, đôi khi phải launch lại; TOOL TIKTOK quản lý vòng đời
qua `wda_manager.py`. Tap = `/wda/tap` hoặc `/wda/swipe` 1px (native, KHÔNG W3C
/actions), điểm 375×667. Gõ = `/wda/keys {value:list(text)}`. Header X-RT-Token
mọi request.

**KẾT LUẬN "tự build bản vá" (điều tra 2026-07-27, 3 research agent + thử máy):**
KHÔNG khả thi trong nỗ lực hợp lý.
- WDA stock của mình **không có dòng nào để patch**: mọi input uỷ thác cho
  `testmanagerd` (`synthesizeEvent`), provenance nằm ngoài WDA (agent đọc mã, tin
  cậy cao).
- RT-MMO là KIẾN TRÚC khác: WDA chạy **standalone** (tự lấy automation session từ
  testmanagerd KHÔNG qua host runner) — có thể kèm bơm HID đặc quyền.
- Tự dựng standalone: **plain-launch bản .xctrunner của mình → server KHÔNG lên**
  (đã thử; iOS 16 không tự bootstrap như iOS 17). Biến thành app standalone +
  tự nối testmanagerd = **không có công thức public cho iOS 16**, điểm chết là
  testmanagerd iOS 16 có cấp session cho runner tự khởi tạo không — gần như phải
  tham chiếu chính binary RT-MMO.
- **→ Dùng binary RT-MMO có sẵn** (enterprise-signed, cài .ipa lên máy nào cũng
  được) là đường thực dụng duy nhất. Đừng đi lại vòng tự-build.

**‼️ CÔNG THỨC ĐẦY ĐỦ điều khiển RT-MMO WDA (từ repo `github.com/cattfan/cloneroutermmoios`
— clone RouterMMO của chính user, có `re/findings.md` + `crates/wda`):**
- **idbagent.ipa PUBLIC**: `github.com/okeroxy/idbagent/releases`. Luôn kiểm tra
  release mới: cert enterprise của bản cũ có thể hết hiệu lực. Nếu signing team
  đổi, uninstall đúng bundle cũ rồi cài sạch; sau đó trust profile thủ công.
- **Launch PHẢI kèm ENV** `USE_PORT=8906`, `MJPEG_SERVER_PORT=9093`,
  `FARM_KEY=<token>` (thiếu env làm agent chập chờn/không bind). Production sidecar
  truyền dict env trực tiếp qua pymobiledevice3 DVT `ProcessControl.launch`; không
  đưa token vào argv của CLI con. Cần Developer Mode + DDI mount (`tidevice developer`).
- Sau kill/restart, port control cũ phải đóng trong cửa sổ bounded; không đóng thì
  fail bootstrap, không launch đè rồi nhận nhầm readiness của process cũ.
- **Session**: liveness/reuse thường poll `GET /status`. Riêng text job phải
  foreground TikTok rồi `POST /session {"capabilities":{"firstMatch":[{}]}}`;
  chỉ fallback status khi build trả đúng HTTP 404/405/501 chặn POST. Lỗi 401/500,
  response thiếu session id, transport hay timeout phải fail phiên mới, không
  attach một status session có thể đã stale.
- **Tap/swipe = sessionless `POST /wda/swipe`** với `delay/fromX/fromY/toX/toY`;
  tap dùng delta 1 px. Không fallback W3C `/actions` cho build RT-MMO hiện tại.
- **Gõ = `POST /session/{sid}/wda/keys {"value":["<cả câu>"]}`**. List từng ký
  tự có thể ACK mà không chèn; payload cả câu đã xác nhận bằng frame trên máy.
  `wda/setPasteboard`/`getPasteboard` cũng có.
- **Header `X-RT-Token: <token>` mọi request.**
- **Màn hình**: RT-MMO KHÔNG dùng `/screenshot` tin cậy (dùng MJPEG :9093) và
  KHÔNG có `/wda/window/size` (404). Quan sát bằng MJPEG :9093 hoặc `tidevice
  screenshot` (kênh DVT riêng).
- **Toạ độ**: logical 375×667; input đã live-chốt `(120,640)`. Engine dò rail
  per-frame cho icon bình luận; không hard-code layout rail.
- **CẢNH BÁO acc**: automation nhiều làm TikTok bật popup "Trạng thái tài khoản"
  (acc có nguy cơ) — cần tiết chế nhịp + xử lý popup này như các popup khác.

**Đã xác nhận end-to-end** bằng ảnh chữ trong ô, nút Gửi armed, comment hiện trong
list và harness production tăng counter; xem live proof ở §5.1.

**ĐÃ GIẢI MÃ bí ẩn "bàn phím hiện 1 lần" (2026-07-27, probe_a1/a2):** bàn phím
iOS QWERTY **luôn hiện** trong lúc long-press ô nhập (đo A1: **6/6** lần mở được
composer, xám đáy 0.387 = đúng chữ ký bàn phím). Lý do trước đây "không tái tạo
được" chỉ là **screenshot WDA nối tiếp gesture nên chụp hụt** — kênh DVT
(`tidevice screenshot`) song song bắt được ngay. NHƯNG: bàn phím đó ở **chế độ
chọn-văn-bản (magnifier)**, và **KHÔNG trụ lại sau khi nhả** — về panel emoji
ngay frame đầu (A2: xám 0.011). Tap ô (synthetic) chỉ focus (con trỏ đỏ) chứ
không dựng bàn phím nhập-liệu thường trực. ⇒ **gõ phím theo toạ độ là BẤT KHẢ**:
không có bàn phím nhập-liệu nào trụ đủ lâu để tap phím. TikTok đặt `inputView`
của ô comment = panel emoji riêng; bàn phím hệ thống không bao giờ thành input
thường trực qua WDA stock. Cùng với `/wda/keys` bị bỏ qua và dán bị chặn → **text
comment đóng cửa hoàn toàn **qua stock WDA** trên iOS 16.7.x. RT-MMO standalone
enterprise-signed là đường đang chạy; không cần TrollStore trên máy test này.

**Kết luận sau khi điều tra tới đáy**: TikTok **chặn accessibility**, nên đường
gõ text của WDA bị khoá ở tầng app — không phải lỗi code của ta. Trên máy iOS
16.7.15 không TrollStore, **emoji reaction là trần** của tính năng comment.

Bằng chứng (dump `GET /session/{id}/source` khi drawer đang mở, đã xác nhận mở):

```
<XCUIElementTypeApplication name="TikTok" accessible="false"
                            bundleId="com.ss.iphone.ugc.Ame">
```
- Toàn cây chỉ có `Other` (64), `ScrollView` (5), `Window` (3), `Button` (2).
- **Không một `StaticText` nào** dù màn hình đầy chữ (comment, caption).
- **Không một `TextView`/`TextField` nào** — ô nhập bình luận không tồn tại
  dưới dạng element.
- Mọi node `accessible="false"`.

Hệ quả, tất cả đã đo được:

| Quan sát | Giải thích |
|---|---|
| `GET /element/active` → 404 cả trước lẫn sau khi tap | không gì giữ keyboard focus |
| `/wda/keys` trả 200 mà không gõ được chữ | `FBTypeText` dùng `XCPointerEventPath initForTextInput` + `synthesizeEvent` — **không cần focus và không báo lỗi khi không có focus** (đọc `XCUIElement+FBTyping.m`) |
| `class name/chain` TextView/TextField → 0 kết quả ở depth 10 và 15 | element không tồn tại |
| liệt kê hết `Other` ở depth 15 → **timeout** | không enumerate nổi để tap ở mức element |
| tap toạ độ mở được drawer, nhưng ô nhập không focus | tap tổng hợp chạy với nút bấm, không kích hoạt được composer |
| lấy mẫu 0.4/0.8/1.2/2.0 s sau khi tap | bàn phím **không hề loé lên** rồi tắt — nó không bao giờ xuất hiện |

Khớp với lỗi đã biết của Appium
([#7868](https://github.com/appium/appium/issues/7868)): trên máy thật, tap bằng
XCUITest không dựng được bàn phím, trong khi tap tay thì được.

Đã thử và loại trừ (đừng thử lại):

| Cách | Kết quả |
|---|---|
| `/actions` tap 60 ms / giữ 200 ms | không focus |
| `/wda/tap` (XCUICoordinate) tại x = 60/120/150/200 | không focus |
| `/wda/dragfromtoforduration` delta 1 px (đúng cách TOOL TIKTOK tap) | không focus |
| `/wda/touchAndHold` 0.2 s, double-tap | không focus |
| nút "Bình luận" giữa drawer trống, icon ảnh/sticker/@ | không focus |
| `POST /element/{id}/value` (đường chuẩn Appium, tự tap để focus) | không có element để nhắm |
| `/wda/element/{id}/focuse` | không có element |
| nâng `snapshotMaxDepth` 20/30/50 (cả khi drawer mở) | treo runner |
| attach session từ `/status` | build này không trả `sessionId` |

Ghi chú: locator của WDA thuần **không có tiền tố `-ios `** — dùng
`"predicate string"` / `"class chain"`. Bản trước `wda.rs::find_and_tap()` gửi
`"-ios predicate string"` nên luôn bị từ chối; đã sửa.

### Composer bình luận: tới được, nhưng chỉ nhận emoji

Ô "Thêm bình luận…" trong drawer **không phải** control mở composer. Đường đúng:

| Bước | Toạ độ (logical 375×667) | Kết quả đo được |
|---|---|---|
| icon bình luận trên rail | (344, 371) | drawer mở |
| **icon emoji/sticker** ở thanh dưới drawer | **(299, 639)** | **composer mở** — ô nhập lớn + hàng icon + nút gửi |
| icon bàn phím trong composer | (70, 307) | đổi được chế độ, **nhưng bàn phím hệ thống không bao giờ hiện** |
| một emoji trong panel | lưới ~y 385–490 | **chèn vào ô nhập**, nút gửi chuyển đỏ đậm |
| nút gửi | (337, 307) | gửi được; ô nhập trống lại, emoji vào "Đã sử dụng gần đây" |

Đo màu nút gửi: **62.8 = disabled (hồng nhạt)**, **156.2 = armed (đỏ đậm)**.

Đo số pixel chữ trong ô nhập (x 110–730, y 450–560 px) qua từng bước:

| Bước | Pixel chữ |
|---|---|
| composer trống | 0 |
| sau khi chèn emoji | **73** |
| sau `/wda/keys` | 73 (không thêm) |
| sau **HID bàn phím cứng** `/wda/performIoHidEvent` page 0x07 | 73 (không thêm) |
| sau toggle bàn phím rồi `/wda/keys` | 73 (không thêm) |

Nghĩa là: ô nhập **đang giữ nội dung và sẵn sàng gửi**, nhưng **không nhận bất kỳ
text tổng hợp nào**. Đây là bằng chứng mạnh nhất rằng chặn nằm ở text view của
TikTok chứ không ở tầng gesture — gesture rõ ràng chạy (emoji chèn được).

### Đối chứng: stack của ta KHÔNG hỏng

Chạy đúng luồng đó trong **app Cài đặt** (`com.apple.Preferences`):

| Bước | Kết quả |
|---|---|
| `/source` depth 30 | 75 KB, 9.4 s — 165 Cell, 126 StaticText, 106 Button (cây đầy đủ) |
| tìm `XCUIElementTypeSearchField` | **n=1**, rect y=123 |
| `POST /element/{id}/click` | 748 ms — **bàn phím iOS hiện đầy đủ** (vùng đáy 244.9 → 220.2) |
| `GET /element/active` | **trả về element** — có thứ đang giữ focus |
| `/wda/keys` "hello" rồi `element/value` "abc" | ô hiện **"helloabc"** |

Nghĩa là WDA, agent, thiết bị, ảnh chụp và `/wda/keys` **đều chạy đúng**. Ảnh chụp
của WDA **có** bắt được bàn phím (đừng nghi ngờ điều này nữa). Chặn nằm riêng ở TikTok.

Đối chiếu cùng thao tác trên TikTok:

| | Cài đặt | TikTok |
|---|---|---|
| `/source` | 75 KB / 9.4 s, đủ loại element | 7 KB / depth 15, **chỉ `Other`** |
| liệt kê element | nhanh | **timeout 90 s** |
| tìm được ô text | có | **không** |
| `element/click` → bàn phím | **có** | không dùng được (không có element) |

**Lưu ý khi implement**: lưới emoji **dịch chỗ** khi panel có mục "Đã sử dụng gần
đây". Toạ độ cứng sẽ trượt — phải **dò emoji bằng hình ảnh** (blob vàng bão hoà
trên nền sáng trong vùng panel) rồi tap tâm blob.

### VÌ SAO TOOL TIKTOK COMMENT ĐƯỢC — đã tìm ra

**Nó không chạy WebDriverAgent thường.** Bằng chứng trong chính source của nó:

| Nguồn | Nội dung |
|---|---|
| `scripts/tiktok/shared/wda_session.py:59-61` | `X-RT-Token: <token>` — token của build RT-MMO, giữ ngoài repo. |
| `wda_session.py:39` | `device_port: int = 8906` — **không phải 8100** |
| `modules/wda_manager.py:129` | `8906,   # WDA idbagent/TrollStore (confirmed — binary default)` |
| `modules/wda_client_fixed.py:155` | *"Build này check X-RT-Token header trên mọi endpoint trừ /status, /health, /wda/healthcheck"* |
| `wda_client_fixed.py:21,612` | *"TrollStore/idbagent WDA"* — không có MJPEG settings như WDA thường |

Tức là họ dùng **agent vá sẵn** (`idbagent.ipa` / `dairack.ipa`, build "RT-MMO"),
cài qua **TrollStore**, chạy ở **port 8906**, mọi request kèm header `X-RT-Token`.
TrollStore cài app với entitlement tuỳ ý (không bị sandbox như app ký bằng chứng
chỉ Apple Development), nên agent đó làm được những việc WDA thường không làm được
— trong đó có focus ô nhập của TikTok.

Dự án này build **Appium WebDriverAgent 16.0.0 gốc**, ký bằng chứng chỉ Apple
Development thường, chạy port 8100. Đó là toàn bộ khác biệt. **Không phải lỗi code.**

Đoạn kết luận cũ "máy chưa có idbagent/TrollStore" đã hết hiệu lực: máy hiện có
RT-MMO enterprise-signed `com.mrph.svc`, chạy standalone trên iOS 16.7.15 và đã
gửi comment chữ như live proof ở đầu mục. Không quay lại hướng TrollStore hay tự
build WDA vá.

### ĐÃ BỎ: bình luận bằng emoji do AI chọn ❌ (bỏ 27/08/2026)

> **STATUS: không còn chạy, và mục này từng nói ngược lại trong nhiều tuần.**
> Tiêu đề cũ là "ĐÃ SHIP ✅ … đang chạy được trong engine", nhưng `do_comment` có
> doc của chính nó nói *"A failed text attempt is closed and classified **without trying
> an emoji** in the same field, which avoids accidentally posting mixed content"*, và một
> test **ghim** điều đó: `text_channel_unavailable_does_not_fall_back_to_emoji`. Tức luồng
> emoji đã bị bỏ **có chủ đích** — lý do: tránh vô tình đăng nội dung lẫn khi một lần
> gõ chữ thất bại. `choose_emoji_reaction`, `EmojiReaction`, `EMOJI_MENU` và
> `COMPOSER_EMOJI_TAB` đã xoá khỏi mã (110 dòng, 0 caller).
>
> **Số đo giữ lại ở đây vì chúng đắt để đo lại**, không phải vì chúng còn hiệu lực:
> `COMPOSER_EMOJI_TAB` là `(174/375, 359/667)` đo trên TikTok 45.8.0, và lý do nó tền tại là
> các tab bên phải nó là **sticker** — bỏng vàng xếp hàng, nên bộ dò lưới emoji khớp chúng
> vui vẻ, mà tap vào thì chèn **không gì** và nút gửi ở nguyên tắt. Nếu ai dựng lại luồng
> này, đó là cái bẫy đầu tiên.
>
> **Bài học đắt hơn cả tính năng:** một mục tự gắn "ĐÃ SHIP ✅" không bao giờ tự đục
> — nó chỉ đứng đó, và người đọc sau tin nó. Cái phát hiện ra nó là một lượt đếm
> caller, không phải một lượt đọc tài liệu.

Luồng **cũ**, giữ làm hồ sơ:

1. tap icon bình luận trên rail (vị trí dò theo frame)
2. tap **icon emoji ở thanh dưới drawer** `(0.797, 0.958)` → composer mở
3. `choose_emoji_reaction()` — model vision xem frame, chọn 1 trong 6 cảm xúc
4. **tap tab ☺ `COMPOSER_EMOJI_TAB (0.464, 0.538)`** — bắt buộc, xem dưới
5. `find_emoji_grid()` dò lưới emoji **trên frame sau khi đổi tab** rồi tap ô
6. chờ nút gửi chuyển đỏ đậm (bằng chứng emoji đã vào ô); trượt thì **thử ô kế
   bên rồi ô hàng dưới** trước khi bỏ cuộc
7. tap gửi, chờ nút tắt lại (bằng chứng đã gửi)
8. `close_comment_ui()` — đóng và **xác nhận đã về feed**, tối đa 3 lần

Ngưỡng đo được: nút gửi **62.8 = trống**, **156.2 = có nội dung**; hằng số
`SEND_ARMED_REDNESS = 100`. Giống hệt nhau trên cả hai máy và cả hai bản TikTok
đã thử — đây là hằng số bền, không phải thứ phải hiệu chỉnh lại mỗi máy.

#### Hai cái bẫy đã ngã vào, đừng ngã lại

**Panel nhớ tab cuối cùng.** Các tab bên phải tab ☺ là **sticker pack** (ví dụ
"Yellow Dog"). Sticker cũng là khối màu vàng xếp thành hàng, nên
`find_emoji_grid()` khớp y như emoji — nhưng **tap sticker không chèn gì**, nút
gửi không sáng. Live run trên TikTok 45.8.0 mất *toàn bộ* lượt comment vì chuyện
này, và log chỉ ghi "bỏ qua bình luận" nên không ai biết. Bước 4 là bắt buộc và
vô hại: bấm khi đang ở lưới emoji thì không có tác dụng gì.

**Một lần tap đóng chưa hết.** `dismiss_drawer()` đóng drawer nhưng không đóng
composer nằm chồng lên. Lượt hỏng để lại composer, lượt sau tap icon bình luận
trúng vào chính điều khiển của composer → hỏng tiếp. Dấu hiệu nhận ra: lý do lỗi
**xen kẽ đều đặn** NotArmed → NoComposer → NotArmed → NoComposer. Luôn dùng
`close_comment_ui()` ở *mọi* nhánh thoát, kể cả nhánh thành công.

`CommentResult` nêu đích danh bước hỏng (`không mở được khay bình luận`,
`khay mở nhưng không lên được composer`, `composer lên nhưng không thấy lưới
emoji`, `đã chọn emoji nhưng nút gửi không sáng`, `đã bấm gửi nhưng nút không
tắt`). Đừng gộp lại thành một dòng chung — chính vì gộp mà hai lỗi trên nằm im
qua nhiều lần chạy.

Kết quả live:

| Vòng | Máy | Video | Tim | **Bình luận** | Popup | Recovery | Lỗi request |
|---|---|---:|---:|---:|---:|---:|---:|
| emoji #1 (7p) | a99f4bd9 | 10 | 1 | **3** | 5 | 0 | 0 |
| emoji #2 (7p) | a99f4bd9 | 17 | 4 | **3** | 7 | 0 | 0 |
| trước khi sửa tab ☺ | 05101fdb | 9 | 0 | **0** | – | 0 | 0 |
| sau khi sửa (7p) | 05101fdb | 12 | 0 | **4** | 3 | 0 | 0 |

Thất bại đều **an toàn** — đóng composer sạch, không bao giờ gửi nhầm, vì chỉ
bấm gửi sau khi *thấy* nút đỏ.

**Đường text** (`generate_vision_comment`) vẫn còn nguyên và có test — bật lại
được ngay khi có agent vá focus được ô nhập.

### Đã thử nốt: bấm phím trên bàn phím ảo

Ý tưởng đúng (phím là nút, mà tap nút thì luôn ăn) nhưng **bàn phím không bao giờ
hiện** nên không có phím để bấm:

| Thử | Kết quả |
|---|---|
| tap ô nhập **trong composer** (210, 252) — khác ô pill ở drawer | ảnh **có con trỏ nhập đỏ** → ô ĐANG focus, nhưng panel vẫn là emoji |
| tap icon ⌨️ giữa hàng (68, 307) | ảnh **giống hệt từng byte** trước/sau — tap không có tác dụng |
| tap @ (108, 307) | không ra bàn phím |
| tap 🔍 tìm emoji (30, 359) | composer đóng lại |
| tap icon ảnh (27, 307) | không ra bàn phím |

Nghịch lý cốt lõi: **ô nhập có focus (con trỏ nhấp nháy) nhưng iOS không dựng bàn
phím hệ thống, và mọi text tổng hợp đều rơi vào hư không.** Trong khi cùng agent
đó, ở app Cài đặt, bàn phím hiện đầy đủ và gõ được. Đây là hành vi riêng của
TikTok với touch tổng hợp.

**Còn lại các đường cho comment TEXT:**
2. **Đổi WDA sang bản cũ** — sau khi thấy gesture chạy mà text view từ chối mọi
   input tổng hợp, **khả năng thành công thấp**; chặn không nằm ở tầng WDA.
3. Ngoài WDA (tweak jailbreak/TrollStore, API) — ngoài phạm vi stack hiện tại.

### 5.2 Xác nhận tim ✅

Ngưỡng là **tuyệt đối**: `LIKE_FILLED_REDNESS = 90`. Đo trên 11 frame thật của
`05101fdb` — tim đã đầy **111 / 121.8 / 122.4 / 122.6**, tim rỗng **−25.9 …
58.7**. Có fixture + test ghim cả hai phía.

**Đừng quay lại ngưỡng tương đối `before + 40`.** Nó hỏng cả hai chiều trên
video nền đỏ: nền nâng baseline lên 42 nên một lần tim thật (→60) bị đọc là
"không đổi", còn tim rỗng trên nền đỏ (58.7) thì vượt mốc `> 45` cũ và bị báo
nhầm "đã tim từ trước" → bỏ luôn không thả tim. Bản chất là *tim đầy hay không
đầy*, không phụ thuộc video phía sau — nên ngưỡng phải tuyệt đối.

`LikeResult::NotConfirmed` mang theo `before`/`best` và log in ra
`đỏ 32→32, cần >90; rail layout 1, tim y=279pt`. Đó là toàn bộ dữ liệu cần để
phân biệt "tap trượt" với "ngưỡng sai" — giữ nguyên.

### 5.2b Thẻ không có thanh hành động — **bẫy lớn nhất trên máy mới** ✅

Hai loại màn hình **vẫn hiện thanh compose nên vẫn phân loại là `Feed`**, nhưng
không có rail nào để tap:

- **thẻ LIVE trong FYP** ("Đang LIVE" / "Nhấn để xem LIVE")
- **frame đang chuyển cảnh** giữa hai video (rail mờ, nửa dưới đen)

Vì `find_action_rail()` chỉ dò badge đỏ, mà badge cũng vắng khi đã follow tác
giả, nên "không thấy badge" không thể hiểu là "đừng tap" — engine rơi về layout
2 và tap mù. Một vòng chạy **14 video liên tiếp, 0 tim**, log chỉ báo "icon không
đổi".

`rail_icons_present()` trả lời câu hỏi khác hẳn: rail thật là **một cột chữ
tượng hình trắng cách đều nhau** ở mép phải. Cần ≥ 2 vệt trắng có khoảng cách
55–80pt (đo thực 65–69pt).

| Frame | chuỗi icon |
|---|---|
| video thường | 312, 377, 443, 512 |
| đã follow, tim đã đỏ | 382, 443, 511 |
| thẻ LIVE | 1 vệt lẻ, không thành chuỗi |
| đang chuyển cảnh | 1 vệt lẻ, không thành chuỗi |

Engine gọi nó khi `find_action_rail()` trả None; không có rail thì **chỉ vuốt
tiếp**, không tim / không comment / không follow. Kết quả ngay sau khi thêm:
**10 tim / 12 lần thử thật**, 14 thẻ LIVE được bỏ qua đúng (trước đó 0/14).

**Lưu ý khi đọc số tim thấp**: tài khoản test đã like gần hết FYP của nó, nên
phần lớn lần thử trả về "đã tim từ trước" — đó là hành vi đúng, không phải lỗi.

### 5.3 Trang "Chọn chủ đề" chưa có capture thật ⚠️

Detector dùng 3 đặc trưng độc lập, có test chống false-positive, nhưng chưa gặp
trang thật. Toạ độ nút "Bỏ qua" `(0.24, 0.93)` kế thừa từ bản cũ, **chưa xác minh**.
Khi gặp thật, `RIVIU_FRAME_DUMP` sẽ lưu frame để hiệu chỉnh.

### 5.4 Phòng LIVE

Vuốt dọc **không thoát** phòng LIVE (nó cuộn nội dung của phòng). Nhận diện qua
pill "+ Follow" đỏ ở đầu phòng, thoát bằng ✕ góc trên phải (`screen::LIVE_EXIT`).

---
