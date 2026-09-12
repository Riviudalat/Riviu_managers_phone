# Dịch vụ nhận diện giao diện

FastAPI chỉ nhận quan sát và trả ứng viên có cấu trúc. Rust sở hữu thiết bị,
ngân sách campaign, effect ledger và kết quả nghiệp vụ. Service không dùng ADB,
không chạy script do model sinh, không mở SQLite của ứng dụng.

## Phát triển

```powershell
uv sync --frozen --python 3.12
uv run pytest -q
uv run ruff check .
uv run ty check riviu_gui serve.py
```

`serve.py` nhận bootstrap qua stdin, bind cổng động 127.0.0.1, trả handshake
trên stdout. Đóng pipe cha yêu cầu Uvicorn dừng. Token và khóa provider không
được đưa vào command line hoặc log. Dùng lifespan để quản lý HTTP client.

API `GET /health/live`, `GET /health/ready`, `POST /v1/gui/resolve` và
`POST /v1/gui/recover` dùng bearer token và protocolVersion 1. Health xác nhận
service/provider được cấu hình; không thay thế nghiệm thu model trên ảnh thật.

Provider chỉ chọn node có trong quan sát. Client kiểm request/observation/epoch,
tính duy nhất, bounds và trạng thái node rồi đọc lại trước khi dùng kết quả.
Response thất bại vẫn giữ usage nếu provider đã trả dữ liệu tính phí.

## Đóng gói

Từ gốc repo chạy `python scripts/build_gui_service.py`, sau đó
`python scripts/test_gui_service_package.py target/gui-service/riviu-gui-service`.
Build tạo `target/tauri-gui-service.conf.json` và manifest hash toàn bộ runtime.
Binary chạy độc lập với Python cài trên PC người dùng; không dùng runtime iOS.

## Bảo trì

Sửa provider trong `riviu_gui/provider.py`, hợp đồng HTTP trong `models.py`,
lifecycle/routing trong `app.py`. Mẫu serialization của Rust và Python phải cùng
được kiểm thử khi đổi hợp đồng. Bằng chứng và SQLite ngân sách thuộc Rust.

Gói tương thích là dữ liệu JSON gồm target và fixture. Replay qua example
`gui_replay` không kết nối thiết bị. Mỗi target trong gói nhập phải có ít nhất
một fixture duy nhất và một fixture thiếu/mơ hồ; sửa gói không thay effect gate.
