## 8. Runtime thống nhất

Artifact/profile resolve tại composition root. Nguồn:
`apps/desktop/src-tauri/src/agent_runtime.rs`, [WDA README](../../sidecars/wda/README.md),
[WDA safety](02-wda-doc-truoc-khi-sua.md) và manifest đang bundle.

- Runtime override/build default chọn profile; Full dùng candidate theo manifest,
  RT-MMO là oracle/rollback. Không tự fallback stock/mock khi auth/transport sai.
- Token không argv/log; health công khai không chứng minh protected auth.
  Session/stream cùng profile/generation và giữ đúng thứ tự lifecycle.
- Kiểm checksum trước install. Metadata/features không chứng minh runtime ready hoặc
  signature còn hợp lệ. Repair có owner, không reinstall lặp vì health probe lỗi.
- Foreground/session/frame và effect bind cùng device/session epoch. Chỉ read recovery
  có proof mới được rebind hữu hạn; không mang target cũ qua reconnect để tap.
- Bootstrap/shutdown/controller/scheduler hiện có sở hữu lifecycle. MCP/harness
  không dựng runtime cạnh tranh hoặc né admission.

[Contract hiện hành](../development/contracts.md) thay các status lịch sử.
[Snapshot runtime](../archive/technical-snapshots-2026-10-01/08-unified-agent-runtime.md)
giữ toàn bộ checkpoint, không chứng nhận phiên/bản cài mới.
