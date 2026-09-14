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

## Tìm mẫu ảnh cục bộ

`POST /v1/gui/template-match` dùng OpenCV headless trong runtime, không cần khóa
provider và không gửi ảnh ra mạng. `/health/ready` công bố `localTemplateMatch`
kể cả khi `providerReady` là false. API chỉ trả vùng ảnh, không thực hiện thao tác.

Request JSON dùng protocolVersion 1, requestId, observationId, sessionEpoch,
generation và remainingMs (1–30000). `screenshot` và `template` cùng có
`bytesBase64`, `sha256`, `width`, `height`; chỉ nhận PNG/JPEG tĩnh và màu đục.
`roi` tùy chọn là `{x,y,width,height}` theo pixel nguyên trong ảnh chụp;
`scales` mặc định `[1.0]`, tối đa chín giá trị khác nhau từ 0.5 đến 2;
`threshold` mặc định 0.9, giới hạn 0.5–1. Không nhận đường dẫn hoặc URL ảnh.

Response lặp lại toàn bộ định danh quan sát và hai hash ảnh, kèm `status`,
`reason`, `elapsedMs`, `searchedScales` và tối đa tám `candidates`. Mỗi ứng viên
gồm `bounds`, `score`, `scale`, `method: "templateCorrelation"`.
Một vị trí vượt ngưỡng trả `resolved`; nhiều vị trí trả `ambiguous`; mẫu thiếu,
quá ít chi tiết hoặc lớn hơn vùng tìm trả `unresolved`. Điểm là hệ số tương quan
chuẩn hóa theo mức xám, không phải xác suất đúng và không kiểm màu sắc, nội dung
văn bản hay danh tính tài khoản. Mẫu cố định không tự thích nghi với giao diện
đổi hoàn toàn. Rust phải kiểm observation/epoch/hash và đọc lại trước khi dùng.

Giới hạn tải: screenshot 8 MiB / 8 MiPixel, template 2 MiB / 1 MiPixel;
tổng vị trí so khớp qua các scale tối đa 32 Mi. Thu hẹp ROI hoặc tập scale khi
vượt giới hạn. Hai request nhận diện dùng chung hai slot; mỗi phép OpenCV dùng
một luồng CPU, chạy ngoài event loop. Hết hạn trả HTTP 504 và yêu cầu worker
dừng; slot chỉ được trả khi worker thực sự kết thúc. Worker kiểm hạn giữa các
phép native và peak; một lời gọi native đang chạy hoàn tất trước khi nhả slot.
HTTP 429 có Retry-After khi đủ slot, HTTP 422 cho đầu vào sai.

Thuật toán dùng [matchTemplate của OpenCV](https://docs.opencv.org/4.x/d4/dc6/tutorial_py_template_matching.html)
với TM_CCOEFF_NORMED và loại các đỉnh trùng vị trí giữa các tỷ lệ.

## Đọc chữ cục bộ trong Flow

`POST /v1/gui/ocr` dùng Tesseract qua `tesserocr==2.10.0`; Windows đóng gói
Tesseract 5.5.2, các nền tảng khác dùng native library trong wheel đã khóa.
Wheel macOS hiện yêu cầu macOS 15 trở lên cho OCR; Windows x64 đã có package gate.
`tessdata_fast` có hai bộ `vie` và `eng`, ghim commit và SHA-256 trong
`riviu_gui/ocr-models.json`. Trước khi chạy dev hãy gọi
`python sidecars/gui-service/stage_ocr_models.py` từ gốc repo. Build gọi bước này
tự động; khi chạy ứng dụng không tải model hay gửi ảnh qua mạng.

OCR nhận cùng định danh quan sát với template matching, một `screenshot`,
`roi` pixel tùy chọn, `languages: ["vi", "en"]` và `minConfidence` từ 0 đến 1.
Ngân sách tối đa 8 MiB/8 MiPixel, 30 giây, 128 dòng/4096 byte UTF-8 đầu ra;
hai yêu cầu chia sẻ slot với các phương thức nhận diện khác. Mỗi lượt chạy trong
tiến trình riêng, một luồng native; timeout/hủy dừng và thu hồi tiến trình trước
khi trả slot. Thiếu model trả HTTP 503 và capability `ocrModelsMissing`.
Sẵn sàng có `localOcr`, `ocr.vi`, `ocr.en` dù chưa cấu hình AI provider.

Kết quả gồm văn bản, từng dòng có bounds pixel trong **ảnh gốc**, confidence,
engine và elapsedMs. Không tìm thấy chữ đạt ngưỡng trả `unresolved` với chuỗi
rỗng; ảnh sai/hash sai không được coi là kết quả rỗng. Điểm của engine không
phải xác suất chính xác. Chữ nhỏ/mờ, nghiêng hay nền phức tạp vẫn cần kiểm tra.

Nút **Đọc chữ từ ảnh (OCR)** trong Flow lưu văn bản vào biến để bước sau dùng.
Inspector cho chọn hai ngôn ngữ, ngưỡng và vùng tỷ lệ; **Chụp ảnh kiểm tra OCR**
chạy đúng cấu hình trên frame đã chụp. Runtime dùng frame mới và kiểm lại
request/observation/epoch/generation/hash trước khi lưu biến.

Fixture tiếng Việt/Anh kiểm dấu `ệ`, ROI/bounds toàn ảnh, ảnh trắng, model thiếu
hoặc sai hash, hủy và timeout. Package gate gọi OCR thực trên PATH sạch.
Nguồn: [API Tesserocr](https://github.com/sirfz/tesserocr),
[model Tesseract](https://github.com/tesseract-ocr/tessdata_fast).

## Dựng runtime

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

Hợp đồng local template nằm ở `template_models.py`, thuật toán ở
`template_match.py`. `tests/test_template_match.py` kiểm ROI, scale, thiếu/mơ hồ,
ảnh lỗi, ngân sách và giữ slot qua timeout/cancel. Package smoke test gọi phép
so khớp thật trên PATH sạch để xác nhận NumPy/OpenCV được đóng gói đủ.
