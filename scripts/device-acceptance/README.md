# Bộ script nghiệm thu trên máy thật

Các lệnh dùng cho một vòng nghiệm thu thiết bị: build bản native, mở **một** controller
nghiệm thu, preflight từng máy, chạy Nuôi / Tương tác / Đăng bài, theo dõi, chụp UI rồi đóng
controller. Trình tự và điểm dừng nằm ở skill
[riviu-device-acceptance](../../.claude/skills/riviu-device-acceptance/SKILL.md). Hợp đồng
publish nằm ở [publish acceptance](../../docs/development/publish-acceptance.md).

Mọi script sau bước launch nói chuyện với controller qua CDP `127.0.0.1:<port>` (mặc định
47182) bằng đúng đường IPC của UI, nên admission và lease vẫn áp dụng. Chạy từ gốc repo,
Python là `py -3.12 -I`.

## Thiết lập một lần

1. Chép `local.config.example.json` thành `local.config.json` (Git bỏ qua). Mọi khóa đều tùy
   chọn: `cdpPort`, `outRoot` (mặc định `target/device-acceptance` của checkout chính),
   `dbPath`, `adb`, `expectFocus`, `numbers` (serial → số Riviu để in cho dễ đọc).
2. Tạo file chạy thật dưới `target/` (không bao giờ trong Git), theo mẫu ở `examples/`:
   - scope thiết bị: `device-scope.example.json`. Controller đọc file này với
     `deny_unknown_fields`, nên không thêm khóa nào khác (kể cả `_comment`).
   - Nuôi: `nurture.example.json`; Tương tác: `interaction.example.json`;
     Đăng bài: `publish-template.example.json`.
3. `adb` phải là bản controller đang dùng (`RIVIU_ADB_PATH`, cấu hình `adb`, rồi `PATH`). Một
   client khác version sẽ restart adb-server ngay dưới controller.

Không chép serial thật, tài khoản, Sheet id, mật khẩu hay cookie vào file tracked. Các script
không ghi tham số IPC ra log; lỗi IPC chỉ lộ `{command, code, message}`.

## Các bước

```powershell
# 1. Build (sau khi gate đã xanh trên main). Không sửa source trong lúc build.
npm run build --prefix apps/desktop
py -3.12 -I scripts/device-acceptance/build-native.py r1
#    -> target/device-acceptance/builds/native-r1.execution.json phải exitCode 0, sourceStable true
#    --optimized: profile acceptance (kế thừa dev, opt-level 2, vào target/acceptance). Vẫn mở
#    chế độ nghiệm thu, còn phần việc trên host nhanh gần bằng bản release. Dùng khi đo hiệu năng.

# 2. Đóng controller cũ (graceful, chờ pid thoát; không bao giờ kill)
node scripts/device-acceptance/close.mjs end-r0 <campaignId,...>

# 2b. Dọn album/stage riviu-* sót trên máy của đợt (controller phải đã đóng). Bộ quét của
#     controller tắt dưới dev-acceptance scope, nên album của dòng hỏng/hủy cứ tích lại và
#     làm picker TikTok chậm. Mặc định chỉ liệt kê; --apply mới xóa.
py -3.12 -I scripts/device-acceptance/sweep-media.py r1 target/device-acceptance/r1/scope.json --apply

# 3. Mở controller mới: backup DB chỉ đọc trước, từ chối nếu đã có riviu*.exe hoặc cổng bận
py -3.12 -I scripts/device-acceptance/launch.py r1 target/device-acceptance/r1/scope.json --build r1
node scripts/device-acceptance/ipc.mjs list_devices
#    Cửa sổ app bị ẩn (RIVIU_DEV_BACKGROUND=1 để bật CDP). Muốn người dùng nhìn thấy:
powershell -NoProfile -File scripts/device-acceptance/show-window.ps1 -ProcId <pid trong bản ghi launch> -Show

# 4. Preflight từng máy (chỉ đọc). Máy không đạt ghi skipped kèm lý do.
node scripts/device-acceptance/preflight.mjs r1 target/device-acceptance/r1/scope.json
#    Chỉ khi đã duyệt: --apply-baseline, --cleanup-overloaded, --helper-recover <serial,...>
#    rồi chạy lại preflight với nhãn mới để có số đọc sau effect.

# 5a. Nuôi
node scripts/device-acceptance/run-nurture.mjs --config target/device-acceptance/r1/nurture.json --out target/device-acceptance/r1/nurture
# 5b. Đăng bài: kiểm nguồn rồi preflight -> submit -> observe
node scripts/device-acceptance/inspect-source.mjs --template target/device-acceptance/r1/publish-template.json
py -3.12 -I scripts/device-acceptance/run-publish.py p1 --template target/device-acceptance/r1/publish-template.json
# 5c. Tương tác trên bài vừa đăng (chỉ bài của tài khoản publish đã duyệt)
node scripts/device-acceptance/run-interaction.mjs target/device-acceptance/r1/targets.json --config target/device-acceptance/r1/interaction.json --out target/device-acceptance/r1/interaction --publish-dir <report-dir>

# 6. Theo dõi (chỉ đọc)
py -3.12 -I scripts/device-acceptance/pubwatch.py <campaignId> 5400 --out target/device-acceptance/r1/pubwatch
bash scripts/device-acceptance/rotwatch.sh 3600
node scripts/device-acceptance/status.mjs --publish <id> --interaction <id>

# 7. Chụp UI thật
$env:SIZES = '1280x720,1366x768,1920x1080'; node scripts/device-acceptance/native-ui-shots.mjs target/device-acceptance/r1/ui
```

Mọi script có `--help`. Mỗi lần chạy runner cần thư mục `--out` hoặc nhãn mới: log và intent
mở ở chế độ exclusive.

## Tự phục hồi

- **Stale lock**: nếu process cũ đã chết, `publish_acceptance.mjs` tự xóa `harness.lock` cũ và
  lấy lock mới thay vì dừng ngay.
- **Scope sau preflight lỗi**: preflight Start protocol mà thất bại trước khi ghi
  `preflight.json` sẽ trả dev-scope về trạng thái inactive để lần chạy tiếp không cần đóng
  controller.
- **close.mjs tự tìm campaign**: khi không truyền campaign id, `close.mjs --publish-dir` tự tìm
  từ `start-intent.json`, `campaign.json` hoặc `preflight.json` trong thư mục đó.
- **inspect-source tự chạy**: `run-publish.py` tự gọi `inspect-source.mjs` khi summary không có
  hoặc cũ hơn thư mục source.

## Không replay

- Runner ghi intent (`intent.json`, `start-intent.json`, `retry-*-intent.json`) **trước**
  effect. Khi intent đã có, chạy lại chỉ theo dõi, không start lần hai.
- ACK mất sau intent ghi là `ackUnknownNeverReplay`: observe, không gửi lại.
- `uncertain`, `verifying` hay dòng đã có effect intent không bao giờ retry.
  `retry-publish.mjs` chỉ nhận `failedBeforeDispatch` không có effect intent và được
  `publish_recovery_capabilities` cho phép; một dòng không đủ điều kiện thì không gửi dòng
  nào. `retry-interaction.mjs` chỉ nhận dòng `failed` có lỗi trước effect và từ chối khi
  chiến dịch còn `running`.
- `ipc.mjs` từ chối lệnh có effect nếu thiếu `--allow-effect`; dùng runner tương ứng.

## Chạy thử không đăng (no-public rehearsal)

`rehearse-publish.mjs` chạy toàn bộ luồng chuẩn bị đăng bài (picker, sound, caption,
pre-Post reproof) trên N máy **song song** rồi dọn sạch draft/media mà **không Post, không
Sheet, không AI trả phí**. Controller chạy chế độ no-public: scratch DB/credential RAM,
`publicEffectsAllowed: false`.

```powershell
# 1. Tạo ảnh kiểm tra (chỉ cần một lần)
py -3.12 tools/no_public_fixture.py

# 2. Soạn scope JSON no-public (xem docs/development/no-public-rehearsal.md).
#    Mỗi deviceScope cần: udid, package (com.zhiliaoapp.musically),
#    expectedAccount (@...), sourceRoot, bundleId, publishFingerprint, soundPolicy,
#    allowInstalledRunner: true, allowWarmLaunch: true.

# 3. Chạy thử
node scripts/device-acceptance/rehearse-publish.mjs r1 target/rehearsal-scope.json \
  --exe target/device-acceptance/builds/<exe> --port 9377 --timeout 360 --ready-wait 120
```

Điều kiện: máy phải có UiAutomator đang chạy (từ phiên production trước) và TikTok đã cài.
Kết quả ghi `target/device-acceptance/rehearsal/rehearsal-<label>.json`. Xem `--help`.

CDP mở trước khi WebView vào trang app: trang đầu tiên là `about:blank`, chưa có
`window.__TAURI_INTERNALS__`. Script chờ trang `tauri.localhost` có Tauri bridge (tối đa
`--ready-wait`) rồi mới gọi IPC (`connectWhenReady` trong `ipc.mjs`). Ở mọi lối ra (xong, lỗi,
Ctrl+C), script gọi `no_public_shutdown`. Nếu process chưa thoát sau 30 s, script hiện cửa sổ
"Riviu Manager" của đúng pid đó và gửi WM_CLOSE, rồi chờ 90 s. Script không bao giờ kill.

## Nguồn

Bộ này được nâng từ các script một lần dưới `target/acceptance-20261010/kit/` và
`target/acceptance-baidang-20261007/`; bản gốc vẫn giữ ở đó làm bằng chứng của các vòng cũ.
