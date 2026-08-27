## 1. Dự án này là gì

**Riviu Manager** (tổ chức: **Riviu Tech**) — app desktop (Tauri + React) điều khiển
một dàn điện thoại qua USB để nuôi tài khoản TikTok: xem video, thả tim, follow,
bình luận, tự đóng popup. Hai nền tảng sau một control plane: iPhone qua
`crates/ios-driver`, Android qua `crates/android-driver`.

### 1.1 Tên: đổi ở desktop, KHÔNG đổi ở artifact iPhone (13/08/2026)

Đổi tên ngày 13/08/2026 từ `Riviumanagersphone`. Ranh giới này là **cố ý**, đừng
"làm nốt cho đồng bộ":

| Đã đổi | Giá trị mới |
|---|---|
| `productName` | `Riviu Manager` / `Riviu Manager Full` |
| `identifier` | `com.riviu.manager` / `com.riviu.manager.full` |
| Tiêu đề cửa sổ, sidebar, `index.html`, README, NOTICE | `Riviu Manager` |

| **Giữ nguyên, và vì sao** |
|---|
| `com.riviu.managersphone.agent[.xctrunner]` — nằm trong IPA đã ký, bị ghim SHA-256 ở §3.15 và trong `text-manifest.json`/`candidate-manifest.json`, và là `EXPECTED_BUNDLE_ID` của `probe_gate_bc.py`. Đổi = ký lại trên Mac + **tin cậy profile thủ công lại trên từng iPhone** (§4.0 nói rõ không có đường lập trình). |
| `sidecars/wda/Riviumanagersphone.ipa` và `sidecars/wda/branded/**` — CI gác bằng `git diff --exit-code` (`desktop-ci-cd.yml`). |
| `sidecars/wda/WebDriverAgent/**/Info.plist` — nằm trong digest của `legacy-wda-source-lock.json` (§3.18). Sửa một ký tự là vỡ lock, build đổ. |
| Thông báo "giữ app Riviumanagersphone" trong `riviu_pmd.py` và `crates/signing` — chúng nêu tên app **trên iPhone**, vốn không đổi. Đổi chữ mà không đổi app là chỉ sai chỗ cho người vận hành. |
| Literal `riviu-managers-phone` ở `state.rs::resolve_desktop_data_dir` và `SERVICE` trong `credentials.rs` — **không** suy ra từ `identifier`. Giữ nguyên chính là thứ bảo toàn SQLite (campaign, flow, cấu hình) và token agent trong Keychain. Đổi chúng là mất dữ liệu thuần, đổi lại con số 0. |
| Tên crate/binary `riviu-managers-phone` — không lộ ra người dùng, đổi thì lan sang workflow và `driver.ps1` `$ProcName` mà không được gì. |

Hệ quả đã biết và đã chấp nhận: máy đang chạy `v0.1.1` sẽ nhận bản cập nhật kế tiếp
thành **một app thứ hai nằm cạnh**, không phải nâng cấp đè. SQLite và token không mất
(chúng khoá theo tên crate, không theo `identifier`), nhưng người vận hành phải tự gỡ
bản cũ. **Đính chính 14/08 — xem 9.56:** nguyên nhân là **`productName`**, không phải
`identifier` như câu này viết ban đầu; và cái giá thật nặng hơn "phải tự gỡ": bộ cài do
updater chạy kèm `/UPDATE` nên **không tạo shortcut**, khiến mọi shortcut cũ vẫn mở
`v0.1.1` và bản cập nhật bị mời lại **mãi mãi**.

```
apps/desktop/          Tauri app (React UI + lệnh Rust)
  src-tauri/src/bin/live_nurture_test.rs   ← harness test thật, headless
crates/core/           Logic thuần: nurture flow, đọc màn hình, AI comment, DB
crates/ios-driver/     Điều khiển thiết bị: WDA, relay USB, stream, supervisor
sidecars/pymobiledevice3/riviu_pmd.py      ← lớp Python nói chuyện với iPhone
TOOL TIKTOK/           Tool Python tham khảo (chỉ đọc, không build)
docs/                  Báo cáo live test
```

Thiết bị đang dùng để test: iPhone 8 · iOS 16.7.15 · UDID
`a99f4bd9f877b2a0e3682ee24fd1c68f75ba6982` · TikTok `com.ss.iphone.ugc.Ame` ·
WDA stock `com.riviu.managersphone.agent.xctrunner`; backend bình luận chữ dùng
RT-MMO standalone `com.mrph.svc`.

---
