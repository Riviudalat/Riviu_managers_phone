# Cửa vào cho agent

Đọc [`README.md`](README.md) trước. Hướng dẫn sản phẩm:

- [`docs/operator-guide.md`](docs/operator-guide.md) — vận hành 12 trang UI
- [`docs/developer-guide.md`](docs/developer-guide.md) — hợp đồng phát triển và cổng kiểm
- [`docs/agents/README.md`](docs/agents/README.md) — ràng buộc kỹ thuật còn hiệu lực
- [`docs/agent-toolkit-setup.md`](docs/agent-toolkit-setup.md) — thiết lập và chọn skill/MCP; điểm vào điều phối là [`riviu-project-toolkit`](.claude/skills/riviu-project-toolkit/SKILL.md)

Khi làm phần AI của dự án (phân loại, xếp hạng, trích xuất hoặc kiểm nội dung),
dùng skill `typesafe-ai` và đọc tài liệu TypeSafe hiện hành theo skill. Bản Codex
cục bộ ở `.agents/skills/typesafe-ai/SKILL.md`; cách cài và phạm vi áp dụng nằm trong
[`hướng dẫn bộ công cụ`](docs/agent-toolkit-setup.md#typesafe-cho-codex).

**Trước khi sửa WDA / thiết bị iOS:** đọc hết
[`docs/agents/02-wda-doc-truoc-khi-sua.md`](docs/agents/02-wda-doc-truoc-khi-sua.md).
Bỏ qua mục đó có thể làm hỏng thiết bị thật.

Không ghi nhật ký thay đổi vào kho tài liệu này; cập nhật README hoặc hướng dẫn sản phẩm khi hành vi người dùng đổi.
