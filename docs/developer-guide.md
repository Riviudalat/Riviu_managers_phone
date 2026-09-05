# Hướng dẫn phát triển

Stack giữ nguyên: Rust workspace, Tauri 2, React/TypeScript/Vite. `src/api.ts` là biên
IPC frontend. Không thêm một control plane riêng để đi vòng ownership/admission hiện có.

## Bản đồ trách nhiệm

| Vùng | Chủ sở hữu mã | Đầu vào và đầu ra | Retry, cleanup và cổng |
|---|---|---|---|
| Shell và điều hướng | `apps/desktop/src/App.tsx`, `workspaceDraft.ts`, `deviceSurface.ts` | PageId, bản nháp, target riêng từng workspace -> vùng đang mở, dialog quyết định | stale response không ghi đè; test App/workspaceDraft, e2e pages |
| Thiết bị/preview | `useFleet.ts`, `viewStore.ts`, `DeviceTile.tsx`, `PhoneCanvas.tsx`; Rust `view_hub`, `commands`, drivers | roster/stream generation -> trạng thái và frame | cleanup theo generation/owner; viewProtocol parity, Android driver, stream-remount tests |
| Ownership/admission | Rust `lib.rs`, `state.rs`, `commands/`, core driver traits | actor/target/effect intent -> dispatch hoặc lỗi có kiểu | không duplicate effect; admission/concurrency tests trước khi đổi biên |
| Tương tác | `InteractionPopup.tsx`, core `interaction` | URL hiện tại, revision, prepared content -> campaign/assignment/evidence | uncertain không gửi lại mù; parser, effect và JobsPanel regressions |
| Nuôi | `NurturePopup.tsx`, core `nurture` | phiên, nhịp, effect plan -> quan sát/outcome/cost | budget hữu hạn, credential riêng; nurture tests và readiness UI |
| Đăng bài | `PublishPage.tsx`, core `publish`, desktop `publish_commands/{mod,preflight,execution,sheet,legacy,tests}.rs` | media/target/preflight -> durable projection và outbox | Post/URL/Sheet/cleanup tách scope; publish tests, preflight và restart recovery |
| Flow/điều phối | `components/flow`, `components/orchestration`, core `flow` | graph/node identity/revision -> execution history | không thay device Flow bằng fleet orchestration; validate/save/archive/import tests |
| App/nội dung | `AppsPage.tsx`, `MaterialPage.tsx`, library ledger | artifact snapshot + targets -> batch/item outcomes | queued cancel; restart running -> uncertain, không auto replay; ledger regressions |
| Lịch sử | `JobsPanel.tsx`, `DataPage.tsx`, aggregate query | source/filter/window -> total + page + hydrated detail | filter trước hydrate; >10.000 nguồn báo thu hẹp; projection/query tests |
| Settings/API | `SettingsPanel.tsx`, `ApiPage.tsx`, settings/local API commands | section draft/credential/listener config -> persisted value + actual listener | readback và stale-response guard; section save tests, Local API tests |
| iOS/WDA | `crates/ios-driver`, `sidecars/pymobiledevice3`, `sidecars/wda` | manifest/auth/session -> capability/transport | giữ thứ tự session-trước-stream; đọc toàn bộ §2; driver/Python gates |
| Android | `crates/android-driver`, helper APK và pinned tools | package/permission/hierarchy -> typed observation/effect | không tap theo toạ độ chưa đo; driver tests, hash/version gates |

Bảng này là ranh giới trách nhiệm hiện có, không khẳng định đã tách hết module lớn.
Khi tách module, giữ public contract và chuyển các test đọc `include_str!` cùng symbol.
Desktop Publish đã tách facade, preflight, execution, Sheet, legacy và tests; facade
giữ tên IPC. Pixel/composer live ở execution, suppression `dead_code` chỉ còn vùng
wrapper legacy. Không suy từ việc tách module này thành đã chia toàn bộ core.

Các contract bổ sung của đợt 06/09: Local API chỉ dùng read deadline cho parse;
device work đã admitted sống theo deadline riêng, response write có giới hạn riêng.
Migration token legacy phải ghi và đọc lại keyring trước khi xoá nguồn SQLite; lỗi
giữ source và báo runtime chưa chạy. Admission gate kiểm AST hàm thật với guard được
giữ trong scope. Quét nguồn và preflight dùng chung `spawn_blocking` hai slot, worker giữ
slot đến khi xong kể cả caller cancel. Các thay đổi này phải đi cùng regressions,
không dùng timeout HTTP ngoài để huỷ effect giữa chừng.

`OperationSourceRef` giữ operation/kind/source/item/máy để Tác vụ mở đúng lịch sử;
navigation không được dispatch/retry. Libraries dùng semantic target rõ ràng, restore
monitor theo exact batch ID; retry hợp lệ theo ID mới chứ không ngầm mở latest batch.

## Chuẩn bị

Đọc `rust-toolchain.toml`, `Cargo.lock`, `apps/desktop/package-lock.json` và CI đang
theo dõi. Node phải thỏa `apps/desktop/package.json#engines`; Windows cần MSVC/Windows
SDK và WebView2. Sidecar/tool pin nằm trong manifest, không đổi theo bản cài ngẫu nhiên.

Tại `apps/desktop`: `npm ci`, sau đó `npm run tauri:dev` để chạy Tauri với backend.
`npm run dev` chỉ chạy web frontend; không dùng kết quả mock để kết luận thiết bị thật.
Skill được theo dõi tại `.claude/skills/run-riviu-managers-phone/SKILL.md`; `.agents`
là bản sao runtime ignored, không phải nơi sửa nguồn chuẩn.

## Cổng theo thay đổi

Frontend, tại `apps/desktop`:

```powershell
npm test
npx tsc -b --pretty false
npm run lint
npm run build
npm run test:e2e
```

Rust, tại gốc repo:

```powershell
cargo fmt --all -- --check
cargo test -p riviu-core --locked
cargo test -p riviu-managers-phone --locked
cargo test -p riviu-android-driver --locked
cargo clippy -p riviu-core --all-targets --locked -- -D warnings
cargo clippy -p riviu-managers-phone --all-targets --locked -- -D warnings
cargo clippy -p riviu-android-driver --all-targets --locked -- -D warnings
```

Tài liệu:

```powershell
python -m unittest scripts.test_build_agents_index scripts.test_check_docs -v
python scripts/build_agents_index.py --check
python scripts/check_docs.py
cargo test -p riviu-managers-phone every_agents_section_citation_resolves --lib --locked
cargo test -p riviu-managers-phone agents_md_stays_a_door --lib --locked
```

Packaging, sidecar và Python: chạy đúng danh sách ở
[Desktop CI/CD](../.github/workflows/desktop-ci-cd.yml); [README](../README.md) giữ lệnh
điểm vào. Kiểm tra lock/version/hash, Python unit, Gate 0, audit và `cargo deny` theo
phạm vi. Không bỏ một cổng vì unit của ngôn ngữ khác đã xanh.

## Quy trình sửa và xác nhận

1. Ghi `git status`, đọc symbol và § liên quan; phân biệt thay đổi của người dùng với phần đang làm.
2. Viết regression thể hiện đúng lỗi. Khi thay contract, chạy baseline và mutant trên bản sao, rồi restore test.
3. Sửa trong module sở hữu; giữ deadline, cancellation, persistence và uncertainty semantics.
4. Chạy focused gate trước, full gate theo blast radius; phân biệt compile/unit/e2e/mock/live/installer.
5. Xem screenshot desktop/laptop, kiểm tra keyboard/focus/contrast/scroll; không chấp nhận snapshot lỗi làm chuẩn.
6. Cập nhật tài liệu chủ đề, nhật ký số mới và index. Đọc lại artifact trước bàn giao.

Không chạy harness song song desktop trên cùng USB. Thao tác công khai/cài app/chuyển
nội dung không thuộc smoke điều hướng read-only. Số lượng test và kết quả live là số
đo của một lần chạy, ghi ở nhật ký có ngày, không chép thành năng lực tuyệt đối.
