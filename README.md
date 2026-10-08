# Riviu Manager

Riviu Manager là ứng dụng desktop của **Riviu Tech** để quản lý và điều khiển nhiều
điện thoại Android/iOS qua một bộ điều phối chung. Ứng dụng kết hợp quan sát màn
hình, thao tác thiết bị, workflow, lịch chạy và lịch sử có bằng chứng.

**Gửi được lệnh không có nghĩa công việc đã thành công.** Riviu phân biệt thao tác
đã gửi, kết quả đã xác minh và kết quả chưa rõ; không tự đăng/gửi lại để bù một ACK
bị mất. Chỉ sử dụng trên thiết bị/tài khoản bạn được phép quản lý và trong phạm vi
được người vận hành duyệt.

## Chức năng

| Nhu cầu | Chức năng |
|---|---|
| Quản lý thiết bị | Control Center, nhóm, trạng thái kết nối, preview, điều khiển, tệp, thư viện ứng dụng (cài/gỡ/cài lại) và chẩn đoán |
| TikTok | Nuôi, Tương tác và Đăng bài với capability/preflight theo từng thiết bị |
| Đăng nội dung | Ảnh/video, caption, ghép bài–máy, nhạc, hẹn giờ, canonical link và báo cáo Sheet |
| Workflow | Flow thiết bị, Macro, My Apps và Điều phối; revision/ledger riêng cho từng lượt |
| Kết quả | Lượt chạy, tiến trình, nhật ký, artifact và thao tác phục hồi đúng phạm vi |
| Tích hợp | Local API loopback, Riviu MCP/Inspector, Google Sheets và AI tùy chọn |

Instagram/Threads mới là điểm mở rộng trong model, **chưa có engine automation**.
Mức hỗ trợ phụ thuộc package, phiên bản, ngôn ngữ, geometry và proof của từng action;
nhận diện được máy không chứng nhận mọi chức năng trên máy đó.

## Nền tảng và giới hạn

| Thành phần | Phạm vi hiện có | Điều kiện/giới hạn |
|---|---|---|
| Windows x64 | Desktop, Android, runtime iOS, GUI-service/OCR đóng gói | WebView2; USB driver theo thiết bị; installer chưa Authenticode |
| macOS Apple Silicon/Intel | Desktop và runtime iOS có pipeline build | Ký ad-hoc, chưa Developer ID/notarization; CI chưa đóng gói GUI-service như Windows |
| Android | ADB + UiAutomator2 + helper, scrcpy view và minicap evidence | USB debugging/authorization; quyền cài USB theo ROM; action chỉ mở khi capability/proof đủ |
| iOS | pymobiledevice3 + WDA/Riviu Agent, usbmux và MJPEG | Trust, signing/provisioning và transport theo iOS; Flow UI/qualification chưa parity Android |
| Linux host | Không nằm trong ma trận bộ cài được hỗ trợ | Không suy hỗ trợ từ việc một crate có thể compile |

Artifact iPhone và installer desktop là hai thứ khác nhau: profile ký iPhone có
thể hết hạn/giới hạn UDID. Không đổi tên bundle, thay IPA hoặc reinstall để thử vận
may. Đọc [WDA safety](docs/agents/02-wda-doc-truoc-khi-sua.md) trước mọi sửa đổi iOS.

## Bắt đầu với bộ cài

1. Lấy đúng installer đã được giao và đối chiếu version/hash của bản đó. Source
   trên Git không tự cập nhật ứng dụng đang cài.
2. Android: cài USB driver phù hợp nếu cần, bật USB debugging và chấp nhận hộp
   thoại trên điện thoại. iPhone: chuẩn bị Apple USB support trên Windows, Trust
   This Computer và agent/provisioning phù hợp.
3. Mở **Control Center → Quét lại thiết bị**. Dùng **Chẩn đoán** để phân biệt mất
   kết nối, chưa cấp quyền, thiếu helper/session và stream lỗi.
4. **Bảo trì → Sửa Riviu Agent** là thao tác riêng có xác nhận, không phải bước
   kiểm tra chỉ đọc. Không chạy hai controller trên cùng thiết bị.
5. Chọn đúng phạm vi trước Nuôi/Tương tác/Đăng bài; đọc preflight và xác nhận cuối.

### Cài đặt máy (chuẩn hệ thống Android)

**Thiết bị → Cài đặt máy** giữ một chuẩn cài đặt hệ thống cho cả đội máy Android và
áp dụng/kiểm tra theo một máy, một nhóm hoặc toàn bộ:

| Cài đặt | Lệnh trên máy | Mặc định |
|---|---|---|
| Tắt khóa màn hình | `locksettings set-disabled true`, rồi `wm dismiss-keyguard` | Bật |
| Tắt tự xoay | `settings put system accelerometer_rotation 0` và `user_rotation 0` | Bật |
| Luôn sáng khi sạc | `settings put global stay_on_while_plugged_in 7` | Tắt |
| Tắt màn hình sau 30 phút | `settings put system screen_off_timeout 1800000` | Tắt |
| Tắt hiệu ứng | ba khóa `*_animation_scale`/`animator_duration_scale` = 0 | Tắt |

- **Kiểm tra** chỉ đọc. Máy không trả lời được hiện **Chưa rõ**, không bao giờ được coi
  là đã đúng chuẩn.
- **Áp dụng** giữ quyền điều khiển máy như mọi thao tác thủ công: máy đang Đăng bài,
  Nuôi hoặc Tương tác bị từ chối (**Máy đang bận**), không bị chiếm quyền. Mỗi cài
  đặt chỉ được ghi khi chưa đúng và chỉ báo **Đã áp dụng** khi đọc lại đúng giá trị.
- Máy có mã PIN/mật khẩu/hình vẽ báo **Cần làm tay** và không bị gửi `locksettings`
  (trên Android 9–11 lệnh đó tính là một lần nhập sai mã). Không đọc được loại khóa
  cũng là **Cần làm tay**. Riviu không mở khóa máy có mã.
- **Tự áp dụng khi máy kết nối** (mặc định bật) áp chuẩn đã lưu mỗi lần máy Android
  cắm lại, trong lượt chuẩn bị Riviu Helper và trước khi máy nhận việc; kết quả ghi
  vào nhật ký thao tác. Máy còn chờ trong hàng chuẩn bị chưa được áp chuẩn.
- Danh mục không có gì đụng tới tài khoản, mạng, quyền gỡ lỗi USB hay dữ liệu ứng
  dụng. iPhone hiện **Chưa hỗ trợ** và không bị thay đổi.
- Đăng bài: máy có mã đang khóa bị preflight chặn với mã `device_screen_locked`; máy
  khóa lại giữa chừng trước Post kết thúc **trước khi gửi**, cùng mã, và được phép
  thử lại sau khi mở khóa. Không có Post nào được gửi lại vì lý do này.

Bundled Python, Android tools và giấy phép: [Android tools](sidecars/android/README.md),
[WDA/runtime](sidecars/wda/README.md), [NOTICE](NOTICE). Máy người dùng không cần
cài Python để chạy bộ cài. USB driver/hộp thoại tin cậy vẫn cần người vận hành.
ADB của máy được ưu tiên nếu phù hợp để tránh thay server đang dùng; nguồn chuẩn
của resolver là `crates/android-driver/src/adb.rs`. Không tự `kill-server` khi một máy offline.

### Cập nhật và chuyển máy

App không tự gọi kiểm bản mới lúc mở. Vào **Cài đặt → Bản cập nhật** để kiểm tra.
Cài cập nhật chỉ được cho phép khi fleet/hàng đợi đã được xác minh rảnh; việc dừng
và nhả thiết bị xảy ra trước khi chạy installer. MSI nhận cập nhật MSI, NSIS nhận NSIS.

Đăng nhập Orca hoặc clone Git **không chuyển DB, credential, driver hay bộ cài**.
Đọc [chuyển máy, sao lưu và phục hồi](docs/operator/installation-and-data.md) trước
khi sao chép dữ liệu hoặc quay về binary cũ.

## Chạy từ source

Đọc [chuẩn bị môi trường và kiểm thử](docs/development/testing.md) trước. Dùng
`rust-toolchain.toml`, npm `package-lock.json` và Python lock làm nguồn pin; không
trộn một node_modules cài bằng package manager khác với kết quả CI `npm ci`.

```powershell
# Tại gốc repo; đây là cài dependency, không phải phép kiểm read-only.
py -3.12 -m pip install -r sidecars/pymobiledevice3/requirements.txt
npm ci --prefix apps/desktop
```

- **Kiểm UI cô lập:** theo [run skill](.claude/skills/run-riviu-managers-phone/SKILL.md)
  và UI smoke, không chỉ đặt `RIVIU_MOCK_DEVICES=1`.
- **Chạy với điện thoại thật:** khi đã dừng controller cạnh tranh và được phép,
  tại `apps/desktop` chạy `npm run tauri:dev`. Startup thường có thể chạy lịch/worker
  đã lưu và dùng dữ liệu vận hành. Full mode theo developer runbook.
- `npm run dev` chỉ là frontend; browser mock không chứng minh Tauri/USB.
- GUI-service/OCR cần môi trường riêng: [GUI-service](sidecars/gui-service/README.md).
  Web evidence cần yt-dlp: [yt-dlp](sidecars/yt-dlp/README.md). Không có binary dev
  thì đường đó có thể unavailable; không tải/cài ngầm khi chỉ review source.

## Build và phát hành

Dùng [runbook build/release](docs/development/build-release.md). Nó bao gồm cấu hình
Google, checker, runtime iOS, GUI-service, Android package tools, frontend embedding
và kiểm artifact. Không dùng một lệnh `tauri build` thiếu overlay làm bằng chứng
bộ cài tự chứa đầy đủ.

[Desktop CI/CD](.github/workflows/desktop-ci-cd.yml) hiện chỉ có **workflow_dispatch**.
Push/PR/tag không tự chạy pipeline. Workflow có quality gates và ba nền tảng build;
artifact giữ 30 ngày. Release/tag phải khớp version npm/Tauri/Cargo, không ghi đè
release đã tồn tại. Chữ ký updater khác Authenticode/notarization của hệ điều hành.

## Kiến trúc

```text
React UI / Local API / Riviu MCP
    -> Tauri commands -> admission + device ownership
    -> Nuôi / Tương tác / Publish / Flow / Điều phối
    -> Android driver hoặc iOS driver
    -> observation + verifier -> SQLite ledger/outbox -> read models/UI
```

| Thư mục | Trách nhiệm |
|---|---|
| `apps/desktop/` | React/TypeScript/Vite, Tauri commands, bootstrap, runtime supervisors |
| `crates/core/` | Contract, SQLite, ownership, engine, verifier và artifact store |
| `crates/android-driver/`, `crates/ios-driver/` | Transport/session/input/observation theo nền tảng |
| `crates/script-engine/` | Script/Flow validation và compilation |
| `crates/signing/`, `crates/deployment-checker/` | Credential/signing legacy và kiểm deployment |
| `sidecars/` | Runtime Python, helper Android, agent iPhone, tools đã pin |
| `scripts/`, `tools/` | Gates/build/integration tooling; từng lệnh có side effect riêng |
| `docs/`, `.claude/skills/` | Hướng dẫn chuẩn, evidence lịch sử và playbook cho agent |

`src/api.ts` là biên IPC frontend. Control plane giữ ownership; không tạo một
controller cạnh tranh để né busy. SQLite giữ identity/revision/intent qua restart;
transaction không giữ qua HTTP hoặc thao tác thiết bị.

## Đăng bài: đọc đúng kết quả

Mở **Đăng bài**, quét nguồn, kiểm caption/ảnh/video/đối tác và ghép đúng bài–máy.
Nguồn hỗ trợ quy tắc tên caption/Excel và thứ tự ảnh; xem [hướng dẫn Đăng bài](docs/operator/publish.md).
Lượt mới bắt buộc Sheet đã kiểm và dọn bản media chuyển lên máy theo contract.

- **Submitted/Đã bấm Đăng:** đã qua biên thao tác; chưa đủ chứng minh bài công khai.
- **Verified:** có canonical link và proof đúng bài/tài khoản/caption/thời gian.
- **Sheet sent/readback:** nghĩa vụ ghi Sheet và bằng chứng ô/receipt là lớp riêng.
- **Uncertain/Cần kiểm tra:** giữ intent/evidence, không tự Post lại.
- Cleanup bản nhập trên phone chỉ sau proof phù hợp; không xóa nguồn PC.

Ra khỏi composer, có ảnh preview hay nhận HTTP 200 không thay các bằng chứng này.

## Tài liệu và đóng góp

- [Hướng dẫn vận hành](docs/operator-guide.md)
- [Hướng dẫn phát triển](docs/developer-guide.md)
- [Hợp đồng UI](docs/ui-reference-matrix.md)
- [Cửa vào agent](AGENTS.md) và [thiết lập skill/MCP](docs/agent-toolkit-setup.md)
- [Kho lịch sử](docs/archive/README.md): kết luận có ngày, không phải trạng thái hiện tại

Khi thay hành vi, cập nhật đúng owner document và regression. Báo rõ phạm vi đã
kiểm: syntax/unit/fixture/browser/Tauri/device/installer. Không dùng số test hoặc
một báo cáo PASS cũ để chứng nhận máy, build hoặc phiên mới.
