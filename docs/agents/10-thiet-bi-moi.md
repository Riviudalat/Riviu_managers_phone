## 10. Mở đường cho thiết bị mới (09/08/2026)

Mục tiêu là **kiến trúc nhận thêm được lớp máy mới**, không phải hiệu chỉnh
thêm máy. Hiện vẫn **chỉ iPhone 8 được hiệu chỉnh**, và điều đó phải hiển nhiên
trong code chứ không nằm rải rác thành hằng số.

**Lỗ hổng đã bịt, đừng để mở lại.** `nurture` **chưa bao giờ** kiểm hình học.
Registry qualification chỉ gác đường Flow/Interaction (`device_control.rs`
`negotiate`); còn `nurture::run_session` đi thẳng từ `window_size()` sang nhân
phân số iPhone 8 — nên **một máy kích thước khác sẽ bị chạm bằng toạ độ iPhone
8**, đúng thứ §3.12 cấm. Tệ hơn, khi không đọc được kích thước nó rơi về
`unwrap_or((375.0, 667.0))`, tức **bịa ra một màn hình**. Giờ cả hai đều là từ
chối có lý do.

- `screen::CALIBRATED_LAYOUTS` là danh sách **lớp màn hình đã đo thật**, hiện
  đúng một entry `iphone8-portrait-v1`. `screen::calibrated_layout()` trả `None`
  cho mọi thứ khác, và `None` **phải** nghĩa là từ chối.
- **Đây là danh sách khác với registry qualification.** Registry (rỗng, ở
  `sidecars/wda/interaction-capabilities.json`) nói máy nào được thương lượng
  năng lực; `CALIBRATED_LAYOUTS` nói bộ dò đã được đo trên màn hình nào. Cái này
  không suy ra cái kia.
- `QualifiedGeometry::validate()` **không còn** so bằng đúng `375.0/667.0`. Nó
  chỉ kiểm nhất quán nội tại (hữu hạn, dương, `logical*scale == pixel`,
  portrait). Bỏ được vì đó là kiểm **entry tĩnh của registry**, còn máy sống vào
  được là nhờ `matches()` so **bằng đúng** với một entry — registry mới là
  allowlist. Trước đây nó khiến registry **không thể chứa nổi lớp máy thứ hai**.

**Vì sao hằng số `screen.rs` không suy ra bằng công thức được.** Chúng là phân
số neo vào khoảng cách điểm **cố định tính từ mép màn**, không phải tỉ lệ của
toàn màn. `COMMENT_INPUT.1 = 640/667` là 27pt từ đáy trên iPhone 8, nhưng nhân
lên màn 844pt thành 35pt từ đáy — sai trước cả khi tính 34pt home indicator mà
iPhone 8 không có. **Thêm một lớp máy = đo lại vật lý theo mục 6, không phải
chia lại.** Và hiện `QualifiedGeometry` **không có trường safe-area nào cả**.

**Trỏ tới `adb`:** thứ tự đầy đủ là

```text
configured → RIVIU_ADB_PATH → ANDROID_SDK_ROOT → ANDROID_HOME → PATH → bundled
```

`RIVIU_ADB_PATH` chỉ thẳng vào file thực thi. Cần thiết vì một máy có thể có
platform-tools giải nén rời, không nằm trong layout SDK — khi đó không có cách nào khai
báo vị trí.

Nguồn sự thật là **`AdbOrigin`** trong `crates/android-driver/src/adb.rs`; đọc nó chứ
đừng đọc lại đoạn này, vì đoạn này đã một lần lệch. Cho tới 27/08/2026 nó ghi
`RIVIU_ADB_PATH → ANDROID_SDK_ROOT/ANDROID_HOME → PATH` — **bốn** nguồn, thiếu hai cái ở
hai đầu: `configured` (cấu hình của app, ưu tiên **cao nhất**) và `bundled` (bản `adb`
trong bộ cài, thấp nhất). Đoạn này viết trước khi bộ cài mang `adb` theo, và không được
cập nhật khi việc đó xảy ra — nên nó vừa bỏ mất lối ưu tiên cao nhất, vừa nói ngầm rằng
một máy không có SDK thì không chạy được, trong khi bản cài luôn có `adb`.

Bản trong `NOTICE` (mục 1) ghi đúng cả sáu; nó chịu lực cho lập luận giấy phép nên
**giữ**, và hai bản là số bản tối thiểu cho hai mục đích khác nhau. Đừng thêm bản thứ ba.

Backend Android chỉ tham gia fleet khi `adb version`
chạy được (`detect_driver`); nếu không, lý do nằm ở `android_unavailable_reason`,
**tách riêng** với `driver_degraded_reason` của sidecar iOS.

**Kênh gõ chữ iOS không còn phụ thuộc IPA bên thứ ba.** Xem mục 13:
`RiviuAgent-text.ipa` là bản tự build từ WebDriverAgent, đã có `text` với bằng
chứng frame thật; `RiviuAgent.ipa` (RT-MMO `com.mrph.svc`) chỉ còn là rollback
oracle. Ràng buộc thật là **free provisioning 7 ngày** và profile nhúng chỉ có
hai UDID test — fleet 20 máy cần tài khoản Apple Developer trả phí.
