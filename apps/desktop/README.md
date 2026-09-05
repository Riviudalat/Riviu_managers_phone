# Frontend Riviu Manager

React + TypeScript + Vite chạy trong Tauri 2. Hướng dẫn đầy đủ:
[developer guide](../../docs/developer-guide.md), [operator guide](../../docs/operator-guide.md),
[UI contract](../../docs/ui-reference-matrix.md), [agent index](../../docs/agents/README.md).

## Chạy và kiểm tra

Từ thư mục này, dùng Node thỏa `package.json#engines` và toolchain của repository:

```powershell
npm ci
npm run tauri:dev
```

`npm run dev` chỉ là web frontend. Fixture/mock và smoke Tauri backend thật là hai
loại bằng chứng riêng. Kiểm tra frontend:

```powershell
npm test
npx tsc -b --pretty false
npm run lint
npm run build
npm run test:e2e
```

Playwright cần Chromium đã được cài cho phiên bản lockfile. Cổng toàn hệ thống nằm
trong [workflow](../../.github/workflows/desktop-ci-cd.yml), không ghim số test ở README.

## Ranh giới code

| Vùng | Contract |
|---|---|
| `src/api.ts` | biên IPC sang Rust; wrapper phải khớp command đã đăng ký |
| `src/types.ts`, `viewProtocol.ts` | parity type/wire format với Rust, có test `include_str!` |
| `src/App.tsx` | shell/route, draft navigation và target riêng từng workspace |
| `src/workspaceDraft.ts` | registry dirty/save/discard; async save có revision guard |
| `src/components/`, `src/pages/` | component/workspace theo ownership, không thêm đường invoke riêng |
| `src/styles/`, `src/index.css` | token, font, responsive layout; token tests chặn drift |
| `src/test/setup.ts` | gọi `afterEach(cleanup)` rõ ràng cho React Testing Library |
| `src/main.tsx`, `crashReport.ts` | ErrorBoundary, global error/rejection và log backend |

`StreamQualitySection.tsx` còn được test của Android driver đọc như contract UI.
Đổi giá trị quality cần chạy `cargo test -p riviu-android-driver` từ gốc repo.

## Quy tắc kiểm thử

- Vitest không thay typecheck; chạy `tsc -b` cho file mới/import mới.
- Mock API phải đủ export mà code gọi; đọc nguyên nhân thiếu mock trước khi tăng timeout.
- Snapshot chỉ đổi sau khi xem actual image; không ghim trang có lỗi làm baseline.
- Layout hẹp phải giữ filter/action/status có thể dùng; không đổi tile/canvas/mật độ thiết bị.
- Giữ actual listener khác config, uncertain khác failed và Partial khác chưa Post.

Các ghi chép onboarding cũ được giữ trong
[archive](../../docs/archive/reports/2026-08-27-frontend-readme.md).
