# Kiểm chứng và bàn giao

Nguồn chuẩn là mục **Cổng theo thay đổi** trong `docs/developer-guide.md` và `.github/workflows/desktop-ci-cd.yml`. Đọc lại phần tương ứng trước chạy; recipe có thể thay đổi. Lệnh dưới đây là bản đồ, không phải chỉ thị chạy tất cả hoặc quyền thao tác live.

## 1. Chọn cổng theo phần đã đổi

| Phạm vi | Cổng/nguồn thật | Không chứng minh |
|---|---|---|
| Rust chung | `cargo fmt --all -- --check`; `cargo test --locked -p riviu-core` hoặc package liên quan; Clippy theo developer guide/CI | Không thay full workspace/features, native UI, phone hay installer |
| Rust/Tauri | Package `riviu-managers-phone`; full CI dùng features diagnostics/deployment-check và cấu hình test riêng | Không dùng `--all-features` vô điều kiện; crate xanh không thay Tauri dev |
| Flow compiler/import | `cargo test --locked -p riviu-script-engine`; test `src/flowImport.test.ts` và `src/components/flow/FlowImportDialog.test.tsx` của frontend | Compile/import hợp lệ không chứng minh runtime/device capability |
| Frontend | Tại `apps/desktop`: `npm test`, `npx tsc -b --pretty false`, `npm run lint`, `npm run build`, `npm run test:e2e` | Playwright có mocked Tauri bridge; không chứng minh native IPC/USB |
| FastAPI GUI service | CI gọi `uv sync --project sidecars/gui-service --frozen --python 3.12`, rồi Ruff/ty/pytest qua `uv run`; boundary check `scripts/check_gui_boundaries.py` | Không chứng minh OCR thực, packaged binary hoặc Python runtime trong bộ cài |
| Apps Script | `node --test scripts/test_publish_sheet.mjs`, dùng VM/Sheets fixture từ `docs/apps-script/publish-sheet.gs` | Không chứng minh deploy, OAuth, quyền Google hoặc Picker thật |
| Python sidecars/tooling | Danh sách `py_compile`/unittest trong CI; Gate 0 test tại `tools/interaction-gate0` | Unit test của probe không phải live probe |
| Mobile MCP launcher | `node --test scripts/test_mobile_mcp.mjs` | Không phải behavior test Riviu MCP hay phone |
| Riviu MCP syntax | `node --check scripts/riviu_agent_mcp.mjs` | Không kiểm auth/backend/action/evidence |
| Perception fixture | `crates/core/examples/gui_replay.rs` đọc CompatibilityPack, báo `deviceActions:0` | Không validator Flow, không nối phone/provider, không live acceptance |
| Build/bundle | Theo cụm stage/build/sidecar/Tauri config/provenance trong CI và skill `run-riviu-managers-phone` | Installer sinh ra không có nghĩa đã cài/chạy trên máy đích |
| Supply-chain | Recipe verify-version/verify-android-tools/provenance/verify-release của `scripts/collect_desktop_ci_artifacts.py`; CI dùng `cargo deny check`, `npm audit --audit-level=high` | Không bảo đảm hết lỗ hổng; không hạ pin/checks để xanh |

Không có entrypoint mặc định `scripts/ci-check.ps1` trong lần khảo sát này. Không bịa lệnh ngắn cho thuận miệng. Với script nhận subcommand/flags, đọc parser và CI invocation thay vì suy đoán.

## 2. Phân tầng bằng chứng

- **Static/config:** đường dẫn tồn tại, schema/manifest hợp lệ; chưa thực thi.
- **Syntax/type/lint:** mã parse/compile/quy ước đạt trong phạm vi command.
- **Unit/fixture:** hành vi đã kiểm trên input/mocks cụ thể.
- **Integration/mock browser:** boundary thật trong môi trường test; phần mock vẫn chưa live.
- **Tauri thực:** WebView/native/process/IPC trong app đã chạy và được quan sát.
- **Device thực:** exact device/task được phép đã đạt hậu điều kiện.
- **Packaged/install:** runtime từ artifact/bộ cài đã kiểm trên nền tảng đích.

Một test xanh có thể rỗng. Kỳ vọng phải độc lập với code dưới test; test gọi đường production và bắt được mutation có ý nghĩa. Khi đảo patch để chứng minh regression, phải cô lập và bảo toàn thay đổi đang có, không reset/stash mù; lỗi compile của patch đảo không chứng minh assertion bắt bug.

## 3. Side effect và quyền

- Cài dependency/build có thể tải mạng và ghi host/cache; không gọi là read-only.
- `observe` phone có session và artifact; headless có thể ghi/xóa/Post tùy example.
- Test browser/mock không cấp phép bấm máy thật, và khởi desktop không cấp phép campaign.
- `publish_sheet_prepare` có thể tạo header; không thay cho `publish_sheet_check` nếu chỉ được đọc. Deploy, write/reset/retire Sheet cần phép riêng.
- Commit/push/tag/release, bật API token, đổi permission/MCP scope là thay đổi riêng; runbook kỹ thuật không tự cho phép.
- Không dùng số lượng tool/test làm bằng chứng đã đáp ứng yêu cầu. Không tạo screenshot/fixture nhạy cảm vào tracked path không được phép; raw evidence để nơi ignored theo repo.

## 4. Xử lý cổng fail hoặc chưa chạy

1. Đọc output/exit code đầy đủ trong phạm vi cần thiết; đừng chỉ nhìn dòng cuối có chữ PASS.
2. Phân biệt regression, lỗi toolchain/config, môi trường bị chặn và test flaky. So sánh bằng chứng, không gắn nhãn flaky theo cảm tính.
3. Sửa đúng nguyên nhân bằng `systematic-debugging`, chạy lại test liên quan rồi phạm vi rộng phù hợp.
4. Nếu chưa thể chạy, ghi rõ “chưa kiểm chứng” và điều kiện thiếu; không nói “chắc chạy được”. Không tự `cargo clean`, tắt Smart App Control, giảm assertion hay bỏ verifier.
5. Không lặp lại cùng test khi code/config/input không đổi và output còn mới chỉ để tạo thêm hoạt động. Nếu phần liên quan đã đổi thì chạy lại.

## 5. Mẫu bàn giao gọn

```text
Đã làm: [thay đổi và phạm vi]
Công cụ đã dùng: [skill/MCP] → [kết quả cụ thể]
Đã kiểm: [lệnh/tác vụ] → [PASS/FAIL và phạm vi]
Chưa kiểm: [native/device/package/deployment nào còn thiếu]
Cần tiếp: [quyết định hoặc điều kiện thực sự thiếu]
```

Không cần tạo file báo cáo riêng cho mọi task. Đối với output có chủ đích giao cho người khác, tuân theo quy tắc bàn giao/publish của môi trường và bảo mật dữ liệu; skill này không tự phát hành nội dung.
