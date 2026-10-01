# Môi trường và kiểm chứng

[Hướng dẫn phát triển](../developer-guide.md)

## Nguồn pin

- Rust: [rust-toolchain.toml](../../rust-toolchain.toml), [Cargo.lock](../../Cargo.lock).
- Node/package manager: [package.json](../../apps/desktop/package.json),
  [package-lock.json](../../apps/desktop/package-lock.json), version release trong
  [CI](../../.github/workflows/desktop-ci-cd.yml). Dùng `npm ci`, không trộn lock khác.
- Python iOS: `sidecars/pymobiledevice3/requirements*.txt`; GUI-service có `uv.lock`
  và Python 3.12 riêng. Không dùng kết quả audit một runtime để chứng nhận runtime kia.
- Windows cần MSVC/Windows SDK/WebView2. Chạy source cần Python theo requirements;
  người dùng bộ cài không cần tự cài Python. Xcode/signing chỉ cần khi sửa agent iOS.

`npm ci`, pip/uv sync và build có thể tải mạng/ghi cache. Không gọi chúng là read-only.
Không cài toolchain hoặc nâng dependency chỉ để một test xanh.

## Chạy app

Từ `apps/desktop`, `npm run dev` chỉ mở frontend. `npm run tauri:dev` mở backend
thật, mặc định có dữ liệu/lịch/USB thật. Với Full dev được phép:

```powershell
$env:RIVIU_AGENT_MODE = 'full'
npm run tauri:dev -- --config src-tauri/tauri.full.conf.json
```

Trước đó dừng controller cạnh tranh và kiểm công việc đang giữ máy. Không dùng
`RIVIU_MOCK_DEVICES=1` đơn lẻ để suy dữ liệu được cô lập.

### Smoke chỉ giao diện

Tại gốc repo:

```powershell
powershell -NoProfile -File .claude/skills/run-riviu-managers-phone/driver.ps1 launch --smoke
```

Launcher dùng `StartupPolicy`/UI smoke hiện có, record đúng PID/path/creation time,
scratch mới và CDP loopback. Đọc owner record trong `target/run-skill/`; chỉ nối
đúng process/scratch. `stop` graceful, không force kill hoặc tự dọn process lạ.
Không chạm credential/DB thật để nạp dữ liệu demo. Chế độ này không chứng nhận
loaded-state production, Google, điện thoại hoặc installer.

## Cổng theo thay đổi

### Rust

```powershell
cargo fmt --all -- --check
cargo test --locked -p riviu-core -- --test-threads=1
cargo test --locked -p riviu-managers-phone -- --test-threads=1
cargo test --locked -p riviu-android-driver -- --test-threads=1
cargo clippy --locked -p riviu-core --all-targets -- -D warnings
cargo clippy --locked -p riviu-managers-phone --all-targets -- -D warnings
cargo clippy --locked -p riviu-android-driver --all-targets -- -D warnings
```

Chọn crate/test hẹp trước. CI chạy workspace với features diagnostics/deployment-check
được chỉ định; không dùng all-features tùy tiện. Diagnostic binary, ví dụ
`live_nurture_test`, cần `--features diagnostics` để build nhưng **không tự được phép
run**: harness có thể phát public effect. Smart App Control chặn test binary phải báo
môi trường, không disable protection hoặc cargo clean.

### Frontend — tại apps/desktop

```powershell
npm run lint
npm test
npx tsc -b --pretty false
npm run build
npm run test:e2e
```

Playwright dùng mock Tauri bridge, cần Chromium đúng version. Screenshot baseline
chỉ thay sau khi xem actual. Vitest không thay typecheck. Lỗi mock/export khác lỗi
UI thật; timeout khác lỗi assertion. Không tăng timeout hoặc bỏ assertion tùy tiện.

### Python, tooling và docs — tại gốc

```powershell
python3 -m unittest scripts.test_check_docs -v
python3 scripts/check_docs.py
python3 scripts/collect_desktop_ci_artifacts.py verify-version
python3 scripts/collect_desktop_ci_artifacts.py verify-android-tools
python3 scripts/check_gui_boundaries.py
uv run --project sidecars/gui-service pytest sidecars/gui-service/tests
node --test scripts/test_publish_sheet.mjs scripts/publish_acceptance.test.mjs
```

`python3` ở máy hiện tại là 3.12; trên máy khác chọn interpreter đúng lock, không
suy alias. Danh sách đầy đủ Python/build/provenance/security gates nằm trong CI.
`cargo deny check`, `npm audit` và pip audit có thể truy cập advisory ngoài; xin
phạm vi mạng nếu cần, không ghi “an toàn” chỉ vì chưa chạy audit.

## Phân tầng bằng chứng

| Tầng | Chứng minh | Không chứng minh |
|---|---|---|
| Static/type/lint | Parse, kiểu, contract cấu hình | Runtime/thiết bị |
| Unit/fixture | Tình huống với input và boundary được kiểm | Live phone/Google |
| Browser mock | UI với bridge giả | IPC/native/USB |
| Tauri UI smoke | Renderer/native trong scratch cô lập | Worker và dữ liệu vận hành |
| Device canary | Đúng thiết bị/action/tuple được phép | Toàn fleet/phiên bản khác |
| Installer | Đúng artifact/runtime và clean-host behavior đã đo | Mọi phone hay tài khoản Google |

Regression quan trọng cần baseline→modified→rollback trên bản sao. Compile failure
không chứng minh assertion bắt được lỗi. Test zero cases không được ghi PASS feature.
Không chạy cùng cache Cargo song song giữa các worktree; xem `scripts/dev_compile_cache.ps1`.

## Bàn giao

Ghi scope, source commit/diff, lệnh thật/exit, baseline failures, artifact/hash và phần
chưa kiểm. Không biến “agent báo đã xong” thành evidence. Giữ raw log/ảnh/DB nhạy cảm
ngoài Git, chỉ fixture sanitized được review mới tracked. Không khôi phục backup DB
lịch sử lên dữ liệu hiện tại để tái hiện nhanh.
