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

Cây code chưa sạch rustfmt, nên chỉ sửa format các hunk mình viết.
`rustfmt --check <file>` còn báo diff của mọi submodule `mod` dưới đường dẫn của file đó,
nên phải lọc theo đường dẫn được báo. Với `core.autocrlf=true`, file trong worktree là CRLF
nhưng `grep -c $'\r'` của Git Bash vẫn báo 0. Muốn khẳng định một file là LF thì đếm byte CR
bằng Python.

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

- Playwright mặc định dùng cổng 1421 và bật `reuseExistingServer`. Nếu nhiều worktree
  chạy song song trên cùng cổng, Playwright sẽ âm thầm test dev server của worktree khác.
  Mỗi worktree chạy song song đặt cổng riêng, ví dụ `$env:RIVIU_E2E_PORT = '1431'` rồi
  `npm run test:e2e`. Không đặt thì vẫn là 1421 như CI. Vite chạy với `--strictPort`,
  nên khi cổng bị chiếm nó báo lỗi luôn, không lặng lẽ chuyển sang cổng khác.
- `toHaveScreenshot` với `maxDiffPixelRatio: 0.002` vẫn cho qua một nút viền phụ (~2,5k px
  ở 1440×900) so với baseline cũ. Sau khi đổi UI, re-bless trang bị ảnh hưởng bằng
  `--update-snapshots=all -g <trang>`, không chỉ `=changed` (đo 2026-10-11).
- `getByText` của Testing Library chỉ khớp text node của chính phần tử; bọc nhãn trong span làm hỏng
  `getByText(…, { selector })`. Ưu tiên truy vấn theo role/name hoặc `.closest()`. `getByTitle`/`getByText`
  của Playwright khớp chuỗi con và không phân biệt hoa thường, nên tooltip trích tên nút khác (hoặc
  title chứa "danh sách" cạnh nút "Danh sách") làm selector mơ hồ; dùng `{ exact: true }`.
- Worktree Orca mới chưa có `sidecars/gui-service/.venv`, nên 2 test roundtrip của
  `gui_service` trong `riviu-managers-phone` báo `gui_service_missing`. Chạy
  `uv sync --project sidecars/gui-service` trong worktree, hoặc ghi đó là lỗi môi trường.
- `scripts/dev_compile_cache.ps1` đặt `$ErrorActionPreference='Stop'`; trên PowerShell 5.1,
  chuyển hướng output bên trong PowerShell (`*>`) biến stderr tiến độ của cargo thành lỗi dừng.
  Gọi script từ Bash và chuyển hướng bên ngoài PowerShell (`powershell … ; exit $LASTEXITCODE > log 2>&1`).
- Fixture e2e nào ghi đè `google_sheets_status` cũng phải trả lời
  `google_sheets_verify_readonly`. Nếu không, bước tự kiểm tra read-only biến panel
  Sheet thành cảnh báo.

### Cổng tập trung theo vùng thay đổi

Worker chạy cổng của vùng mình sửa, cộng `cargo fmt`/lint/typecheck. Coordinator vẫn chạy
**toàn bộ** cổng CI một lần trước khi phát hành hoặc merge lô. Danh sách này giúp chọn
nhanh hơn, không bỏ bớt cổng nào, timeout hay assertion nào.

| Thay đổi | Chạy | Bỏ được cho tới cổng đầy đủ |
|---|---|---|
| Chỉ UI (`apps/desktop/src`, không đổi `api.ts`/IPC) | `npx tsc -b`, `npm run lint`, Vitest của file/thư mục đã sửa, spec Playwright liên quan (`-g`/tên file) | Mọi test Cargo |
| IPC (`api.ts`, `types.ts`, `generated-ipc.ts`, command) | Hàng trên + `export_ipc`, `check_generated_ipc.py`, `cargo test -p riviu-managers-phone` lọc theo module, clippy desktop | `riviu-core` đầy đủ, nếu core không đổi |
| Chỉ driver (`crates/android-driver` hoặc `ios-driver`) | test + clippy của crate đó | Vitest, Playwright, `riviu-core` |
| Core (`crates/core`) | `cargo test -p riviu-core <module>` cho module đã sửa, clippy core; thêm desktop nếu kiểu public đổi | Vitest, Playwright |
| Command/lifecycle desktop | `cargo test -p riviu-managers-phone` (gồm các test quét source) + clippy desktop | Vitest/Playwright nếu UI không đổi |
| Chỉ docs | `check_docs.py --include-untracked` | Mọi cổng code |

Thời gian đo (11/10/2026 khi máy tải, và log merge gate ngày 09/10/2026):

| Cổng | Thời gian |
|---|---:|
| `npm test` (Vitest, 196 file) | 257 s. Khi máy tải có 3 test timeout, chạy lại riêng thì qua |
| `npm run test:e2e` (243 test, 10 worker) | 350 s. Khi máy tải có 2 test timeout, chạy lại riêng thì qua |
| `npx tsc -b` | 34–38 s |
| `cargo test -p riviu-core` | compile 6 phút 34 + chạy 933 s ≈ 22 phút |
| `cargo test -p riviu-managers-phone` | compile 2 phút 35–3 phút 40 + chạy ~50 s |
| `cargo test -p riviu-android-driver` | compile 1 phút 04 + chạy 18 s |

`riviu-core` với `--test-threads=1` là cổng đắt nhất. Lọc theo module trước, chỉ chạy
đầy đủ ở cổng cuối. Test timeout khi máy tải là tín hiệu về tải, không phải lý do tăng timeout.

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

Nghiệm thu trên máy thật (build native, một controller, preflight từng máy, chạy và theo dõi
Nuôi/Tương tác/Đăng bài, chụp UI thật) dùng
[bộ script nghiệm thu](../../scripts/device-acceptance/README.md); trình tự nằm ở skill
[riviu-device-acceptance](../../.claude/skills/riviu-device-acceptance/SKILL.md).

Khi một lượt chạy trên máy dừng hoặc kẹt, chụp màn hình và hierarchy **trước** khi phiên
đóng app. Ảnh chụp sau đó không còn nguyên nhân (popup, bảng, overlay đã biến mất).

Khi phân loại một lượt thử hỏng từ `stageDiagnostics`, `/element` `interrupted` nằm cuối với
`remainingBudgetMs` null là phần dọn của `leave()`, không phải lần đọc đã hết hạn. Lần đọc hết
hạn là sự kiện đứng trước nó.

Regression quan trọng cần baseline→modified→rollback trên bản sao. Compile failure
không chứng minh assertion bắt được lỗi. Test zero cases không được ghi PASS feature.
Không chạy cùng cache Cargo song song giữa các worktree; xem `scripts/dev_compile_cache.ps1`.
Worktree mới có target Cargo riêng, trừ khi `CARGO_TARGET_DIR` trỏ về `target/` của checkout
chính (`dev_compile_cache.ps1` làm việc này). Dù vậy, các crate trong workspace vẫn build
lại từ đầu cho mỗi đường dẫn worktree.

Hai bẫy khi dùng `target/` chung:
- **Độ mới.** Cargo xét độ mới theo mtime trên đường dẫn tương đối với workspace. Bản build
  của worktree khác, nếu build sau, sẽ được coi là mới với source cũ hơn của bạn. Trước mỗi
  lần gọi runner, kể cả cổng tích hợp trên main, hãy `touch` một file gốc của mỗi crate
  (`crates/*/src/lib.rs`, `crates/deployment-checker/src/main.rs`,
  `apps/desktop/src-tauri/src/lib.rs`).
- **Tham số.** `powershell -File … -CargoArgs a,b,c` truyền cả danh sách thành một chuỗi.
  Gọi bằng `-Command` với `@('a','b')`. Tên receipt không được chứa `::`.

Sổ độ trễ đọc dùng chung cho cả tiến trình. Test nào kỳ vọng cửa sổ lúc rảnh cho một máy
chưa đo chỉ đúng khi trong cùng tiến trình test có ít hơn `FLEET_MIN_PHONES` máy khác có lần
đọc gần đây. Trong vòng máy, cảnh báo `Đọc chậm <command>` (lần đọc quá 500 ms, tính cả thời
gian chờ lane) là telemetry đọc IPC.

Chế độ nghiệm thu không chạy worker đóng TikTok sau job (`state.rs:1678`), cũng không chạy
bảo trì helper tự động. Hệ quả:
- TikTok bị để phát feed sau job. Ngày 15/10 có 6/20 máy bị preflight báo quá tải
  (surfaceflinger 62–94%); bấm HOME là 20/20 đạt.
- Helper bị kẹt `owner_conflict` hoặc `HelperMaintenanceCleanupPending` không tự gỡ.

Vì vậy trước preflight hãy đưa máy về HOME. Đếm mọi `--helper-recover` và retry tay là can
thiệp.

## Bàn giao

Ghi scope, source commit/diff, lệnh thật/exit, baseline failures, artifact/hash và phần
chưa kiểm. Không biến “agent báo đã xong” thành evidence. Giữ raw log/ảnh/DB nhạy cảm
ngoài Git, chỉ fixture sanitized được review mới tracked. Không khôi phục backup DB
lịch sử lên dữ liệu hiện tại để tái hiện nhanh.
