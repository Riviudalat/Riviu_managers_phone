# Hợp đồng Flow và kiểm tra không chạm máy

Nguồn tính từ worktree Riviu, phải kiểm tra lại source hiện hành trước khi dùng.

## Bản đồ source

| Phần | Nguồn |
|---|---|
| Mô tả sản phẩm và subset import | `docs/operator-guide.md`, `docs/developer-guide.md` |
| DTO Flow V2 | `crates/core/src/flow/model.rs` |
| Compile graph/types/catalog | `crates/script-engine/src/flow.rs` |
| Validate/save/run/cancel handlers | `apps/desktop/src-tauri/src/flow_commands.rs` |
| API task/Flow dùng chung handler UI | `apps/desktop/src-tauri/src/local_api/tasks.rs` |
| Import/preview TS | `apps/desktop/src/flowImport.ts`, `apps/desktop/src/components/flow/FlowImportDialog.tsx` |
| Test importer | `apps/desktop/src/flowImport.test.ts`, `apps/desktop/src/components/flow/FlowImportDialog.test.tsx` |
| Run/revision persistence | `crates/core/src/db/flows.rs`, `crates/core/src/db/flow_runs.rs` |
| Khảo sát GenFarmer lịch sử | `docs/re/genfarmer/README.md` |

## Đường validate đã có

`JSON GenFarmer/Macro → preview draft → FlowDocumentV2 → flow_validate(document) → compile_flow(document, release_one_catalog()) → CompiledRevision hoặc diagnostics`.

- `flow_validate` hiện là hàm thuần, không nhận `AppState` hoặc device; không gọi `flow_save_revision`/`flow_run` để “validate”.
- Schema hiện được khai báo typed (`FlowDocumentV2`, schemaVersion 2). Không có căn cứ để đưa lệnh CLI tưởng tượng `flow validate` hoặc file JSON Schema không tồn tại.
- Compiler kiểm schema/graph/cycle/action và từ chối `RawHttp`/`RawWda`/`Shell` ở catalog được khảo sát. Không biến action bị từ chối thành lệnh shell fallback.
- Import legacy có trần dữ liệu riêng trong source; không tự bỏ giới hạn. Node không map được phải xuất chẩn đoán, không im lặng bỏ node rồi báo thành công.
- Preview thành công không phải bằng chứng runtime hỗ trợ Android/iOS/device cụ thể. Đọc capability/catalog tại nơi thực thi.

## Save, run và cancel là tác động riêng

`flow_save_revision` thay DB, có expected revision/concurrency check. Giữ quy tắc revision hiện hành; không ghi đè bản vừa được phiên/người khác sửa.

API hiện có nhóm GET catalog/list/detail/run-history và POST chạy/cancel:

- `POST /v1/flows/{id}/runs`
- `POST /v1/flow-runs/{id}/cancel`

Đọc parser/payload handler trước khi gọi. Năm tool Riviu MCP trong `scripts/riviu_agent_mcp.mjs` không có Flow-run; không chế ra `riviu_flow_run`. Cancel requested không chứng minh worker đã dừng hoặc thiết bị đã nhả lease.

## Ví dụ cổng kiểm thật

Các lệnh sau là gợi ý khi nhiệm vụ cho phép test. Chúng có thể biên dịch/ghi cache/temp, nhưng không được dùng làm cớ chạy thiết bị.

Từ worktree root:

```powershell
cargo test --locked -p riviu-script-engine
npm --prefix apps/desktop test -- src/flowImport.test.ts src/components/flow/FlowImportDialog.test.tsx
```

Dùng đúng toolchain/lệnh của runbook nếu repo đổi. Test importer không chứng minh backend, test compiler không chứng minh UI hay live action. Không chạy cả workspace nặng chỉ để chứng minh file skill được tải.

## Ma trận tối thiểu khi đổi runtime

- Hợp lệ/happy path; schema/action/input không hợp lệ bị từ chối.
- Selector không có, trùng, disabled; package/version/locale đổi.
- Screenshot cũ, orientation đổi, keyboard che control; không dùng tọa độ từ frame cũ.
- Timeout trước dispatch so với sau dispatch; không replay effect chưa xác định.
- Cancel trước, trong và sau effect; restart với intent/evidence còn trên đĩa.
- Hai job giành cùng device; capacity exhausted; máy offline giữa bước.
- Kết quả riêng từng máy, biến không rò qua run, retry không lặp bước đã được xác nhận.

Dùng fixture/mock tại biên I/O thật sự cần cô lập. Bài test phải gọi code production, kỳ vọng độc lập với code được test; không kiểm mỗi chuỗi source hoặc mock gọi lại chính nó.
