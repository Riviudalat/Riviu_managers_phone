> Historical frontend onboarding notes, retained from the README replaced on 2026-09-06.
> Counts and source line numbers below belong to that earlier snapshot.

# Frontend của Riviu Manager

React + TypeScript + Vite, chạy trong Tauri 2. **Không phải một template** — file này
từng là scaffold `create-vite` chưa sửa một chữ ("This template provides a minimal
setup…"), trong đúng thư mục người làm frontend mở đầu tiên.

Ba thứ ở đây khác một app React thường, và cả ba đều làm vỡ build nếu bỏ qua.

## `api.ts` là cửa duy nhất sang Rust

`src/api.ts` là **file duy nhất** ngoài test import `@tauri-apps/api/core` — đã kiểm
bằng grep, và có cổng giữ: `src-tauri/src/lib.rs` `include_str!("../../src/api.ts")`
rồi đối chiếu với `generate_handler!`, nên một command đăng ký mà `api.ts` không gọi
là **`cargo test` đỏ**, không phải một hàm ngủ yên.

Thêm một lệnh backend: khai trong `generate_handler!`, thêm wrapper trong `api.ts`,
gọi nó từ UI. Bỏ bất kỳ bước nào thì cổng nói ra bước còn thiếu.

## Bốn file frontend bị Rust đọc như dữ liệu test

`include_str!` băng qua ranh giới ngôn ngữ, nên sửa những file này bằng con mắt
"chỉ là TypeScript" sẽ làm đỏ một crate mà tsc không hề cảnh báo:

| file | ai đọc | giữ cái gì |
|---|---|---|
| `src/types.ts` | `core/src/types.rs` (4 test), `core/src/interaction.rs`, `nurture_commands.rs` (2) | tên/biến thể type khớp Rust, và **sàn `shared >= 24`** — xoá một type có cặp Rust là âm thầm hạ sàn đó |
| `src/viewProtocol.ts` | `view_hub.rs:923` | parity wire-format Rust↔TS cho stream |
| `src/api.ts` | `lib.rs:1130` | mọi command đăng ký đều có người gọi |
| `src/components/settings/StreamQualitySection.tsx` | `android-driver/src/scrcpy.rs:1213`, `commands/mod.rs:372` | các mức chất lượng UI chào đúng bằng các mức scrcpy nhận |

File cuối là cái bất ngờ: **một component `.tsx` chịu lực cho test của một crate
driver.** Nếu đổi nhãn trong panel đó, chạy `cargo test -p riviu-android-driver`.

## Cổng

```powershell
npx tsc -b                 # vitest XOÁ kiểu — cái này bắt thứ vitest không thấy
npx oxlint --deny-warnings
npx vitest run             # 752 test / 89 file
npx vite build
npx playwright test        # cần `npx playwright install chromium` một lần
```

`tsc -b` phải chạy sau **mỗi** file mới, không phải sau mỗi đợt: một file test import
`node:fs` từng qua được vitest, đỏ ở `tsc -b`, và đi tới CI — vì `types` chỉ có
`vite/client`, không có `@types/node`.

Type-aware lint của oxlint (`oxlint-tsgolint`) **cố ý không bật**: `tsc -b` đã là một
cổng riêng và làm việc đó đầy đủ hơn; bật thêm chỉ cộng thời gian cho cùng một câu
trả lời. Chỉ có một override, `FlowWorkspace.tsx` tắt `react-hooks/exhaustive-deps`.

## Hai cái bẫy đã cắn thật

**`vi.mock` thiếu một export** — đã xảy ra **bốn lần**. Mock một module thì phải kể
đủ mọi export mà mã dưới test gọi; thiếu một cái thì nó là `undefined`, `.catch` trên
đó **ném đồng bộ**, và cả file test đỏ với thông báo không trỏ vào nguyên nhân. Khi
một file test đỏ hàng loạt sau khi thêm một hàm API, so danh sách `vi.mock` trước.

**Cleanup của React Testing Library không tự bật.** `vite.config.ts` không đặt
`globals: true`, nên auto-cleanup của RTL **chưa từng được kích** — 14 trong 27 file
dùng `render` để lại DOM cho file sau. `src/test/setup.ts` giờ gọi `afterEach(cleanup)`
tay. Đừng bỏ dòng đó.

## Bố cục

```text
src/api.ts          cửa duy nhất sang Rust (xem trên)
src/types.ts        type dùng chung, Rust đối chiếu
src/App.tsx         khung + điều hướng
src/main.tsx        ErrorBoundary + window.onerror + unhandledrejection + boot-marker
src/crashReport.ts  gom lỗi frontend, đẩy về log app qua `log_frontend_error`
src/pages/          6 trang
src/components/     30 component + settings/, flow/
src/flow/           runtime editor Flow (@xyflow/react)
src/styles/         token + stylesheet; `designTokens.test.ts` chặn class chết
src/test/setup.ts   setup vitest, gồm afterEach(cleanup)
```

Lỗi frontend đi về log app qua `crashReport.ts` → lệnh `log_frontend_error`. Trước
đợt 27/08/2026 **không có đường nào**: một throw lúc render là màn hình trắng, không
log, không toast. Nếu thêm một chỗ bắt lỗi mới, cho nó đi cùng đường đó — có cổng
quét mã trong `lib.rs` khẳng định `main.tsx` vẫn đăng ký cả ba.

Quy ước chung của repo, và tài liệu kiến trúc: `AGENTS.md` ở gốc.
