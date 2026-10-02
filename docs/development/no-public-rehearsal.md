# No-public rehearsal — diagnostic debug, chưa nghiệm thu live

Đường nghiệm thu này không phát hành bài hoặc tương tác công khai. Không dùng
`live_interaction`/campaign canary rồi cancel để giả dry-run.

## Ranh giới

- Publish dừng **trước Post intent và tap Post**. Xác minh bài đăng là bước sau
  Post, nên không thuộc lượt này. Không trả SubmittedAt hoặc publication success.
- Interaction chỉ root draft: chứng minh tài khoản/target, nhập/readback/clear chữ
  thuộc lượt chạy. Không Send, Like, Save, Follow hoặc Share-to-recipient. Reply
  chưa qualified. Copy link chỉ là navigation phục vụ canonical proof.
- Không AI trả phí, Sheets write, dispatcher/outbox hoặc dữ liệu vận hành.
- Worktree có thể build/review song song; điện thoại chỉ qua một AppState/controller,
  một lease mỗi serial. Không mở diagnostic cạnh production controller.

## Khởi động và IPC

Debug-only, process-pinned activation: `RIVIU_NO_PUBLIC_REHEARSAL=1`, scope absolute
qua `RIVIU_NO_PUBLIC_SCOPE`, directory **mới** qua `RIVIU_NO_PUBLIC_DIR`. Không trộn
mock/UI smoke hoặc override WebView. Scope ghim serial, package, account, target/
text hoặc source/bundle/fingerprint/sound policy. Discovery cho phép account trống
chỉ khi không có input; kết quả discovery không tự cấp quyền chuẩn bị. Activation
chuẩn bị phải được tạo riêng từ tài khoản đã quan sát. `allowWarmLaunch` mặc định
false; khi scope duyệt, chỉ dispatch một launcher intent của đúng package rồi đọc
foreground, không force-stop/Back/unlock/grant. `helperCanary` cũng mặc định false;
không suy quyền clipboard SET từ READ canary hoặc metadata. `allowInstalledRunner` default
false; khi exact serial được duyệt, chỉ khởi UiAutomator APK đã cài nếu không có
instrumentation/server khác, không install/grant/restart. Legacy setup reconciliation
chỉ nhận exact request/provenance đã review và được operator duyệt riêng; không dùng
phase hoặc chuỗi lỗi để xóa fence chung.

Scratch SQLite/credential RAM, worker nền không khởi chạy. Ingress command và
plugin ACL deny ngoài allowlist. Handler typed `no_public_inspect`,
`no_public_prepare_interaction`, `no_public_prepare_publish`, status/cancel không
đi qua production campaign commands.

Registry sở hữu task, deadline/cancel, duplicate-request status-only và durable
receipt trước effect. Fence device ổn định ngoài activation chặn chạy lại sau
partial/unknown/restart. Không xóa fence để retry. Semantic cleanup không rõ hoặc
persist lỗi giữ `needsAttention` và quarantine lease; đóng stream không chứng minh
đã xóa draft/media.

## Transport discovery strict

- Không install/upgrade APK, grant permission, staging minicap, helper-token handoff,
  enable/switch IME, unlock hoặc Back/relaunch để sửa foreground.
- Scope strict chỉ app package đã ở foreground; warm launch cần flag duyệt riêng.
  Screen phải được chứng minh awake/unlocked, existing healthy UiAutomator server,
  payload minicap hash đúng. Thiếu/unknown → blocked.
- Không eager-attach helper cho discovery. Borrowed helper không force-stop khi
  detach; authenticated capability phải qua authenticated read, không chỉ `/status`.
- Agent forward diagnostic `--no-rebind`; minicap dùng socket UUID riêng và allocate
  `tcp:0`, không prune forward của phiên khác. Teardown không DELETE/force-stop
  borrowed UiAutomator.
- `POST /session` và observation settings vẫn tạo session trên server đã có; không
  phải read-only attach. Controller khác cùng device phải drain/nhả ownership;
  discovery pin legacy observation, không install hoặc đổi policy thiết bị.
- Account discovery không nhập chữ hay mở Publish; yêu cầu positive safe-screen
  trước Profile navigation, không discard draft cũ.

Full Interaction cần authenticated clipboard SET/GET và temporary IME restoration;
Android Publish dùng native-media ADB route, không helper HTTP import. Strict discovery không tự cấp các
quyền đó. Khi chưa được qualified/cho phép, không thay token bằng secret cũ, không
nới target proof hoặc chạy setup dưới nhãn read-only.

## Publish primitive

Candidate preparation tuples: Trill `38.3.2/en` và Global `45.7.3/en`, 1–10 ảnh,
không video. Resource IDs của sound navigation keyed riêng từng tuple, không chứng
nhận live chỉ vì metadata nhận diện được. Safe feed/account trước transfer, actual source hash, import identity/MediaStore refs, fresh account
reproof và production picker/sound/caption/final Post readback. Typed callback từ
chối trước Post. Chỉ cleanup owned composer/import với readback exact; lost ACK/
foreign refs/unknown giữ attention, không replay.

## Công cụ local

- `tools/rehearsal_inventory.py`: host:devices-l từ ADB server đã chạy, không start/
  kill server. Roster không chứng minh session/account/composer.
- `tools/no_public_process.py`, `no_public_close.py`: identity PID/path/start time/
  HWND; close chỉ WM_CLOSE với operator idle authorization, không force-kill.
- `tools/no_public_frontend.py`: Vite-only từ dependency hiện có, không controller.
- `tools/no_public_launch.py`: fresh discovery activation, từ chối app còn chạy.
- `tools/no_public_harness.mjs`: narrow loopback CDP → typed IPC, kiểm activation,
  scope và publicEffectsAllowed=false trước request; report local dưới target.
  `--mode policy-check` gửi command bị cấm **không có argument** và đòi đúng
  ingress denial, không coi argument validation là proof. `--wait true` chỉ poll
  receipt sau một start, không replay preparation.
- `tools/no_public_fixture.py`: local ảnh/caption “không đăng công khai”; không gửi
  model/cloud. Không chạy lại trên directory đã có.

## Cổng và cách báo

Chạy `cargo test --offline --locked -p riviu-core rehearsal --lib` cho root draft,
discovery và production media/sound/caption journey từ chối Post; desktop
`no_public` tests cho ingress/registry/media receipts; Android helper/borrowed
transport teardown tests; semantic quarantine retains-lease test và build debug
trên source tích hợp cuối. Giữ output thực trong report local, không suy kết quả
worker thay cổng tích hợp. Unit/fixture không chứng nhận phone hoặc installer.

Báo riêng: inspected, prepared-before-effect, cleanup-verified, blocked và
needsAttention; kèm tuple/run/receipt. Không gọi rehearsal là publication/comment/
Sheet success, không suy fleet PASS từ một canary. Nhánh này chưa được đưa vào main
hoặc installer.
