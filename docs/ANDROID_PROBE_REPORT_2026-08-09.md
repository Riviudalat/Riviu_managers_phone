# Android Pha 0 — báo cáo đo thật (09/08/2026)

Gate **G0** của kế hoạch thêm hỗ trợ Android. Mục đích: trả lời câu hỏi đắt nhất
trước khi cam kết phần còn lại. Kết quả **đã làm thay đổi kế hoạch** — chi tiết ở
mục "Hệ quả".

## Thiết bị đo

| | |
|---|---|
| Fleet | 15 máy đã authorize + 1 chưa cho phép USB debugging |
| Model | Samsung Galaxy S8+ — `SM-G955F` (dream2ltexx) và `SM-G955N` (dream2lteks) |
| Máy đo | `ce011711c354be2005` (SM-G955N) |
| Android | 9 (SDK 28), `arm64-v8a`, `ro.build.tags=release-keys` |
| Màn hình | Physical `1440x2960`, **Override `1080x2220`**, density 420 |
| TikTok | `com.zhiliaoapp.musically`, activity `com.ss.android.ugc.aweme.splash.SplashActivity` |
| adb | 1.0.41 (37.0.1), tải về thư mục tạm, **không cài vào hệ thống** |

Ba quan sát về hiện trạng fleet, không nằm trong kế hoạch nhưng phải ghi:

1. **`wm size` trả HAI dòng.** Physical `1440x2960` nhưng Override `1080x2220`.
   Driver phải đọc **Override** khi có; đọc nhầm Physical là `validate_geometry`
   lệch ngay từ máy đầu tiên.
2. **Máy này không root.** `su -c id` không trả gì, `ro.build.tags=release-keys`.
3. **Fleet đã được cài sẵn automation của bên thứ ba**: `com.genfarmer.uiautomator`
   v2.4.6-dirty, và IME đang hoạt động là `com.genfarmer.uiautomator/.AdbKeyboard`.
   Ngoài ra còn `com.android.xwkeyboard/.XwIME`.

   ⚠️ **Đính chính (đo ở mục 6):** tôi đã vội kết luận rằng "bàn phím Unicode Pha 3
   định viết thì đã có sẵn". **Sai.** `dumpsys package` cho thấy gói này chỉ đăng ký
   receiver `send.mock` / `stop.mock` — nó là công cụ **giả lập vị trí**, và cái IME
   mang tên `.AdbKeyboard` của nó **không nói giao thức `ADB_INPUT_TEXT`**
   (broadcast trả `result=0`, ô nhập không đổi). Pha 3 vẫn phải có kênh text riêng.

## Đo 1 — `uiautomator dump` dưới feed đang phát video

**Tiêu chí phản chứng đã tuyên bố TRƯỚC khi đo:** tỉ lệ thành công < 90% **hoặc**
p95 > 1,5 s ⇒ giả thuyết hierarchy-qua-CLI chết.

| | |
|---|---|
| Mẫu | 40, feed TikTok đang phát video |
| **Thành công** | **40/40 = 100%** |
| p50 | 2693 ms |
| **p95** | **2957 ms** |
| min / max | 2488 / 2973 ms |
| Kích thước dump | 82.520 byte (`--compressed`: 32.082 byte, **không nhanh hơn**) |

**Nỗi lo "dump chết dưới animation" KHÔNG xảy ra** — 100% thành công. Nhưng
**p95 = 2957 ms, vượt gần gấp đôi ngưỡng 1,5 s**. Tiêu chí phản chứng đã bị chạm.

## Đo 2 — tách chi phí: đường truyền hay công cụ?

| Lệnh | Feed đang phát | Màn hình chủ |
|---|---|---|
| `cat /proc/uptime` | 68 ms | 97 ms |
| `input tap` | 1590 ms | 1873 ms |
| `uiautomator dump` | 2693 ms | 2559 ms |
| `screencap -p` ra file | 2606 ms | **1158 ms** |
| `adb shell echo` (baseline) | **52 ms** | — |

Đọc ra:

- **Đường truyền adb rẻ**: 52 ms. Không phải thủ phạm.
- **`uiautomator dump` và `input tap` không đổi khi tắt video** ⇒ chi phí **nội
  tại**, không phải tranh chấp với bộ giải mã video.
- **Chỉ `screencap` bị video ảnh hưởng** (2606 → 1158 ms), hợp lý vì nó chụp
  surface đã hợp thành. Phần còn lại là encode PNG 4,2 triệu điểm ảnh trên SoC 2017.

## Đo 3 — chi phí nằm ở mỗi lần gọi, không phải mỗi phiên adb

| | |
|---|---|
| 1 tap, 1 phiên adb | 1502 ms |
| 5 tap, **cùng một** phiên adb | 7781 ms ⇒ **1556 ms mỗi tap** |

Gộp lệnh **không giúp gì**. `/system/bin/input` được xác nhận là **shell script**
khởi động một VM app_process mỗi lần gọi. `uiautomator` cũng vậy.

Ghi chú về một sai lầm trong quá trình đo, giữ lại để không lặp: tôi từng bác bỏ
giả thuyết VM vì `screencap` (native C++) cũng chậm. Bác bỏ đó **sai** —
`screencap` chậm vì lý do khác (encode PNG + tranh chấp video). Đo 3 mới là phép
thử đúng, và nó khẳng định giả thuyết VM cho `input`/`uiautomator`.

## Đo 4 — TikTok Android phơi locator gì

Từ dump 82 KB của feed: **179 node có `resource-id`, 158 giá trị duy nhất**;
**21 `content-desc`** có nghĩa.

**`resource-id` bị obfuscate** — `a1p`, `ty9`, `tyb`, `ebz`, `eqx`, `j2d`, `nwy`,
`wxb`… Đây là tên rút gọn của R8/ProGuard, **sẽ đổi giữa các bản build TikTok**.
Chỉ vài id là ổn định vì đặt tay: `viewpager`, `viewpager_container`,
`view_pager_layout_wrapper`.

**`content-desc` thì ngữ nghĩa, bằng tiếng Anh, và mã hoá cả trạng thái**:

| content-desc | Dùng cho |
|---|---|
| `Like` / **`Video liked`** | nút like **và trạng thái đã like** |
| `Follow Thúy Ngân` | nút follow, **kèm tên chủ tài khoản** |
| `Thúy Ngân profile` | avatar/profile |
| `Read or add comments. 15 comments` | nút bình luận, **kèm số bình luận** |
| `Share video. 16 shares` | chia sẻ, kèm số lượt |
| `Add or remove this video from Favorites.` | lưu |
| `For You`, `Following`, `Friends`, `Home`, `Shop`, `Create`, `Inbox`, `Search` | thanh điều hướng |

Hai điều quan trọng:

1. **`Like` → `Video liked` nghĩa là bằng chứng "đã like" đọc thẳng được từ
   accessibility**, không cần đếm pixel đỏ. Đây đúng là chỗ thứ bậc bằng chứng
   đảo chiều so với iOS.
2. **content-desc là tiếng Anh dù nội dung hiển thị tiếng Việt** ⇒ locator không
   phụ thuộc ngôn ngữ giao diện.

## Hệ quả — kế hoạch phải đổi

**Kết luận: transport gọi lệnh CLI mỗi thao tác KHÔNG dùng được cho vòng điều
khiển trên fleet này.** Một chu trình "chạm like rồi xác minh" tốn
1,55 s + 2,5 s ≈ **4 giây**, chưa tính vuốt và chụp màn hình. Nuôi acc sẽ bị chi
phí công cụ chi phối chứ không phải nhịp giống người.

Nhưng **nguyên nhân là khởi động VM mỗi lần gọi, không phải bản thân accessibility**.
Một agent thường trú giữ **một** VM + một kết nối `UiAutomation`, nói chuyện qua
`adb forward`, trả chi phí đó **đúng một lần lúc khởi động**. Đây chính là lý do
tồn tại của uiautomator2-server (Appium) và scrcpy.

**Vì vậy: APK agent phải chuyển từ Pha 3 lên Pha 1.** Nó không phải tối ưu hoá —
nó là vé vào cửa. CLI vẫn giữ, nhưng chỉ cho vòng đời (`pm install`, `am start`,
`am force-stop`, `getprop`, `wm size`), nơi 1–2 giây là chấp nhận được.

**Chiến lược locator cũng đảo:** kế hoạch định thêm biến thể `ResourceId`. Số đo
nói ngược lại — **`content-desc` (đã map sẵn vào `AccessibilityId`) là chiến lược
chính**, còn `resource-id` obfuscate là loại **kém** ổn định hơn, chỉ dùng cho
vài id đặt tay như `viewpager`.

## Đo 5 — chạm like rồi đọc lại hierarchy (đã được cho phép)

Tìm node `content-desc="Like"`/`"Video liked"`, lấy tâm bounds, `input tap`,
dump lại:

```
TRUOC       : content-desc = 'Video liked'   tai (996,1250)
sau tap     : content-desc = 'Like'
sau tap lai : content-desc = 'Video liked'
```

**Hierarchy phản ánh đúng trạng thái like.** Chạm bằng toạ độ lấy từ hierarchy
hoạt động, và bằng chứng "đã like" đọc thẳng từ accessibility. Đây là bằng chứng
quyết định để **không port tầng CV sang Android**.

Video đó vốn đã ở trạng thái đã-like trước khi đo và kết thúc vẫn vậy — không đổi
trạng thái tài khoản.

## Đo 6 — gõ tiếng Việt

Mở khay bình luận qua `content-desc="Read or add comments…"`, chạm ô nhập
(`EditText` xác nhận `focused="true"`, `mInputShown=true`):

| Cách | Kết quả |
|---|---|
| `am broadcast -a ADB_INPUT_TEXT --es msg …` | `result=0`, ô nhập **không đổi** — IME có sẵn không nói giao thức này |
| `input text 'Mon nay ngon'` (ASCII) | **được** → ô nhập hiện `Mon nay ngon` |
| `input text ' cuc ky'` (ASCII, có dấu cách) | **được** → `Mon nay ngon cuc ky` |
| `input text ' ngon quá đi mất'` (tiếng Việt) | **`Killed`** — tiến trình chết, không gõ được gì |

Tiếng Việt **không chỉ mất dấu mà làm chết hẳn tiến trình `input`**. Kênh text
riêng là bắt buộc, đúng như Pha 3 đặt ra.

Dọn dẹp: xoá sạch ô nhập (`input keyevent 67 67 … ` — **`input keyevent` nhận
nhiều keycode trong MỘT lần gọi**, nên chia đều được chi phí VM), đóng khay bằng
hai lần BACK. **Không bấm gửi, không đăng gì.** Ô nhập trở về placeholder
`Add comment...`.

## Đo 7 — kiểm kê toàn fleet

16 máy đã authorize:

| Nhóm | Số lượng | Model | Android | Màn hình | TikTok |
|---|---|---|---|---|---|
| A | **15** | Galaxy S8+ `SM-G955F`/`SM-G955N` | 9 (SDK 28) | `1080x2220` | 14/15 có |
| B | **1** | Xiaomi `23021RAAEG`, SoC `SM6225` | **15 (SDK 35)** | `1080x2400` | chưa có |

- **Fleet KHÔNG đồng nhất.** Phần cứng và Android cách nhau 6 phiên bản, và độ
  phân giải khác nhau. Registry qualification phải chứa được cả hai lớp.
- **Không máy nào root.** `su -c id` không trả `uid=0` ở bất kỳ máy nào, và quét
  toàn fleet **không thấy** Magisk / SuperSU / KernelSU. Máy root mà chủ dự án nói
  hoặc chưa cắm, hoặc root bằng đường khác chưa cấp quyền su cho shell.
- **Pin rất thấp ở nhiều máy**: 7 máy ở mức 1%, vài máy 4-10%. Phải xử lý trước
  khi chạy chiến dịch dài.

## Đo 8 — cùng lệnh, hai lớp phần cứng

| Lệnh | S8+ (Android 9) | Xiaomi (Android 15) |
|---|---|---|
| `adb shell echo` | 52 ms | 62 ms |
| **`input tap`** | **1502 ms** | **129 ms** |
| **`uiautomator dump`** | **2693–4239 ms** | **2684 ms** |
| `screencap` | 1158–2606 ms | 540 ms |

Đây là phép đo làm sắc lại kết luận:

- **`input tap` chậm là chuyện riêng của phần cứng/Android cũ** — Android 15
  nhanh hơn **12 lần**. Nên giải thích "khởi động VM" ở Đo 3 đúng cho fleet S8+
  nhưng **không phải quy luật chung**.
- **`uiautomator dump` chậm trên CẢ HAI lớp** (2,7 s và 2,7–4,2 s), không liên
  quan phần cứng ⇒ **chi phí nội tại của công cụ CLI**.

⇒ Kết luận "cần agent thường trú" **vẫn đứng, và còn chắc hơn**: nó cần cho
**hierarchy trên mọi máy**, và thêm cho **thao tác chạm trên 15/16 máy** của fleet.

## Phụ lục — cạm bẫy khi đo, để không lặp lại

1. **`\$?` trong chuỗi PowerShell không phải escaping.** PowerShell nội suy `$?`
   thành `True`, làm 40/40 lần dump thành công bị đếm nhầm là 0/40 thất bại. Dùng
   backtick hoặc tránh hẳn `$?`.
2. **`Get-Content` đọc dump XML ra sai dấu tiếng Việt** (`Thúy Ngân` →
   `ThÃºy NgÃ¢n`). Đó là lỗi giải mã phía Windows, **không phải lỗi máy** — file
   trên máy là UTF-8 đúng. Đọc bằng `-Encoding utf8`.
3. **`adb devices` khởi động daemon ở `tcp:5037`.** Một server mỗi host; app phải
   nhận quyền sở hữu tường minh để không đánh nhau với Android Studio.
