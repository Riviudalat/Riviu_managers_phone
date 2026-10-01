---
name: run-riviu-managers-phone
description: Build and verify the Riviu Tauri desktop on Windows. Prefer isolated UI smoke and scoped CDP; native input and live devices require explicit scope. Distinguish unit, browser, native, phone and installer evidence.
metadata:
  origin: local
---

# Chạy và kiểm Riviu Manager

## Chọn đúng phạm vi

Đọc [developer guide](../../../docs/developer-guide.md),
[testing](../../../docs/development/testing.md) và [build/release](../../../docs/development/build-release.md).
Source/pin hiện hành thắng snapshot trong report cũ. Skill không cấp quyền cài dependency,
đóng app đang giữ fleet, chạy lịch hay public effect. Không cần mở app khi chỉ sửa tài liệu.

| Mục tiêu | Đường đi | Không chứng minh |
|---|---|---|
| Type/lint/unit | npm/Cargo/Python gates đúng scope | Native/phone |
| UI layout/mock | Playwright suite | IPC/USB thật |
| Tauri renderer/native | UI smoke scratch + CDP đúng process | DB vận hành, Google, phone |
| Fleet/automation | Controller đang chạy + approved headless harness | Platform/build khác |
| Bộ cài | Build/release runbook, checker và package startup | Public-action acceptance |

## UI smoke an toàn trên Windows

```powershell
# Tại gốc checkout; không cần điện thoại.
$d = '.claude/skills/run-riviu-managers-phone/driver.ps1'
powershell -NoProfile -File $d launch --smoke
powershell -NoProfile -File $d status
powershell -NoProfile -File $d stop
```

`launch` không đối số và `--mock` đều chọn **UI smoke**, không chỉ mock driver.
Backend dùng StartupPolicy hiện có: debug-only, scratch tuyệt đối mới, DB/log/WebView
riêng, credential RAM, không USB/sidecar/API/worker nghiệp vụ, allowlist IPC. Launcher
không tạo sẵn scratch mà backend phải claim. Isolation fail thì dừng, không fallback.

`target/run-skill/owner.json` giữ run/mode/scratch/CDP port và process identity, không
credential. CDP loopback có trong debug background; chỉ nối port của đúng PID/path/
creation time và scratch đã chứng minh. CDP không tự read-only: không invoke command
ngoài scope. Browser mock vẫn khác WebView Tauri thật.

Không attach theo tên executable hoặc “cửa sổ đầu tiên”. Có app khác đang chạy thì
launcher từ chối; không tự tắt production để có chỗ cho smoke. Vite port bị chiếm,
biến môi trường mode/profile mâu thuẫn hoặc launcher identity không đọc được đều dừng.

## Native helper

`driver.ps1` dùng `process-policy.ps1` để chọn đúng app do launcher của checkout
hiện tại tạo. Mọi thao tác phải kiểm lại PID/creation time/executable và HWND. PID
được OS tái sử dụng không là quyền thao tác process mới. Record thiếu/hỏng/ambiguous
thì từ chối; không tự chuyển sang một instance khác.

| Command | Side effect / giới hạn |
|---|---|
| `status`, `wait`, `log` | Đọc đúng phiên; tool version check có thể spawn process, không chứng nhận readiness |
| `shot <name>` | Chụp cửa sổ được phép; occluded/blank phải từ chối, không chụp desktop ngoài scope |
| `click`, `fill`, `type`, `key`, `scroll` | Input thật, cần phép rõ; prefer CDP. `fill`/`type` escape literal, từ chối control/newline; chỉ `key` nhận cú pháp SendKeys có chủ ý. Báo dispatched, chưa có readback; không replay vì ảnh không đổi |
| `drag`, `ctrlscroll` | Tạm unavailable ở native safety path; dùng scoped CDP cho UI smoke |
| `devices`, `android` | Device diagnostics có I/O, không phải offline; chỉ khi phạm vi cho phép |
| `usbmux` | Khởi dịch vụ Apple, cần quyền riêng |
| `stop` | WM_CLOSE đúng owned app; timeout báo chưa drain, không force-kill hoặc reap launcher tree |

Dev watcher có thể còn sống sau app đóng. Helper chỉ báo, không tìm/kill process theo
tên hoặc substring repo. Không gọi `stop` để dừng bản cài/checkout khác. Sau hot-reload
đổi PID, record cũ không được retarget tự động; cần quy trình launch mới được phép.
Không dùng tọa độ đã đo từ UI/version cũ để bấm. Screenshot không cấp quyền thao tác phone.

## Live mode và thiết bị

`launch --live-confirmed` chỉ khi operator đã duyệt phạm vi và controller cũ đã nhả;
cờ này ghi ý định, không chứng minh fleet rảnh. Startup thường dùng DB/credential/lịch
vận hành, có thể phát việc đã hẹn. Không launch live để kiểm typo hoặc làm screenshot.
Không public action trong nhiệm vụ chỉ build/run UI.

Dùng [diagnostics](../riviu-device-diagnostics/SKILL.md) cho transport/session/input.
Không restart ADB server khi một serial offline. Không dùng Mobile MCP/direct ADB để
né lease của Riviu. Android đã có UiAutomator2/helper/hierarchy path, không bị gate pixel
iPhone8 chung; capability vẫn phải kiểm từng action/build/locale.

`hunt_badge_4642.ps1` là scout **ngoài controller**, không chạy mặc định: exact serial,
output absolute mới và `-IsolatedDeviceConfirmed` bắt buộc. Dump có ghi/xóa temp trên
phone; nhiều vòng cần `-AllowSwipe`, force-stop là effect riêng. Không dùng trên máy
production đang upload/session; sự xác nhận không thay bằng chứng ownership. Prefer
fixture hoặc production Inspector đã được cấp quyền.

## Cổng và bàn giao

- Pins: rust-toolchain.toml, npm lock, Python locks, manifest. Không cài latest hoặc
  sửa quyền chỉ vì linter mới báo lỗi. CI có e2e nhưng hiện workflow_dispatch-only.
- Focused tests trước, full gate theo blast radius; Cargo cache chung chạy tuần tự.
- PowerShell parse/test helper bằng fake process/device, không dot-source driver để test
  vì entrypoint có Win32/filesystem. Không dùng test binary public-effect như smoke.
- `.gitattributes` bảo vệ byte-pinned assets. EOL churn cần kiểm diff và bảo toàn byte,
  không `git checkout --` để xóa thay đổi người dùng cho status đẹp.
- Báo command/exit/source/mode/scratch/PID/artifact phù hợp, không ghi token/nội dung riêng.
  Nêu đã/chưa kiểm native/device/installer; không khẳng định live từ mock/handshake.
