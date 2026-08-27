## 4. Chạy và test

```bash
export PATH="$HOME/Library/Python/3.9/bin:$PATH"

cargo test --workspace                     # toàn bộ Rust workspace
cargo build -p riviu-managers-phone --bin live_nurture_test --release

# dọn trước khi test thật
tidevice -u <UDID> kill notes.3u
tidevice -u <UDID> kill com.riviu.managersphone.agent.xctrunner
tidevice -u <UDID> launch com.ss.iphone.ugc.Ame

RIVIU_AI_API_KEY=<key> \
RIVIU_WDA_BACKEND=rt-mmo \
RIVIU_RTMMO_TOKEN=<token> \
RIVIU_RTMMO_IPA=<path-to-current-RiviuAgent.ipa> \
RIVIU_FRAME_DUMP=/tmp/riviu-frames/run \
RIVIU_WDA_TRACE=/tmp/riviu-live/trace.jsonl \
./target/release/live_nurture_test --udid <UDID> \
  --minutes 15 --videos 200 --like-prob 35 --comment-prob 20 --follow-prob 6 \
  --watch-min 4 --watch-max 12 --jsonl /tmp/riviu-live/summary.jsonl
```

**Không chạy harness cùng lúc với desktop app** — hai process tranh USB.

### Biến môi trường

| Biến | Tác dụng |
|---|---|
| `RIVIU_AI_API_KEY` | Key cho API bình luận. **Không bao giờ hard-code vào repo.** |
| `RIVIU_WDA_BACKEND` | Chỉ đọc ở harness. Desktop bỏ qua biến này và luôn dùng Unified Agent; `stock` chỉ là rollback/debug tường minh. |
| `RIVIU_RTMMO_TOKEN` | Desktop chỉ nhập một lần vào OS credential store; harness đọc ở biên binary. **Không hard-code hoặc đọc biến này trong driver library.** |
| `RIVIU_RTMMO_IPA` | Chỉ là override cho harness. Desktop dùng `sidecars/wda/agent-manifest.json` + `RiviuAgent.ipa` và bắt buộc khớp SHA-256. |
| `RIVIU_FRAME_DUMP` | Thư mục dump frame mỗi khi phân loại màn hình đổi — công cụ chính để hiệu chỉnh detector |
| `RIVIU_WDA_TRACE` | JSONL mọi request WDA (endpoint, ms, outcome) |
| `RIVIU_PROXY_LOG` | stderr của `wda-proxy`, dùng khi relay chết bất thường |
| `RIVIU_SIDECAR_ROOT` | Trỏ tới thư mục `sidecars/` khác |
| `RIVIU_MOCK_DEVICES=1` | Driver giả, không cần máy thật |

### Exit code của harness

`0` đạt · `1` sai tham số/thiết lập · `2` không đạt (0 video, kết thúc
`partial`/`failed`, hoặc >1 lần recovery nặng).

**`--like-prob` / `--comment-prob` / `--follow-prob` là số nguyên phần trăm**
(`30`, không phải `0.30`). Truyền `0.30` sẽ bị cắt thành `0` và chạy im lặng
với 0% — header in ra `like=0%`, đọc header để phát hiện.

### 4.0 Provisioning fleet — GIỚI HẠN TÀI KHOẢN (đo 2026-07-27, 20 máy)

Agent WDA ký bằng **tài khoản Apple Developer FREE** (`cattfan239@gmail.com`,
team `VJQ9MM29VH`). Đo thực tế trên fleet 20 iPhone 8:

- **Cert sống 7 ngày** (profile hết hạn sau đúng 1 tuần → phải ký + cài lại hàng tuần).
- **Tối đa ~3 thiết bị đăng ký** cho cả năm, **không reset được**. Thêm máy thứ 4
  báo `Your development team has reached the maximum number of registered iPhone devices`.
- Mỗi máy mới phải **tin cậy cert thủ công** trên điện thoại (Cài đặt → Cài đặt
  chung → VPN & Quản lý thiết bị → tin cậy). Không có đường lập trình. Cài xong mà
  chưa tin cậy thì launch báo `FBSOpenApplicationErrorDomain … Security … Unable to launch`.

**Tự động hoá được (đã chứng minh)**: đăng ký UDID + build + ký + cài, bằng
`xcodebuild build-for-testing … -allowProvisioningUpdates -allowProvisioningDeviceRegistration
DEVELOPMENT_TEAM=VJQ9MM29VH CODE_SIGN_STYLE=Automatic
PRODUCT_BUNDLE_IDENTIFIER=com.riviu.managersphone.agent` rồi đóng gói `.app`→`.ipa`
→ `tidevice install`. Script mẫu: `/tmp/batch_install.sh`.

**Chặn cứng**: giới hạn 3 máy của tài khoản free. Muốn chạy fleet >3 máy **phải
dùng tài khoản trả phí** ($99/năm → 100 thiết bị, cert 1 năm). Khi có tài khoản
trả phí: đăng ký toàn bộ UDID một lần, ký một lần, script cài như trên chạy cho
cả fleet; chỉ còn bước tin cậy thủ công mỗi máy.

#### GIỚI HẠN CONCURRENCY trên một Mac (đo 2026-07-27)

Chạy **3 phiên nurture song song** trên cùng một Mac làm **vỡ phân loại màn
hình**: mọi frame đọc thành "không có rail" / "không ở FYP", video đứng ở 0.
Một mình một máy thì hoàn hảo (95% tim). Nguyên nhân: **một usbmux không kham
nổi 3 stream MJPEG + 3 relay điều khiển cùng lúc** — frame về chậm/cũ nên các
test đặc trưng màu thất bại. Tắt bớt máy giữa chừng KHÔNG cứu được phiên đang
hỏng (stream đã kẹt); chạy lại sạch một máy thì tốt ngay.

Hệ quả cho fleet 20 máy: **không stream + chạy tất cả trên một Mac cùng lúc**.
Lựa chọn: (a) nhiều Mac/USB-hub có controller riêng, (b) hạ FPS/kích thước
frame MJPEG để giảm băng thông mỗi máy, (c) chia thời gian round-robin (quan
sát+hành động lần lượt từng máy). Chưa triển khai — là quyết định kiến trúc.
Ngưỡng an toàn hiện tại trên máy này: **1–2 máy đồng thời**.

### 4.1 Đưa một máy mới vào dùng

Đã làm thật trên `05101fdb…` (iPhone 8, iOS 16.7.15) ngày 2026-07-27.

```bash
export PATH="$HOME/Library/Python/3.9/bin:$PATH"
tidevice list                                    # máy phải hiện ConnType=usb
python3 sidecars/pymobiledevice3/riviu_pmd.py install-agent --udid <UDID>
tidevice -u <UDID> developer                     # mount DDI (thường báo already mounted)
```

**CLI `python3 -m pymobiledevice3` đang hỏng** (`ModuleNotFoundError: typer._click`)
— bản cài là pymobiledevice3 v5 API **async**. Dùng `tidevice` cho CLI, hoặc gọi
thư viện qua `asyncio` (`await create_using_usbmux(serial=…)`).

#### Chẩn đoán khi WDA không lên trên máy mới

Đừng đoán. Mở từng lockdown service riêng lẻ để biết cái nào hỏng:

```python
for svc in ["com.apple.instruments.remoteserver.DVTSecureSocketProxy",
            "com.apple.testmanagerd.lockdown.secure",
            "com.apple.debugserver.DVTSecureSocketProxy"]:
    await ld.start_lockdown_service(svc)     # đo thời gian + bắt exception
```

Bảng đọc kết quả:

| Triệu chứng | Kết luận | Cách xử lý |
|---|---|---|
| Cả ba đều lỗi | pairing/trust hỏng | pair lại, mở khoá máy, Trust This Computer |
| `testmanagerd` + `debugserver` OK, chỉ `instruments.remoteserver` timeout/`ConnectionTerminatedError` sau ~10s | **daemon Instruments phía máy bị kẹt** — pairing, TLS và DDI đều tốt | **reboot máy** (`tidevice -u <UDID> reboot`) |
| `com.apple.instruments.remoteserver` báo `InvalidService` | bình thường trên iOS ≥ 14 (chỉ bản `.DVTSecureSocketProxy` còn sống) | bỏ qua |

Triệu chứng bề mặt của ca giữa là `socket.timeout: _ssl.c:1112: The handshake
operation timed out` từ `tidevice launch` / `tidevice xctest`. Rất dễ bị chẩn
đoán nhầm thành "chưa bật Developer Mode" hoặc "chưa trust chứng chỉ" — kiểm
tra `DeveloperModeStatus` và thử `start_lockdown_service` trước khi đổ lỗi.

Sau reboot: **`tidevice info` trả lời được trong ~10s đầu là máy CHƯA tắt hẳn**.
Máy rụng khỏi USB thêm ~30s nữa. Poll bằng `tidevice list` (đếm dòng UDID), đừng
poll bằng `tidevice info`.

Xác nhận runner chạy được trước khi chạy harness:

```bash
tidevice -u <UDID> xctest -B com.riviu.managersphone.agent.xctrunner
# phải thấy: ServerURLHere->http://<ip>:8100<-ServerURLHere
```

---
