## 8. Unified Agent Runtime (28/07/2026)

> Mục này thay thế mọi kết luận cũ trong §5 nói rằng text comment phải hạ iOS,
> dùng TrollStore hoặc chỉ còn emoji. Các kết luận đó chỉ đúng với stock WDA;
> runtime sản phẩm hiện dùng agent `com.mrph.svc` đã kiểm chứng text comment thật.

- Desktop resolve `DriverConfig` một lần tại composition root. `crates/ios-driver`
  không được đọc `RIVIU_WDA_BACKEND`, `RIVIU_RTMMO_TOKEN` hoặc
  `RIVIU_RTMMO_IPA`; stock không phải fallback của desktop.
- Artifact chính là `sidecars/wda/RiviuAgent.ipa`, mô tả bởi
  `agent-manifest.json`. Luôn kiểm tra SHA-256 trước mọi lần cài. Stock
  `Riviumanagersphone.ipa` chỉ còn là rollback/debug artifact.
- Artifact RT-MMO đã chốt là release `777wealth.app` cập nhật ngày `2026-07-24`,
  SHA-256 `8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea`, profile
  enterprise Beijing `chuvendor` hết hạn `2027-07-24`. Ngày 28/07 đã live-pass
  install, launch, protected `/wda/locked` và MJPEG trên iPhone 8 iOS 16.7.15.
- Không dùng bản Wuhan `csc-native-ios.app`, SHA-256
  `628b4b3b36dbe2fa1e4c753d1d7b004443d00c829bf8581a28101ab499b7cb5a`: identity
  đã bị thu hồi và install trả `0xe8008018`, dù profile ghi hạn `2026-08-07`.
- Build hiện dùng token RT-MMO cố định: `FARM_KEY` tuỳ ý vẫn bị protected endpoint
  từ chối. Lần chạy desktop đầu phải nhận `RIVIU_RTMMO_TOKEN` đúng một lần rồi
  migrate vào OS credential account `agent-auth-token`; không sinh token ngẫu nhiên,
  không ghi token vào manifest, SQLite, frontend hoặc log. Một env token tường minh,
  không rỗng phải ghi đè keyring cũ để phục hồi máy từng lưu token sai; không có env
  thì desktop/harness đọc lại keyring.
- Token agent nằm trong credential store native của hệ điều hành, account
  `agent-auth-token`. SQLite chỉ lưu `agent.settings.v1` với `autoRepair`.
- Mỗi UDID có `AgentStatus` cache và dùng cùng slot lock với relay/session/stream.
  Generic health command `agent_preflight` vẫn phải kiểm tra metadata cài đặt,
  protected auth, session và frame MJPEG; không được khôi phục cache boolean kiểu
  "đã thấy bundle là xong". Rieng Interaction execution khong goi command nay:
  no dung non-mutating inspect + atomic foreground/session/MJPEG transition o §3.12
  de khong tao session/stream truoc fresh-text sequence.
- Metadata chỉ khớp artifact khi đồng thời đúng bundle/version/build, payload app
  `777wealth.app` và signer identity trong manifest. Bản Wuhan có cùng
  `com.mrph.svc` / `1.0` / `1` nhưng payload/signer khác nên bắt buộc repair.
- Repair dừng stream trước, xoá session, dừng relay, chỉ gỡ đúng bundle trong
  manifest, kiểm checksum rồi mới cài và dựng lại theo thứ tự session-trước-stream.
  Auto-repair chỉ chạy khi app thiếu hoặc metadata artifact lệch; lỗi protected auth,
  session hay MJPEG không được reinstall lặp. Background poll backoff 30 giây rồi
  thử dựng lại transport khi state `Error`; state `Missing` / `RepairRequired` chỉ
  tiếp tục sau lần Check/Repair tường minh.
- Ordinary unified session chỉ điều khiển màn hình và phải báo
  `supports_text_input=false`. Chỉ fresh session tạo sau khi TikTok foreground mới
  báo `true`. Nếu fresh transition lỗi, xoá trạng thái nửa chừng và phục hồi ordinary
  session + stream theo best effort trước khi trả lỗi gốc.
- Desktop expose `agent_get_settings`, `agent_save_settings`, `agent_list_statuses`,
  `agent_preflight`, `agent_repair` và `agent_bulk_repair`. Nút Agent của sản phẩm
  phải gọi nhóm lệnh này; các lệnh re-sign Apple ID/stock chỉ dành cho rollback/debug
  và không cung cấp text comment tin cậy.
- Nurture job có `commentProb > 0` phải generic-preflight toàn bộ UDID trước khi báo
  started. Interaction comment job dung atomic inspect/foreground/fresh-session/
  MJPEG path o §3.12, khong goi generic preflight. Ca hai engine phai chan neu
  driver/session khong quang ba text capability; khong tu roi ve emoji fallback.
- Hai kết quả `TextNotArmed` liên tiếp phải dựng fresh text session mới, mở lại stream
  rồi thay đồng thời session của feed và watcher. `TextNotSent` không được retry vì
  trạng thái gửi là mơ hồ.
- Milestone hiện tại chỉ hoàn thiện runtime Agent hợp nhất và text comment. Các phase
  2-6 của capability control plane vẫn chưa triển khai; MDM/full fleet policy thuộc
  phase 3 và được để lại cho kế hoạch sau.

### 9. Context-grounded comment (04/08/2026)

- Comment chữ production phải lấy bằng chứng từ **ba frame MJPEG liên tiếp** của
  cùng màn hình Feed. Frame được ghép thành contact sheet portrait, kèm crop phóng
  vùng caption; không dùng `GET /screenshot` và không lấy caption từ OCR UI riêng.
- Mỗi lần comment chạy hai lượt AI: `grounded_generate` đọc caption/visual facts và
  tạo một câu; `grounded_verify` đọc lại frame độc lập để chấm relevance,
  evidenceSupport, instructionFit và genericity. Nội dung/caption luôn thắng
  direction giọng điệu; câu đạt phải ngắn, khẩu ngữ như phản ứng vừa xem xong,
  không mang giọng báo cáo/tóm tắt. Marker kiểu `được trình bày`, `mang đến`,
  `người xem`, `chất lượng` bị coi là formal-style và phải retry/skip.
- Chỉ nhận khi overall >= 80, instructionFit >= 70, genericity <= 30 và không có
  contradiction/unsupportedClaim/uiTextConfusion. Một lần retry chỉ dành cho lỗi
  điểm mềm; API lỗi, JSON sai, frame không phải Feed hoặc bằng chứng mơ hồ đều
  `ContextSkipped` và **không** dùng pool comment chung.
- Mỗi attempt grounded, kể cả lượt bị skip trước UI, được ghi vào
  `nurture_comment_attempts`; attempt qua gate có caption preview, frame SHA-256,
  điểm kiểm chứng, token/cost và outcome (`sent`, `text_not_armed`,
  `text_uncertain`, `context_skipped`, ...). Cost row chỉ được ghi sau xác nhận
  nút Gửi đã tắt; HTTP ACK không phải bằng chứng gửi thành công.
- `generate_comment_pool` và pool fixture chỉ còn để tương thích test cũ; không
  được gọi từ production `NurtureEngine`. Thay đổi schema phải kèm migration,
  rollback test và cập nhật command `nurture_list_comment_attempts` nếu UI cần
  hiển thị lịch sử.

### 10. Interaction Campaign implementation checkpoint (04/08/2026)

- `crates/core/src/interaction.rs` hiện có parser URL TikTok video/photo trực tiếp,
  reject typed cho host/scheme/path/short-link, planner root rotation theo
  `(target_index + ordinal) % actor_count`, chain parent và hash exact text trước
  UI. Short link vẫn phải resolve qua bước identity Copy Link trước khi được
  phép chạy; parser không tự coi URL rút gọn là target hợp lệ.
- Migration `interaction-comment-threads` là **version 4** trong ledger chung,
  không tạo ledger riêng. SQLite lưu campaign/actor/target/assignment, prepared
  text, effect intent, evidence, retry/cancel projection và artifact locator.
  `Database` có create/list/get/request/prepare/state/artifact APIs; test rollback
  migration và test persistence đều phải giữ.
- Tauri đã đăng ký `interaction_parse_links`, preview/start/list/get/cancel/retry
  và `interaction_open_on_device`. React có nút `Tương tác` cạnh `Nuôi TT`, panel
  Setup/Monitor, multiline direct link, actor 2-6, message 2-6, instruction và
  max words. Run Now persist trước khi spawn worker; không có scheduler phase này.
- Worker dùng `DeviceWorkOwner::Interaction` và thứ tự session -> MJPEG -> open URL;
  từng target chuẩn bị toàn bộ text qua grounded AI rồi persist hash trước send.
  Root sender xác nhận drawer/type/Send armed/Send cleared bằng frame. Sau root,
  Vision OCR revision 3 phải thấy author + exact normalized text trên hai frame;
  reply chỉ tap nút `Reply` khi locator khớp hai frame, nếu không assignment là
  `skippedParent`/partial. Sau effect intent, lỗi send là `uncertain` và retry bị
  chặn; không báo thành công theo HTTP ACK.
- Gate live Mac 04/08: candidate mới build/sign được với
  `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`, nhưng DVT launch
  `com.riviu.managersphone.agent.xctrunner` trả deviceprocesscontrol code 2 trên
  iPhone 8 iOS 16.7.16; Gate B/C = `FAIL`, report tại
  `docs/re/riviu-agent/interaction-gate-live-e561.json`. Supplemental reuse cũng
  chỉ là `SUPPLEMENTAL_ONLY`. Vì vậy production
  `sidecars/wda/interaction-capabilities.json` vẫn giữ `qualifications: []`,
  không promote candidate và không gọi Interaction Ready trên desktop.
- Hash production text/manifest/capability sau vòng test vẫn phải khớp lần lượt
  `45b98dda18ad403b2fdeb547e239a3594506944e1235d8e99345cd7450158389`,
  `562578b1740a1e4ae13c863b28e6f72c448c3be80bfb3906b9d8342595850e73` và
  `f2e75b2c71dda557de6ec21f64f49b7ab0c8bb3bfe0bbccb5e64ab59be2c9709`.
- Verification roles, scoped diff và rollback artifact nằm ở
  `docs/verification/interaction-thread-20260804/`; lỗi trước khi ghi effect
  intent là `Failed` và có thể retry, còn lỗi sau `Sending` là `Uncertain` và
  không dispatch lại.

### 11. API comment preview (05/08/2026)

- Popup `Nuôi TT` có nút `Test API` trong `Cấu hình AI`. Nút lưu và validate đúng
  cấu hình hiện tại, lấy tối đa ba frame MJPEG của máy đã chọn, rồi gọi
  `nurture_test_api` dùng chính `prepare_grounded_comment` của production khi
  provider nhận ảnh. DeepSeek V4 public endpoint là text-only nên nhánh này OCR
  caption cục bộ và gọi `prepare_caption_comment` hai lượt text JSON; kết quả ghi
  rõ `OCR caption + text`. Lệnh chỉ trả preview comment, caption, điểm bằng
  chứng, token, cost, model/host và SHA-256 evidence; không mở composer, không tap
  Send, không ghi comment lên TikTok.
- Test phải chạy khi máy đang ở một video/photo TikTok có frame stream mới. Home,
  profile, LIVE hoặc frame không đủ bằng chứng sẽ trả lỗi/context rejection thay
  vì tạo comment chung chung. Không coi HTTP/API thành công là comment đã gửi.
- Model và Base URL vẫn là cấu hình người dùng; không hiển thị dòng gợi ý model
  cố định trong popup. Cấu hình hiện tại của DB dùng endpoint DeepSeek và model
  DeepSeek đã lưu, không được ghi đè khi build/cài lại app.
- **Đã đo 09/08/2026** — `GET api.deepseek.com/models` trả `deepseek-v4-flash` và
  `deepseek-v4-pro`; gửi content part `image_url` tới **cả hai** đều trả 400
  `unknown variant "image_url", expected "text"`. Serde chỉ liệt kê đúng một
  biến thể, tức content-part enum của endpoint **không có** case ảnh: giới hạn
  nằm ở request schema của endpoint chứ không phải khả năng của model, nên không
  model string nào đi vòng qua được. `provider_supports_vision` khoá theo host là
  vì thế. Đo lại trước khi tin — DeepSeek thêm image part thì cờ này sai âm thầm.
- Nurture (`nurture_test_api`) và chiến dịch chuỗi bình luận dùng **chung**
  `prepare_comment_for_frames`. Trước đây mỗi bên tự viết nhánh và bên chuỗi
  không viết: nó gọi `prepare_grounded_comment` vô điều kiện nên provider
  text-only làm hỏng cả campaign. Provider text-only là đường bằng chứng yếu hơn
  (OCR caption + gate `accepts_caption`), **không phải** lý do từ chối — đừng
  thêm lại cổng chặn ở `interaction_start_thread`.
- Gate text-only dùng context OCR >= 60 và relevance/evidenceSupport >= 80,
  kèm các cờ contradiction/unsupportedClaim/uiTextConfusion và formal-style như
  gate vision. Không hạ gate vision; hai mode phải hiển thị rõ nguồn evidence.
- Artifact và lệnh kiểm chứng/rollback của preview nằm ở
  `docs/verification/api-test-20260805/`.

### 12. Stream preview scaling (05/08/2026)

- `StreamHub` giữ một fleet feed cho desktop scheduler và channel riêng theo
  UDID cho `FrameSource`/popup watcher. Không quay lại kiểu mỗi watcher đọc
  broadcast toàn fleet: 100 máy sẽ tạo fan-out O(n²) và làm stream bị giật dù
  từng MJPEG vẫn còn sống.
- Desktop preview giữ latest frame theo UDID, round-robin và mã hoá base64 tối
  đa 240 frame/giây toàn fleet. Hai máy vẫn nhận tối đa 24 FPS/máy; khi tăng
  fleet, tốc độ preview tự chia đều (20 máy = 12 FPS/máy, 100 máy = 2 FPS/máy)
  và producer đã dừng sẽ rời ngân sách sau 10 giây. Đây chỉ là ngân sách UI;
  stream raw vẫn là nguồn bằng chứng cho watcher và nurture.
- `RIVIU_STREAM_CAPACITY` là cấu hình desktop cho 1..100 producer, mặc định 2
  để giữ hành vi live hiện tại. Giá trị ngoài khoảng bị bỏ qua và ghi cảnh báo,
  không được tự nâng capacity trong code gọi lệnh.
- **Đã sửa 09/08/2026**: `StreamBudgetManager` chặn cứng ở 2, nên
  `RIVIU_STREAM_CAPACITY=3` từng làm app **panic lúc khởi động** qua `.expect()`
  — trái đúng hợp đồng "giá trị ngoài khoảng bị bỏ qua" ở ngay trên. Giờ hạ về
  mặc định kèm cảnh báo (`state.rs::desktop_stream_budget`).
- Dải local WDA-control có 128 slot (`18100..18227`) để không đụng port khi
  fleet xoay vòng tới 100 UDID. Mọi relay vẫn phải nằm trong supervisor lock và
  registry fingerprint; không tạo relay thứ hai cho cùng UDID.
- Khi thêm virtualized grid hoặc focus priority, chỉ thay scheduler preview;
  không nới `snapshotMaxDepth`, không bật `autoDismissAlerts`, không đổi thứ tự
  session-trước-stream và không dùng preview event để chứng minh gesture/comment.

### 13. Standalone Riviu Agent full interaction install (05/08/2026)

- `sidecars/wda/RiviuAgent-text.ipa` build `2` là artifact candidate độc lập cho
  scope tương tác hiện tại: `stream`, `tap`, `swipe`, `clipboard`, `text`.
  Text gate TikTok đã có frame `armed`/`sent` thật và manifest SHA-256 là
  `45b98dda18ad403b2fdeb547e239a3594506944e1235d8e99345cd7450158389`.
- Build `2` đã được upgrade trên cả hai iPhone test; `is-installed` xác nhận
  `com.riviu.managersphone.agent.xctrunner`, version `1.0`, build `2`, đúng
  Apple Development signer. Desktop Full được build với
  `RIVIU_DEFAULT_AGENT_MODE=full` và khởi động không cần biến môi trường.
- Protected runtime hiện dùng `backend=riviu-agent` trên cả hai máy. Sáu mẫu
  `/status` liên tiếp trả `state=ready`, protocol `2`, có `text`; mỗi máy đã
  trả một JPEG MJPEG HTTP `200` hợp lệ qua header `X-Riviu-Token`. Không có
  process/port RT-MMO (`8906`/`9093`) trong phiên này. `RiviuAgent.ipa` và
  `agent-manifest.json` production vẫn chỉ là rollback oracle, không phải
  dependency runtime của Full.
- Bản desktop đã được đóng gói self-contained bằng PyInstaller onedir Python
  3.12.13 với closure khóa `pymobiledevice3==10.1.0` và `tidevice==0.12.11`;
  frozen `ping`, embedded tidevice, signer và signing-resource self-test đều
  PASS. Process thực tế chạy từ
  `Contents/Resources/sidecars/pymobiledevice3/runtime/riviu-pmd`, không cần
  Python/pip/tidevice trên máy người dùng. Executable hash là
  `46b711e1ddf7e133cca945a28dc9a50e4a400214e527e966b7c65ec87f901946`, tree
  hash `56774fce35dc0a20f29e052c86b5cfeda342e274e827b0a978a70a1aea15e0cf`.
- CI release phải truyền `RIVIU_DEFAULT_AGENT_MODE=full` và merge
  `apps/desktop/src-tauri/tauri.full.conf.json` trước
  `target/tauri-sidecar.conf.json`; nếu chỉ dùng config mặc định thì artifact
  sẽ trở về tên desktop cũ và mode legacy. Thư mục `target/` không commit: push
  `main` tạo artifact Windows/MSI/NSIS trong Actions, còn tag `v*` tạo Release.
- Verification record, desktop preview capture, IPA rollback và desktop
  pre-sidecar rollback nằm trong
  `docs/verification/standalone-agent-full-20260805/`.
- `RiviuAgent-text.ipa` hiện đã deep-verify chữ ký và embedded profile có đúng
  hai UDID test, `CreationDate=2026-08-03` và `ExpirationDate=2026-08-10`;
  đây là Xcode-managed/free provisioning 7 ngày. Windows desktop installer
  không có hạn này, nhưng Agent trên iPhone sẽ cần IPA ký lại sau ngày hết hạn
  hoặc khi đổi UDID. Không gọi IPA này là universal artifact cho thiết bị mới.
- Candidate v2 chưa quảng bá `pushMedia`; capability này chỉ được thêm sau khi
  có route contract và read-back test riêng theo source-reconstruction design.
  Không gọi bản candidate hiện tại là parity đầy đủ với oracle RT-MMO cho tới
  khi gate đó hoàn tất. Verification và rollback của lần cài này nằm ở
  `docs/verification/standalone-agent-full-20260805/`.

### 14. Photo carousel publish campaign (05/08/2026)

- Input publish là một thư mục một cấp: mỗi thư mục con là một carousel image,
  ảnh phải có tiền tố số liên tiếp bắt đầu từ `01`, có đúng một `caption*.txt`;
  `partners*.xlsx`, file ẩn và file không nhận diện bị bỏ qua có notice. Parser
  chỉ đọc PNG/JPG/JPEG (HEIC chưa được decoder hỗ trợ), giữ caption UTF-8 sau
  chuẩn hoá newline và tính SHA-256 từng ảnh/caption. Không tự sửa caption bị
  cắt hoặc tự thêm hashtag.
- `crates/core/src/publish.rs` tạo manifest side-effect-free; copy sang
  `artifacts/publish/<request-id>/<bundle-id>` được verify lại hash trước khi
  ghi DB. Mapping là một-một theo thứ tự bundle đã chọn và UDID đã chọn, cấm
  trùng/thiếu. Visibility hiện cố định `Public`, âm thanh TikTok mặc định,
  cleanup chỉ được phép sau bằng chứng post thành công.
- Migration 5 (`publish-campaigns`) lưu request, manifest bundle, assignment,
  dispatch lease và event revision. Tauri commands mới là
  `publish_scan_folder`, `publish_create_campaign`, `publish_list`,
  `publish_get`, `publish_prepare`, `publish_transfer`, `publish_cancel`;
  `publish_prepare` chỉ chuyển sang `ready`, không giả nhận đã đăng.
- `publish_transfer` và `push_material` không được gọi `install_app` cho media.
  Chúng giữ device lease rồi gọi sidecar `media-stage`, đẩy ảnh/caption qua
  HouseArrest/AFC vào `Documents/Riviu/Publish/<campaign-id>`, ghi manifest và
  đọc lại size + SHA-256. Candidate media route sau đó gọi protected native
  `prepare` rồi `import`: Photos tạo album `Riviu-<import-id>` theo đúng thứ tự
  ảnh và lưu asset IDs để cleanup idempotent. Lỗi stage/native import phải ghi
  `uncertain`, không để assignment kẹt ở `transferring` và không tự đăng lại.
- `sidecars/wda/riviu-agent/Contracts/media-v1.json` nay là candidate-route cho
  native `pushMedia`: patch 0006 thêm protected `POST/GET
  /riviu/media/v1/prepare`, kiểm tra campaign/schema, path containment, size và
  SHA-256 readback. `build_candidate.py --media-capable` và probe truyền cờ
  runtime một cách opt-in; production/default candidate vẫn không advertise
  feature này cho tới khi gate TikTok import, post-frame evidence và cleanup
  verification hoàn tất.
- UI Publish hiện cho chọn thư mục, subset bundle, subset phone, hiển thị mapping
  tuần tự/caption, chạy ngay hoặc lịch một lần. Assignment `imported` có nút
  `Post`; `publish_post` mở fresh TikTok session, stream MJPEG, chọn album, chọn
  đủ ảnh, nhập caption Unicode, xác nhận modal Public và chỉ ghi `succeeded`
  khi frame sau đăng thay đổi. Scheduler đến hạn chạy transfer rồi post tự động;
  lỗi sau effect intent là `uncertain`. Test đã pass: core parser/DB campaign,
  TypeScript/Vite build, Python media manifest, candidate contract.
- Bản Full arm64 đã build/cài tại `/Applications/Riviumanagersphone Full.app`,
  `codesign --verify --deep --strict` PASS. Candidate `0.5.2-media-text` (build
  `8`, source SHA
  `6055167f6cc2bab55147839bb21d028328554660568c7884d68fc93154443e03`) quảng bá
  đúng `stream/tap/swipe/clipboard/text/pushMedia`; resource sidecar frozen có
  `pymobiledevice3 10.1.0`. Live e561 đã PASS stage + native import (8 ảnh,
  1 caption) và đã chạy tới TikTok composer/post flow. iOS yêu cầu xác nhận khi
  xoá album Photos; patch 0007 chuyển cleanup sang `performChanges` async, bơm
  run loop và tự bấm nút `Xóa/Delete`, còn desktop cleanup chạy trước khi đóng
  stream và có một lần retry. Build/install đã PASS; Gate post+cleanup cuối vẫn
  chờ e561 được mở khóa lại sau reboot để chạy lại live. Production/default IPA
  vẫn giữ nguyên; không promote candidate trước khi record mới có frame post và
  cleanup `state=cleaned`.

#### 14.1 Live checkpoint 06/08/2026

- Native media permission đã có retry bằng XCTest pointer event. Patch `0011`
  fallback `wdFrame` và patch `0012` ưu tiên `UIScreen.mainScreen.bounds`, sau
  đó dùng fixture `375x667` nếu UIKit chưa trả frame. Cleanup giữ retry native
  bốn lần và fallback frame từ patch `0010`.
- Baseline lock hiện có 12 patch; output source SHA-256 là
  `f219ee8e356dc68119ee763059803934f80caaa275eda07ba8f42ea7bdb4f9a9`.
  Candidate build `0.5.7-media-text`/build `13`, IPA SHA-256
  `feeaa11cc68d9ab040e3a4326c5d4a52d0de037fb820c7406a28fa65f712708d`,
  source/contract/objective-C unit tests đều PASS và feature list gồm
  `stream/tap/swipe/clipboard/text/pushMedia`.
- Full app được build từ `apps/desktop/src-tauri` với cả hai config full và
  sidecar overlay; executable SHA-256 hiện tại là
  `663d03a2a48363115e65f345fafc2e4eea4785428ee79d9facb4059d36cd5a53` và
  `codesign --verify --deep --strict` PASS. Production
  `sidecars/wda/RiviuAgent.ipa` không bị thay thế.
- Live campaign `49496e40-9642-42fa-a44b-949edb5ecc24` và
  `723cc89d-36f4-4b72-8b33-d686ef296d3e` đã xác nhận stage/readback, nhưng
  import e561 timeout ở popup Photos nên state là `uncertain`; không gọi đây là
  `imported`. Cần trust lại IPA build 13 trên thiết bị trước khi chạy lại
  transfer/post/cleanup và ghi frame evidence.
- Test xác nhận: `cargo test -p riviu-core --lib publish` 9/9,
  `cargo check -p riviu-managers-phone` PASS (chỉ dead-code warning), Python
  `unittest discover sidecars/wda/riviu-agent/Tests` 125/125. Hai assertion
  patch-count đã đổi sang đọc số patch từ `baseline-lock.json` để không vỡ khi
  thêm patch native.

#### 14.2 Live verifier checkpoint 06/08/2026

- Candidate 0.5.7/build 13 đã chạy thật trên e561. Photos permission không tự
  đóng trước deadline; manual native tap `(187,407)` đóng được popup, sau đó
  phải bỏ qua alert `iPhone chưa được Kích hoạt`. Campaign
  `521e1510-ba54-4bdf-9e57-73384cbe2468` giữ `uncertain/media_transfer_native_failed`.
- Với quyền Photos đã được cấp, campaign
  `94389eb4-68a5-416c-816c-e47e2e0ee3b0` đạt `imported` (8 ảnh), Post flow rời
  composer và cleanup trả `state=cleaned` cho 8 asset. Frame sau Post lại hiện
  popup `Trạng thái tài khoản / Tài khoản của bạn đã bị khóa.`; record đã được
  sửa transactionally thành `uncertain/post_account_locked`, assignment có
  `effectIntent=post_carousel`, frame `/tmp/e561-post-success.png`, event
  `verification_failed` revision 7. Không gọi đây là post thành công.
- `publish_commands.rs` nay chạy Vision OCR ở frame sau Post và frame chờ tiếp
  theo, chặn cả chuỗi tiếng Việt/không dấu và English `account locked`. Desktop
  crate test 47/47, core publish 9/9, Python candidate 125/125, fmt/check PASS.
- Baseline lock có 13 patch; patch mới
  `0013-media-permission-logical-tap-fallback.patch` SHA-256
  `31567ca568c71550b130bb8054e647c83fe9453ea7a154f43c1561ea45bd1831` kéo dài
  16 lần tap native và dùng logical `(187.5,407)` nếu UIKit báo bounds vật lý
  2x. Source SHA-256 mới là
  `4c7465251a31469c5b90edfb56defa988f7f80f69b1278c3027366722304d915`.
- Candidate `0.5.8-media-text`/build `14`, IPA SHA-256
  `e86e77abe14d7190090b19e8e88c2a9b14417caac5ec18c604ab4ebb9a2e7d51`, features
  `stream/tap/swipe/clipboard/text/pushMedia`, Objective-C unit tests PASS. Build
  dùng a99 vì e561 đã rớt khỏi danh sách Xcode; gate live vẫn `PENDING_MAC_DEVICE`.
- Full app mới tại `/Applications/Riviumanagersphone Full.app`, executable SHA-256
  `d4a033b259a43debd4dd1fb02ca2b778822509834afe1184c73102958f42ba1b`,
  `codesign --verify --deep --strict` PASS. Production
  `sidecars/wda/RiviuAgent.ipa`/manifest không bị thay thế. Candidate 14 cần một
  vòng cài/trust e561 mới để xác nhận automatic Photos permission; không ghi
  PASS trước vòng đó.

#### 14.3 Gate B/C a99 checkpoint 06/08/2026

- Candidate media-only `0.5.8-media`/build `15` được build trên a99 với patch
  0013; feature set đúng contract gate là
  `stream/tap/swipe/clipboard/pushMedia`, IPA SHA-256
  `5f085ee785b77c7bd3050592212c38a5dcc438a77930dde34c0203b0ec8d3420`, manifest
  SHA-256 `083263e4101b986d23d40790fd6816deca17d877fc0acfc2e542ff01926b25bf`,
  source SHA-256 vẫn `4c7465251a31469c5b90edfb56defa988f7f80f69b1278c3027366722304d915`.
- Probe fresh report `docs/re/riviu-agent/candidate-probes-a99-20260806-media-fresh2.json`
  xác nhận `candidateFreshInstalled=true`, identity và cleanup đều pass, nhưng
  cold launch bị iOS từ chối với `Security ... profile has not been explicitly
  trusted by the user`; Gate B/C là `FAIL`. Đây là trust của profile sau
  uninstall/fresh-install, không phải HTTP/auth/manifest failure. Settings trên
  a99 đang mở popup `Nhà phát triển Không đáng tin cậy` để user xác nhận profile.
- Sau reboot, `tidevice developer -r` đã mount Developer Support. Runner text cũ
  chỉ launch được khi đã trusted; media candidate cũng báo `Test runner ready`
  khi được upgrade từ bản trusted. Không gọi supplemental reuse là Gate PASS;
  live Gate B/C chính thức vẫn chờ user trust candidate mới rồi chạy lại fresh
  probe với ngưỡng cố định.
- Các report supplemental/fresh fail đều được giữ lại và qua
  `rtmmo-re verify-redaction`; production IPA/manifest và app Full không bị
  thay đổi bởi gate probe.

#### 14.4 Human-like nurture checkpoint 06/08/2026

- Guard nhịp cũ đã được gỡ khỏi `NurtureSettings`, Tauri validation và popup.
  UI không còn mục `Nhịp an toàn`; cấu hình người dùng chỉ giữ xác suất và
  thời lượng xem. Không thêm lại các trường `risk_*`/`RiskGuard`.
- `crates/core/src/human_behavior.rs::HumanSessionPolicy` là policy nội bộ,
  luôn bật: cap rolling ngẫu nhiên theo giờ (tim/bình luận/follow), khoảng
  cách 12..35 giây, tối đa 2 bài đã tương tác trong 5 bài gần nhất, micro-rest
  7..13 video, block 20..45 phút, nghỉ Home 60..240 giây, Home ngẫu nhiên và
  cold restart rất hiếm (tối đa một lần mỗi phiên). Attempt được ghi trước
  gesture; counter thành công chỉ tăng khi frame sau xác nhận.
- Engine lấy action rail mới trên từng frame, không dùng rail cũ. `FeedCardKind`
  phân biệt video, `PhotoCarousel` (vuốt ngang 1..3 ảnh), `LivePreview` (vào
  phòng theo xác suất, dwell rồi thoát hoặc vuốt qua) và transition. Watcher
  tạm nhường `LiveRoom` khi engine đang sở hữu phòng để không tự đóng nhầm.
- Production DeepSeek text-only đi qua `FrameTextSource` của desktop, OCR
  caption rồi `prepare_caption_comment`; provider vision vẫn dùng 3-frame
  grounded path. Default **cũ** (06/08) là `https://api.deepseek.com` /
  `deepseek-v4-flash`. Từ 14/08 default là OpenRouter + Luna — xem §9.55.
  Windows adapter hiện báo thiếu Vision OCR thay vì giả nhận diện.
- Harness headless gọi preflight install/auth bằng context `Repair`, thả
  context trước khi chạy nurture, và dùng token env trực tiếp để tránh Keychain
  prompt. Trình tự live xác nhận: relay/auth -> session -> stream -> foreground.
- Verification: `cargo test -q -p riviu-core --lib` 299 pass/1 ignored,
  `cargo test -q -p riviu-core --test real_frames` 15 pass,
  `cargo test -q -p riviu-managers-phone` 49 pass, frontend `npm run build`
  PASS, `codesign --verify --deep --strict` PASS. Full executable hiện tại có
  SHA-256 `e4da1fb730ad7fcb4cf82b750c85ed05f5b3bcf743f6ab4a427c4d81ec9e53e2`.
- Installed app là `/Applications/Riviumanagersphone Full.app`; rollback copy
  được giữ tại `/Applications/Riviumanagersphone Full.app.rollback-20260806-human-v2`
  với hash baseline `335c35fcb79af920e0714b2f96d20ffeb250100ef361628f8ff798252d1ef68a`.
  Không overwrite production IPA/manifest trong `sidecars/wda/`.
- Live smoke pass trên a99 (1 phút): session create/prime pass, stream có frame,
  6 video, popup đóng 1 lần, nhận diện LIVE preview và bài ảnh, 0 recovery nặng.
  Một lượt sau gặp màn không phải FYP và kết thúc `0 video`; giữ cả hai log,
  không chuyển lượt fail thành pass. Chi tiết nằm ở
  `docs/verification/nurture-human-v2-20260806/`.
- Review default 06/08/2026: `HumanSessionPolicy` giữ một ngưỡng nghỉ cố định
  7..13 video rồi mới bốc ngưỡng tiếp theo; trước đây nó bốc lại ở từng video
  nên cadence không ổn định. `frenzy_prob` giờ được nối vào các swipe feed
  bình thường (retry sau swipe kẹt vẫn dùng tốc độ thường) và có ô chỉnh trong
  popup. Default fresh install là like `35%`, comment `0%` (comment chỉ bật sau
  khi có API key), follow `3%`, vuốt nhanh `6%`, xem `3..18s`; lịch vẫn tắt,
  nếu bật dùng chu kỳ `240 phút`/block `150 phút`. Setting đã lưu không bị
  migrate/ghi đè.
- Validation mới chặn `num_videos` > 10000, `num_rounds` > 100, watch > 120s,
  lịch ngoài `15..1440` phút hoặc block ngoài `15..360` phút; engine dùng
  `saturating_mul` cho legacy fixture. Tests sau review: core `299 pass/1
  ignored`, Tauri `49 pass`, frontend `73 pass`; Full app rebuild hash
  `e4da1fb730ad7fcb4cf82b750c85ed05f5b3bcf743f6ab4a427c4d81ec9e53e2`, harness
  hash `681ffe53517fb1244791778c177091ff8baf0d33389c9167bec309e29f6246df`,
  codesign strict PASS. Live smoke cũ vẫn là bằng chứng hành vi thiết bị; chưa
  gán nó thành pass mới cho thay đổi default.
- Touch/speed review 06/08/2026: `crates/core/src/nurture/touch.rs` giữ lịch sử
  tọa độ theo UDID và session, lượng tử hóa về lưới logical nguyên, không trả
  lại điểm đã dùng và tránh điểm gần nhau trong 96 lần gần nhất. Planner được
  dùng cho rail, LIVE, comment drawer/composer, emoji, thread reply và send;
  watcher popup vẫn giữ điểm đóng cố định để không miss hộp thoại hệ thống.
  Swipe feed dùng mixture nhanh hiếm `190..280ms`, bình thường `300..520ms`,
  chậm `520..820ms`; cờ frenzy dùng `150..240ms`, retry swipe kẹt không frenzy.
  Carousel dùng `280..420ms` nhanh hoặc `420..760ms` thường. Không gọi đây là
  bất biến vô hạn: vùng hitbox hữu hạn; planner có fallback mở rộng và fail
  closed khi toàn bộ logical screen đã cạn điểm.
- Final closure 06/08/2026: legacy nurture settings được migrate một lần với
  marker `nurture.settings.migration.v2`, DB backup và `rollback-db.sh`; candidate
  Riviu Agent mở URL bằng `/url` khi capability report không có route riêng,
  desktop inject OCR caption thật và text-only comment retry sau verifier. Live
  target-photo run `live-comment-target-open-url-v6.jsonl` PASS: 3 video, 2
  comment có frame xác nhận, 0 recovery. Không quảng bá comment khi evidence gate
  fail; stock/RT-MMO vẫn giữ fail-closed contract.

#### 14.5 Interaction/nurture stability contract (01/09/2026; xem §9.137)

- Bằng chứng chữ có một thứ tự chung cho drafter, verifier và batch:
  `transcript > caption web/source authoritative > pixels/OCR`. `--caption` của
  binary là fixture authoritative; OCR/local caption không được tự nâng hạng.
  Nurture không có URL thì không được bịa transcript, và brief rỗng phải giữ
  prompt cũ byte-identical.
- Root/reply và hierarchy/pixel đều trả lỗi theo effect phase. Chỉ lỗi đã chứng
  minh xảy ra trước tap Send mới là `BeforeEffect` và retryable; lỗi tại hoặc sau
  tap là `AfterEffect`/uncertain và không tự retry. Revision CAS giữ quyền sở hữu
  assignment xuyên prepare/send; SQL không được hạ một delivery đã settled.
- Follow/comment phải chụp và kiểm lại đúng author, card, rail/control ngay trước
  gesture. Pixel identity bắt buộc author OCR; caption OCR chỉ bổ sung. Không dùng
  lại rail cũ sau pause/model/audit, và folded-parent fact phải đi cùng evidence
  tới operator.
- `HumanSessionPolicy` reserve trước gesture: no-op đã chứng minh chưa effect thì
  cancel, còn tap/type hoặc phase mơ hồ thì commit. Refresh live settings lỗi đặt
  cả ba public-action rate về `0`, báo một lần mỗi chuỗi lỗi và phục hồi bằng lần
  đọc thành công kế tiếp; không có live source thì giữ snapshot ban đầu.
- Comment chỉ được mở UI sau khi audit write-ahead insert thành công; `attempt_id`
  là bắt buộc. Lỗi update outcome sau Send chỉ được báo lớn, không được biến thành
  một lần gửi lại.

#### 14.6 Windows clean-host và effect closure (01/09/2026; xem §9.138)

- Provider text-only dùng đúng một text batch cho cả cold-start refusal, giữ
  dedup/reuse và không ghi số frame ảnh giả. Verifier mặc định đọc `0..100`; chỉ
  dùng `0..1` khi payload có số thập phân rõ ràng. Draft/reject/error đều cộng
  verification spend đúng một lần.
- Comment chuyển `preparing -> sending` bằng callback one-shot ở lệnh cuối ngay
  trước tap Send. Huỷ campaign và nhả mọi claim `preparing` là một transaction;
  claim tới sau bị chặn, còn `sending`/`succeeded`/`uncertain` không bị hạ. Nếu
  gate thất bại sau khi đã gõ, composer phải được đóng có kiểm chứng; cleanup
  không chứng minh được thì assignment là uncertain, không tự retry.
- Pixel author được lấy từ cụm metadata thấp bên trái, không từ một ROI hàng cố
  định và không được nhận caption hai dòng làm author. Audit token comment không
  `Clone`, buộc vào SHA-256 của text đã ghi write-ahead và bị consume đúng một
  lần ngay trước lời gọi gõ bình luận.
- `riviu-deployment-check.exe --profile internal --report <path>` là cổng
  cài đặt Windows; `--device-check [serial]` chỉ đọc. JSON schema 1 dùng đúng
  `pass | warning | fail | not_applicable`; exit `0` khi profile đạt, `2` khi
  check/prerequisite fail, `3` khi checker lỗi nội bộ. Profile internal cho bộ
  cài chưa ký thành warning; production đòi installer hash và Authenticode hợp
  lệ.
- Checker chỉ gọi `adb.exe` trong bundle, migrate DB production trên thư mục tạm,
  thử set/get/delete Credential Manager tạm và luôn cleanup. MSI/NSIS đều cài
  theo current user, mang `NOTICE` và checker; collector phải cài thật, chạy
  checker rồi khởi động app ở mock smoke tới `tauriReady && frontendReady` trước
  khi gỡ cài đặt.
- Scratch DB của startup smoke do collector sở hữu. App xóa best-effort, nhưng
  collector phải xóa lại sau khi tiến trình con đã thoát hẳn vì Windows có thể
  giữ handle SQLite tới process teardown; lỗi startup vẫn là lỗi chính nếu
  cleanup cũng lỗi. NSIS `/S` có tiến trình self-delete riêng, nên collector chờ
  bounded tới khi checker và install root cùng biến mất thay vì đọc hậu điều
  kiện ngay khi launcher uninstaller trả về.
- Android được khởi tạo độc lập với iOS: thiếu/hỏng runtime iOS chỉ làm backend
  iOS degraded, không chặn Android và không tạo banner lỗi toàn cục sai. Headless
  ở đây là automation trong user session; Windows Session 0/service không thuộc
  contract. Enrollment Android vẫn cần OEM driver và một lần duyệt `Allow USB
  debugging`; không chuyển `adbkey` giữa máy.
- UI dùng notice typed `info/success/warning/error`, unknown không được hiển thị
  như `No`, tooltip trợ giúp phải dùng được bằng hover/focus/click/Escape. Danh
  sách phải tách Loading/Error/Empty/Data và cho retry; raw outcome/code chỉ nằm
  trong tooltip hoặc disclosure/chi tiết, gồm mapping `skipped: card_changed`.

#### 14.7 Nurture session shutdown và log theo số máy (02/09/2026; xem §9.139)

- Sau khi `open_for_session` trả một `OpenedDevice`, mọi nhánh `return` và `?`
  phải nằm trong cùng một result boundary. Hậu xử lý luôn dừng và join popup watcher,
  gọi `terminate_streaming_app` khi stream lease còn hợp lệ, rồi mới
  `close_ui_context`; đảo thứ tự sẽ mất đường force-stop được buộc vào đúng máy.
- Chỉ `ProcessAbsenceProof` từ driver mới cho phép ghi `đã tắt sạch TikTok`.
  Việc frame cuối không ở feed TikTok không phải process proof. Terminate hoặc
  release lỗi phải hiện trong terminal status; phiên chưa xử lý video nào là
  `failed`, phiên đã có video là `partial`.
- Log nurture nằm trong tab `Log` cạnh `Hành vi / AI / Bình luận`. Tên dòng dùng
  cùng `DeviceMeta` và quy tắc số/tên của tile: `Máy N · tên`; model chỉ còn là
  fallback sau số máy, không được đứng một mình.
- `RIVIU_MOCK_DEVICES=1` là một fleet fixture cô lập. Bootstrap không được probe
  hoặc ghép Android thật vào mock fleet; làm vậy sẽ tự mở preview và health-repair
  trên các máy USB dù operator chỉ yêu cầu headless mock.
