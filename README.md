# Riviumanagersphone

Ứng dụng desktop Tauri 2 + Rust + React để quản lý và điều khiển dàn iPhone qua
USB. Bản phân phối hỗ trợ Windows x64, macOS Apple Silicon và macOS Intel.

## Cài bản dựng

Mỗi lần push lên `main`, workflow **Desktop CI/CD** tạo ba artifact trong trang
GitHub Actions:

- `desktop-windows-x64`: bộ cài `.msi` và `.exe` (NSIS).
- `desktop-macos-arm64`: `.dmg` cho Mac Apple Silicon.
- `desktop-macos-x64`: `.dmg` cho Mac Intel.

Artifact được giữ 30 ngày. Tag dạng `v*` (ví dụ `v0.1.0`) tạo GitHub Release và
đính kèm toàn bộ bộ cài, manifest cùng SHA-256. Tag phải khớp chính xác version
trong Tauri, npm và Cargo; lệch version thì pipeline dừng trước khi phát hành.
Release đã tồn tại không bị ghi đè; muốn phát hành lại phải tăng version và tạo tag
mới.

Bộ cài đã mang sẵn Python runtime, `pymobiledevice3==10.1.0` và
`tidevice==0.12.11`; máy người dùng không cần cài Python hay pip. Trên Windows,
installer tự tải WebView2 bootstrapper khi hệ điều hành chưa có, và bản dựng
link tĩnh CRT nên không cần cài Visual C++ Redistributable.

Các prerequisite thuộc hệ điều hành/nhà cung cấp:

- Windows cần Apple Devices hoặc Apple Mobile Device Support để có USB driver và
  dịch vụ usbmux của Apple. Thành phần này không được phân phối lại trong app.
- macOS chạy app bình thường không cần Python. Xcode chỉ bắt buộc khi rebuild hoặc
  ký lại agent iPhone.
- Artifact macOS CI hiện ký ad-hoc. Khi chưa cấu hình Developer ID + notarization,
  macOS có thể yêu cầu xác nhận trong **Privacy & Security** ở lần mở đầu.

## Chạy từ source

```powershell
# Windows PowerShell
py -3.12 -m pip install -r sidecars/pymobiledevice3/requirements.txt
cd apps/desktop
npm ci
npm run tauri:dev
```

Trên macOS, thay `py -3.12` bằng `python3.12`; các lệnh còn lại giữ nguyên.

1. Cắm iPhone và chọn **Trust This Computer**.
2. iOS 17+ cần tunnel `pymobiledevice3` phù hợp với phiên thiết bị.
3. Trong Control Center, chọn **Refresh devices**.
4. Dùng **Cài / Re-sign Riviumanagersphone** khi cần chuẩn bị agent.

Mock farm chỉ dùng khi phát triển: `RIVIU_MOCK_DEVICES=1`.

## Build bộ cài local

```powershell
# Windows PowerShell
$env:RIVIU_DEFAULT_AGENT_MODE = "full"
py -3.12 -m pip install -r sidecars/pymobiledevice3/requirements-build.txt
cd apps/desktop
npm ci
npm run sidecar:build
npm run tauri -- build `
  --config src-tauri/tauri.full.conf.json `
  --config ../../target/tauri-sidecar.conf.json
```

Trên macOS/Linux, dùng `export RIVIU_DEFAULT_AGENT_MODE=full`, thay `py -3.12`
bằng `python3.12`, rồi chạy cùng các lệnh build với dấu `\` thay cho dấu `` ` ``.

`build_desktop_sidecar.py` tạo runtime native đúng kiến trúc, chạy smoke test và
ghi `runtime-manifest.json`. Package Python không liên quan trong môi trường local
được bỏ qua, nhưng mọi dependency đang hoạt động của runtime phải có mặt và đúng
exact version trong lock. Bản release chính thức dùng Python 3.12.10, Node 24.15.0
và Rust 1.95.0. Không commit thư mục `target/`; CI dựng lại sạch trên từng hệ điều
hành. Push lên `main` không đưa bundle trong `target/` vào Git; workflow sẽ dựng
`desktop-windows-x64` và upload bộ cài `.msi`/NSIS trong Actions trong 30 ngày. Tag
`v<version>` sẽ đính kèm các bộ cài đã verify vào GitHub Release.
IPA Agent là artifact ký riêng theo provisioning/UDID: muốn workflow đóng gói
đúng bản Full thì phải commit `sidecars/wda/RiviuAgent-text.ipa` cùng
`text-manifest.json`; IPA hiện tại dùng profile Xcode-managed 7 ngày, còn
installer Windows không có hạn 7 ngày. Đổi sang iPhone mới hoặc hết hạn profile
vẫn cần build/ký IPA trên Mac trước.

Luồng re-sign legacy trên Mac dùng source WDA 16.0.0 và asset đã khóa hash trong
bundle, sau đó copy sang cache người dùng để build. Nó không tải source upstream
không pin và không ghi vào app đã ký.

## Workspace

```text
apps/desktop/          Tauri + React UI
crates/core/           registry, SQLite, nurture/interaction/Flow
crates/ios-driver/     pymobiledevice3 + WDA (+ mock)
crates/signing/        credential store và luồng ký agent
crates/script-engine/  JSON/Flow runtime
sidecars/wda/          agent IPA + manifest
sidecars/pymobiledevice3/
sidecars/signer/
scripts/               build/attestation/CI artifact tooling
```

Production oracle vẫn là `sidecars/wda/RiviuAgent.ipa`; bản Full dùng candidate
kết hợp `sidecars/wda/RiviuAgent-candidate.ipa` với `text` và `pushMedia` đã build
theo transaction. Xem `AGENTS.md` trước
khi sửa runtime, WDA, IPA hoặc các gate thiết bị thật.

## Đăng carousel ảnh

Mở trang **Đăng carousel**, chọn thư mục nội dung một cấp (mỗi thư mục con là
một bài), quét rồi chọn subset bundle và phone. Ảnh `01`, `02`... được giữ đúng
thứ tự; `caption*.txt` được hash nguyên bản; `partners*.xlsx` và file ẩn bị bỏ
qua. Mapping phải một-một theo thứ tự hiển thị. Có thể tạo ngay hoặc lịch một
lần. Media được copy vào staging managed và transfer qua HouseArrest/AFC với
readback size + SHA-256, sau đó Agent native import vào Photos. Nút `Post` mở
composer ảnh TikTok, chọn đúng thứ tự ảnh, nhập caption, bấm Đăng và chờ frame
thoát khỏi composer trước khi ghi `Succeeded`; chỉ khi đó asset import mới được
cleanup. Lỗi sau khi đã bấm Đăng được ghi `Uncertain` để tránh đăng trùng.
