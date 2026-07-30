# Unified Agent Runtime - live verification 2026-07-28

## Kết quả

Runtime hợp nhất đã chạy end-to-end trên iPhone 8, iOS 16.7.15, UDID
`<UDID>`:

- cài và launch `com.mrph.svc`;
- xác thực protected `/wda/locked` và MJPEG `:9093`;
- foreground TikTok, tạo fresh text session và gõ qua `/wda/keys`;
- gửi 2 bình luận chữ trong 4 video, cả hai được xác nhận bằng frame nút Gửi tắt;
- 0 recovery, 1 popup tự đóng, harness exit 0 sau 73 giây.

Artifact live:

| Trường | Giá trị |
|---|---|
| File | `sidecars/wda/RiviuAgent.ipa` |
| Artifact version | `2026.07.28.3` |
| Payload | `777wealth.app` |
| Bundle | `com.mrph.svc` |
| SHA-256 | `8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea` |
| Control / MJPEG | `8906` / `9093` |
| Provisioning profile | Beijing `chuvendor`, hết hạn `2027-07-24` |

Đây là asset release công khai cập nhật ngày 24/07/2026. Bản Wuhan
`csc-native-ios.app` có SHA-256
`628b4b3b36dbe2fa1e4c753d1d7b004443d00c829bf8581a28101ab499b7cb5a` đã bị
thu hồi identity: cài trả `0xe8008018`, dù profile ghi hạn 07/08/2026.

## Live gate

Lệnh chạy dùng `--steady chatty`, `--comment-prob 100`, `--like-prob 0`,
`--follow-prob 0`, 4 video. Token chỉ đi qua environment của process và không
nằm trong command log.

Artifact run:
`<TEMP>\riviu-agent-live-20260728-231343`

Summary chính:

```text
videos=4 comments=2 likes=0 follows=0 popups=1 recoveries=0 elapsed=73s exit=0
keys: n=2 p50=525ms max=525ms
fresh session: 8ms
```

Gate cuối sau khi khóa payload/signer và thêm process-tree ownership:
`<TEMP>\riviu-agent-final-live-20260728-235611`.
Harness đọc token từ keyring, xử lý 4 video, gửi 1 bình luận chữ đã xác nhận, chạy
recovery fresh-session có giới hạn một lần và exit 0 sau 138 giây. Kết thúc phiên
không còn harness/proxy/stream/relay nào trên máy.

Có một timeout probe `/wda/locked` 2016 ms trong quá trình dựng agent; retry có
giới hạn thành công và không kích hoạt recovery. Hai log gửi đều là
`đã gửi bình luận chữ (xác nhận nút gửi tắt)`.

Lần live đầu ở
`<TEMP>\riviu-agent-live-20260728-224124` phát hiện
trạng thái cài agent cũ không nhất quán và DVT launch trả Security. Cài lại đúng
asset release hiện hành giải quyết lỗi; syslog sau đó xác nhận process
`WebDriverAgentRunner-Runner` chạy, rồi bootstrap protected auth + MJPEG đạt.

## Desktop đã cài

- NSIS:
  `<WORKSPACE>\target\release\bundle\nsis\Riviumanagersphone_0.1.0_x64-setup.exe`
- MSI:
  `<WORKSPACE>\target\release\bundle\msi\Riviumanagersphone_0.1.0_x64_en-US.msi`
- Executable đã cài:
  `<LOCAL_APP_DATA>\Riviumanagersphone\riviu-managers-phone.exe`
- IPA đã cài kèm desktop:
  `<LOCAL_APP_DATA>\Riviumanagersphone\sidecars\wda\RiviuAgent.ipa`

NSIS cài exit 0. IPA trong thư mục cài, IPA trong repo và checksum manifest khớp
nhau. Desktop được bootstrap token đúng một lần, lưu ở Windows Credential Manager
target `agent-auth-token.riviu-managers-phone`; lần launch tiếp theo không có
`RIVIU_RTMMO_TOKEN` trong process vẫn chạy ổn. Source scan và SQLite scan đều có
0 token pattern.

Inventory cài đặt hiện bắt buộc khớp thêm `Path` kết thúc bằng `777wealth.app` và
signer `iPhone Distribution: Beijing Hfvast Technology Co. ,ltd.`. Điều này chặn
bản Wuhan dù nó dùng cùng bundle/version/build. Env token không rỗng có thể thay
keyring cũ để phục hồi; không có env thì desktop và harness chỉ đọc keyring.

Windows process audit đã force-stop cả release harness lẫn executable trong thư
mục cài. Ở cả hai lần, kill-on-close Job Object dọn đủ proxy, MJPEG stream,
`tidevice relay`, relay Python child và WebView descendants trong 3 giây. Một lần
desktop chạy chỉ có đúng một proxy, một stream và một relay cho UDID test; watch
ổn định 120 giây không vượt quá một process cho bất kỳ vai trò nào.

## Verification

- `cargo test --workspace -- --test-threads=1`: 148 test pass.
- Python unittest: 26 test pass; discovery thấy đúng một iPhone USB và hai frame
  smoke cuối có 31237 / 31237 byte JPEG.
- Frontend: 4 test pass; `npm run build` pass.
- `npm run lint`: exit 0; còn 3 warning Fast Refresh có sẵn trong `Icons.tsx` và
  `SelectionStrip.tsx`.
- `rustfmt --check` trên 28 file Rust thay đổi: pass.
- `git diff --check`: pass.
- Checksum test của bundled IPA: pass.

`cargo fmt --all -- --check` vẫn exit 1 do format debt có sẵn ở file ngoài phạm
vi thay đổi như `build.rs`, `events.rs`, `human_behavior.rs` và `job_queue.rs`.
Không format hàng loạt các file đó trong lần sửa này; toàn bộ file Rust được thay
đổi bởi runtime đã qua scoped check.

## Vận hành tiếp

Build RT-MMO hiện chỉ chấp nhận token cố định của artifact; `FARM_KEY` tuỳ ý bị
protected endpoint từ chối. Desktop không sinh token ngẫu nhiên nữa: máy mới phải
migrate token artifact một lần vào native credential store, sau đó chỉ đọc keyring.

Phải thay artifact và cập nhật manifest/checksum trước 24/07/2027, hoặc sớm hơn
nếu enterprise identity bị thu hồi. Máy này có sẵn pipeline ký qua Apple ID trong
`<SIGNING_WORKSPACE>\backend\assets\ios\apple_signer.py` và
zsign WASM, nhưng cần Apple ID/2FA, UDID đã đăng ký và profile phù hợp. Profile
tài khoản miễn phí vẫn chịu hạn 7 ngày và giới hạn thiết bị.

Runtime hiện được live chứng minh trên iOS 16.7.15. iPhone/iOS mới hơn đi qua
capability negotiation và transport adapter đã dành trong thiết kế; mỗi dải iOS
vẫn phải qua live gate trước khi đưa vào fleet. MDM/supervision, erase, OS update
policy và Activation Lock escrow thuộc phase 3, chưa nằm trong milestone 1-2 này.
