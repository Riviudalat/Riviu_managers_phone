# Kế hoạch và trạng thái — 13/08/2026

Ảnh chụp trạng thái ở commit `c11c9ac` trên `main`. Viết để bàn giao: ai đọc file này phải
biết được **cái gì chạy được**, **cái gì chưa**, và **vì sao chưa** — mà không phải đọc lại
lịch sử git.

Số đo chi tiết nằm ở `AGENTS.md` mục 9.18–9.39. File này là bản đồ, không lặp lại số đo.

---

## Đích của cả đợt

Ba thứ, do người vận hành đặt ra:

1. **Máy Windows sạch cài xong chạy được ngay** — không phải cài Android SDK, không phải đặt
   biến môi trường bằng tay.
2. **App đã cài tự biết có bản mới.**
3. **Ba đường Android chạy thật qua UI**: nuôi tài khoản, tương tác (tim + bình luận), đăng bài.

Trạng thái: (1) xong, (2) gần xong, (3) hai trong ba đường xong, đường đăng bài **bị số đo
chặn lại** — xem Phần III.

---

## Phần I — Đã xong, và nghiệm thu bằng gì

Bảng này chỉ ghi những thứ đã **chạy thật** hoặc có **gate tự động** giữ. Thứ chỉ mới viết ra
mà chưa ai gọi thì nằm ở Phần II.

> **Cách bảng này được kiểm.** Trước khi viết, từng dòng được đối chiếu lại với code bằng bốn
> agent đọc độc lập: 34 khẳng định, **29 CONFIRMED, 5 OVERSTATED**. Năm chỗ nói quá đã được sửa
> ở đây và trong doc của code, và **một trong số đó là lỗi thật** — xem "Lỗi tìm ra khi đối
> chiếu" ngay dưới bảng. Bảng này là bản đã sửa, không phải bản viết từ ký ức.

| Việc | Nghiệm thu bằng |
|---|---|
| Đóng gói `minicap.apk` + `adb.exe` + 2 DLL vào bộ cài | Launch **không** có `RIVIU_MINICAP_APK`: hai máy `● Live`, fleet `2/2`. Cùng lệnh đó trước khi đóng gói cho `● Error` trên cả hai. |
| Thứ tự tìm adb `config → env → SDK → PATH → bundled` | 7 test, viết sao cho môi trường không quyết được kết quả (xem mục 9.38 về lần vi phạm chính luật này) |
| Digest của tool đóng gói không thể trôi | `verify-android-tools` chạy trong job **quality** của CI, xanh trên runner. (Năm cổng `test -f` ở job `build` phủ manifest + apk + adb + hai DLL — **không** phủ `NOTICE-platform-tools.txt`; file đó chỉ được size+sha256 trong `verify-android-tools` bảo vệ.) |
| Interaction: bình luận thủ công | Chiến dịch `44edce27` — đăng thật lên **cả** bài `/photo/` và `/video/`, đọc lại đúng nguyên văn |
| Luật chia pool `(target_index × message_count + ordinal)` | Cả 4 dòng khớp y bảng dự đoán, **và cả hai target đều chạy** nên chiều target cũng được chứng minh |
| Interaction: reply lồng đúng cha | Chiến dịch `e851ce3f` — khớp cha theo **cả tác giả và nguyên văn**, kiểm bằng mắt trên ảnh artifact |
| Interaction: thả tim | 3 dòng `đã thả tim (nhãn đổi trạng thái)`, và **thả tim thất bại không làm mất bình luận** |
| Interaction: AI viết bình luận | Chiến dịch `9b1ddc61` — chữ AI đăng được, đọc lại khớp |
| Artifact bằng chứng có thật trên đĩa | 4 file JPEG, mở ra thấy khay bình luận với đúng câu vừa đăng, nhãn `Bình luận đầu tiên`, `1 giây` |
| Cửa arrival không còn fail open | `ArrivalRefusal::NoBaseline` từ chối **trước** khi gửi intent; nổ đúng 2 lần trong thực tế trên máy đang ở thẻ LIVE |
| Lý do nửa Android vắng mặt hiện ra được | Banner `warn` + 3 test |
| Sidecar iOS hỏng không còn báo khoẻ | `classify_sidecar_ping` xét payload bất kể exit code; 5 test |
| Mọi đường thoát đi qua một chuỗi dọn dẹp | `graceful_shutdown` gọi từ cả `Exit` và `ExitRequested`; đo được sau WM_CLOSE không còn tiến trình `riviu-pmd` nào |
| Ký artifact updater | CI log: `Finished 2 updater signatures at:` với `.msi.sig` và `.exe.sig` |
| `ProfileTab = Exact("Hồ sơ")` | Đo trên Redmi; và bẫy `Contains` xác nhận **trên cùng một màn** |

### Lỗi tìm ra khi đối chiếu, không phải khi chạy

Một agent đọc lại khẳng định *"retry không thể ghi đè trạng thái `Succeeded`"* và phát hiện nó
đúng với **vòng chuẩn bị** nhưng **sai với `fail_whole_target`**: hàm đó đóng `Failed` lên
**mọi** assignment của target, kể cả cái đã `Succeeded`. `Failed` thì retry được — nên một retry
mà pha thu bằng chứng thất bại sẽ **xoá dấu vết một bình luận đã công khai và cho lần retry sau
đăng lại nó**. Đúng nguy cơ đăng trùng mà guard `only_assignments` vừa bịt, mở lại từ một đường
khác.

Đã sửa: `fail_whole_target` nhận thêm `protected` và không bao giờ ghi đè
`Sending | Succeeded | Uncertain` — cùng ba trạng thái `retryable_assignments` loại, cùng lý do
(bấm Gửi không idempotent). Ghim bởi
`a_target_failure_never_reopens_a_comment_that_is_already_public`.

Đáng ghi vì cách nó lộ ra: **không phải test nào fail, cũng không phải lần chạy nào sai.** Nó lộ
ra vì có người đọc lại chính lời tôi tự khẳng định và hỏi "câu này đúng ở đâu, sai ở đâu".

**Một khẳng định khác cũng phải hẹp lại:** "một lỗi ở target chỉ giết target đó" **chỉ** đúng cho
lỗi của `collect_target_evidence_frames`, và **chỉ ở chế độ AI** (chế độ thủ công bỏ qua cả khối
đó). Mọi đường lỗi khác trong thân vòng lặp per-target — tra id, `prepare_interaction_assignment`,
`streaming_session`, mọi lệnh ghi DB — vẫn là `?` và vẫn kết thúc cả campaign.

**Gate hiện tại:** 872 test Rust / 0 fail, 98 test frontend / 18 file, e2e 6/6, `cargo fmt` +
`clippy -D warnings` sạch, 19 test Python. Toàn bộ CI xanh ở `c11c9ac`.

---

## Phần II — Chưa xong, nhóm theo **lý do** chưa xong

Nhóm theo lý do chứ không theo tính năng, vì lý do mới là thứ quyết định ai làm được gì.

### A. Chặn bởi một số đo mà chỉ máy thật trả lời được

| Việc | Câu hỏi chặn |
|---|---|
| ~~**M4 — caption đọc được nguyên chuỗi?**~~ | **ĐÃ ĐO — ĐẠT.** Xem mục dưới và `AGENTS.md` 9.40. |
| **Xoá bài tự động** | **Đã đo và câu trả lời là không.** Xem mục dưới. |

**M4 ĐẠT — nhờ kiểm máy thứ hai.** Lần trước tôi ghi "chưa đo được" sau khi chỉ kiểm Redmi. Tài
khoản trên Note 8 (`@user19257731814158`) có nhiều bài carousel **và đều có caption**. Số đo:
caption 39 ký tự → `probe --measure-own-post` báo **`VERBATIM`**; caption 49 ký tự nằm nguyên văn
trong cây; caption 116 ký tự **bị cắt**, kết thúc bằng một ký tự `…` (U+2026), tức prefix đọc
được ~115. Thiết kế cần **≥ 24** ⇒ **dư rất nhiều**. `Follow control: absent` xác nhận lại dấu
hiệu bài-của-mình.

Vậy **luật của người vận hành hiện thực được**: P0–P5 dựng được, caption đủ làm bằng chứng duy
nhất. Chỉ **P6** (mở sheet, tap Xoá) là không — vì nút xoá không có nhãn, không phải vì caption.

**Xoá tự động: số đo nói không dựng được từ trang bài.** Trên `com.ss.android.ugc.trill` 46.3.3:

- Trang bài của mình **không có control xoá nào có nhãn** — cụm `...` không có `content-desc`
  lẫn `text`.
- Chuỗi duy nhất chứa "xóa" trên cả trang là `Thêm hoặc xóa video này khỏi mục Yêu thích.` —
  nút **Yêu thích**, một mồi bẫy.
- Tap `...` mở sheet **`Gửi đến`** (chia sẻ). Inventory đầy đủ qua agent: **không có mục xoá**.

Nên phương án trung thực là **từ chối xoá tự động, giữ tay** — đúng phương án dự phòng kế hoạch
đã nêu, nhưng tới bằng đường ngắn hơn: không phải vì caption bị cắt, mà vì **nút xoá không có ở
đó**. Dùng toạ độ để lách là đúng thứ project này từ chối bịa (`AGENTS.md` mục 10).

**Hai lối chưa thử**, nếu vẫn muốn theo đuổi xoá tự động: long-press trong grid hồ sơ, và thứ
nằm sau `Cài đặt quyền riêng tư`.

### B. Chặn bởi phần cứng không có mặt

| Việc | Cần gì |
|---|---|
| **H6-e** — trộn iPhone + Android ở Threaded phải bị từ chối `MixedPlatformThread` | Một **iPhone** cắm vào. Gate phân hoạch theo `reports_element_bounds`, nên hai máy Android thì nhóm pixel rỗng và gate **luôn cho qua** — không có cách lách. |
| Nhánh candidate **adb đóng gói** thực sự được chạy | Một **máy Windows sạch**. Theo thiết kế, `PATH` và SDK thắng trên mọi máy dev, nên nhánh bundled không bao giờ được đi vào ở local. |
| Vòng update đầu-cuối | Hai release thật + một máy sạch để cài bản cũ rồi cập nhật. |

### C. Viết rồi nhưng **chưa ai gọi** (nên chưa tính là xong)

- **`publish_driver.rs`** — trait `PublishDriver`, `PostProof`, `DeleteFailure` và 8 test đã có,
  nhưng **không caller nào**. Module mang `allow(dead_code)` với điều kiện tháo ghi rõ trong
  chính file: *khi `publish_commands` gate campaign post-then-delete vào
  `can_remove_its_own_post`*.

  **Một chỗ tôi từng nói quá, đã sửa lại trong doc của code:** `PostProof::new` là `pub`, nên
  bất kỳ code nào trong crate cũng gọi được mà không đi qua một hiện thực `prove_own_post`.
  Thứ thật sự bảo đảm "chứng minh rồi mới xoá" là **các kiểm tra trong `new`**, không phải
  visibility. Ràng nó vào trait sẽ cần một sealed token — chưa có.
- ~~**`--measure-own-post`**~~ — đã chạy thật trên Note 8, báo `VERBATIM`. Xem Phần II A.

### D. Còn thiếu để một tính năng hoàn chỉnh

- **Auto-update chưa chạy đầu-cuối.** Đã có: khoá, ký lúc build (đã xác nhận trên CI), plugin,
  capability, command `update_check`. **Chưa có**: sinh + upload `latest.json` trong cùng một
  `gh release create`, và **UI để bấm**. Không có `latest.json` thì bản đã cài không biết hỏi ai.
- **`interaction_events` rỗng.** Bảng có sẵn, không writer, không reader. Chưa quyết: ghi ở các
  transition đã có, hoặc xoá bảng.
- **`TikTokControl::ALL` khó trôi, không phải không thể trôi.** Hai match exhaustive
  (`ordinal()`, `translated()`) buộc compile error khi thêm variant, và độ dài mảng cố định buộc
  bump số khi đã thêm vào `ALL`. Nhưng **không gì cơ học buộc variant mới phải vào `ALL`**:
  `every_control_appears_in_all` tự lấy kích thước từ `ALL` và chỉ lặp `ALL`, nên một variant có
  ordinal mà không có trong mảng vẫn qua được. Đóng hẳn thì cần một macro sinh enum và mảng cùng
  lúc. Đã ghi giới hạn này vào doc của `ALL` thay vì để nó đọc như một bảo đảm.

### E. Quyết định của người vận hành, không phải việc kỹ thuật

- **Ký số Windows.** Chưa ký. Giá cụ thể: SmartScreen hiện "Windows protected your PC" ở lần
  chạy `.exe` đầu, **và bảo vệ theo reputation có thể chặn thẳng bước cài im lặng của updater**.
- **Tag `v0.1.1`.** Tạo GitHub Release công khai. Version hiện là `0.1.0` ở cả ba file, 0 tag.
- **Review giấy phép Google platform-tools.** Hoãn có ý thức; mức phơi nhiễm ghi ở `NOTICE`.
- **Backup private key của updater.** Xem Phần IV.

---

## Phần III — Việc tiếp theo, có lệnh cụ thể

### 1. ~~Đo M4~~ — xong, `VERBATIM`

Lệnh đã chạy, giữ lại để lặp lại được (chỉ đọc, không tap gì):

```powershell
$env:RIVIU_ADB_PATH       = "C:\Users\cattfan\AppData\Local\Android\platform-tools\adb.exe"
$env:RIVIU_TIKTOK_PACKAGE = "com.ss.android.ugc.trill"
cargo build -q -p riviu-android-driver --example probe        # build TRƯỚC, xem mục 9.36
.\target\debug\examples\probe.exe ce0617164585646f0d7e --measure-own-post "<caption của bài đó>"
```

Kết quả: **nguyên văn** (mức tốt nhất trong bảng nguyên văn / ≥24 / 1–23 / 0). Nên caption
**không** phải chỗ chặn xoá tự động; chỗ chặn là nút xoá không có nhãn.

### 2. Hoàn thiện auto-update

Thứ tự: sinh `latest.json` trong collector → upload nó **trong cùng** `gh release create` (không
`edit`, không `--clobber`, để gate bất biến còn nguyên) → UI gọi `update_check` và **chỉ** hiện
nút cài khi `busyReason` là `null`.

**Watch-item đã xác nhận bằng CI log:** overlay release đổi `productName` thành
`Riviumanagersphone Full`, và artifact ký ra là `Riviumanagersphone Full_0.1.0_x64-setup.exe.sig`.
Nên URL Windows trong `latest.json` **phải** trỏ đúng tên **Full**.

### 3. Chạy gate như CI, không như máy này

```powershell
$env:ANDROID_HOME = "C:\Users\cattfan\AppData\Local\Android"
cargo test --workspace --locked -- --test-threads=1
```

Lý do ở mục 9.38: một test đọc `std::env` là một test mà **môi trường** quyết định kết quả, và
máy dev không phải môi trường CI. Việc này đã làm CI đỏ bốn lần.

---

## Phần IV — Quyết định đã chốt, và lý do

| Quyết định | Lý do, và giá phải trả |
|---|---|
| **Đóng gói minicap thay vì để ngoài repo** | Đảo quyết định cũ. Biến User là một bước tay CI không kiểm được, vô hình tới lần stream đầu, và chết cả nurture. Giá: +4,2 MB mọi bộ cài, kể cả macOS nơi minicap vô dụng. |
| **adb đóng gói xếp CUỐI, sau cả `PATH`** | Máy đã có platform-tools thì bản đó đang giữ adb server ở 5037; một client khác revision buộc `adb server version doesn't match this client; killing...`, phá session công cụ khác. |
| **Hai field `bundled_*` riêng** | Đặt thẳng vào `minicap_apk` sẽ phá **cả hai** override, vì config được ưu tiên trước env. Field người vận hành không vượt lên được thì không phải lưới an toàn, nó là chiếm quyền. |
| **`human_limits` mặc định TẮT** | Người vận hành muốn toàn quyền: số đã cấu hình phải thắng, không bị cap nội bộ và mood multiplier âm thầm ghi đè. |
| **Hai cột chết để nguyên, không lấp** | `interaction_assignments` mới là bản ghi thật; một state ở cấp target duy trì song song là nguồn sự thật thứ hai có thể lệch với cái thứ nhất. |
| **Updater có mật khẩu, không để trống** | Một GitHub secret bị lộ khi đó vẫn chưa là khoá dùng được. Giá: thư mục backup chứa cả hai, tự đủ để phục hồi **và** tự đủ để mất. |
| **Không kiểm update lúc mở app, không bao giờ tự cài** | Máy farm hay offline và không ai yêu cầu nó gọi mạng. Cài đặt thay thế binary đang chạy, mà tiến trình đó giữ WDA relay, XCTest runner và lease của các máy. |
| **Từ chối xoá tự động** | Số đo, không phải sở thích: nút xoá không định vị được bằng nhãn. Xem Phần II A. |

### Khoá updater — việc người vận hành phải làm

```
C:\Users\cattfan\Documents\riviu-updater-key\
```

Chứa private key **và** mật khẩu. Cả hai đã là GitHub secret
(`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`). Public key commit trong
`tauri.conf.json` — không mật, và **phải** commit.

> **Backup thư mục đó offline.** Mất private key = **mọi bản đã cài không bao giờ update được
> nữa**, vì pubkey nướng vào từng binary đã ship và không có đường phát hành lại bằng khoá khác.

---

## Phần V — Khoảng trống đã nêu tên, không giả vờ đầy đủ

Ghi ở đây để không ai đọc code rồi tưởng nó bao trùm hơn thực tế.

- **`busy_reason` chỉ hỏi phiên nurture.** Flow run và job run cũng gây gián đoạn y như thế. Nên
  "rảnh" nghĩa là "không có phiên nurture", **không** phải "không có gì cả".
- **`interaction_dispatch` có hình dạng lease một-chủ nhưng không ai claim.** Đừng coi dòng đó là
  bằng chứng có chủ. Nếu hai instance trên cùng data dir thành chuyện có thể xảy ra, đây là chỗ
  đặt guard — hiện chưa có.
- **`MultiplexDriver::new` cố định danh sách backend lúc construct.** Cài adb khi app đang chạy
  **không** làm Android join được; banner nói thẳng là phải khởi động lại. Bỏ được giới hạn này
  thì bỏ được luôn trạng thái "cần khởi động lại".
- **Mức bằng chứng arrival gần như luôn là `Structural`.** Nickname folds lên handle khoảng **1
  trên 3** account, nên gate việc gửi vào `Identified` sẽ từ chối gần hết bài mở tốt. Hệ quả
  trung thực: **không lần gửi nào chứng minh được nó đăng vào đúng bài đã chọn**, chỉ chứng minh
  được "một bài đã mở và nó khác bài trước đó".
- **Bản dump trang bài lấy lúc share sheet che phần trên, và chỉ trên một build.** Nhìn lại trên
  trang sạch vẫn có thể thấy nhãn khác.
- **Gate chất lượng AI loại ~50% lần thử đầu mỗi target** (`comment_context_rejected`, ngưỡng
  trên 60). Khác biệt duy nhất giữa ordinal 0 và 1 là `direction` có thêm câu "trả lời câu
  trước" — mà ở chế độ **Riêng lẻ** câu đó vô lý. Chỗ sửa là **prompt**, không phải gate.
- **API key AI nằm plaintext** trong bảng `settings`. Không do đợt này gây ra, ghi để đừng ai
  phải phát hiện lại.

---

## Phần VI — Ba bẫy của môi trường, không phải của app

Mất thời gian thật vì chúng, ghi để lần sau không mất nữa.

1. **EVKey64** (bộ gõ tiếng Việt bên thứ ba, hook bàn phím toàn cục, **không** hiện trong
   `Get-WinUserLanguageList`) ăn keystroke của SendKeys: `www`→`ww`, `@user`→`@ùe`,
   `photo`→`phồt`. Cách đúng: `Set-Clipboard` rồi `fill x y "^a{DEL}^v"`.
2. **`uiautomator dump` không đo được cây khi TikTok đang phát video** —
   `ERROR: could not get idle state`. Và nó **giết agent** (mục 9.21). Dùng đường agent:
   `probe --measure-tab-bar` dump cả màn hình và không cần idle.
3. **`mCurrentFocus=…SplashActivity` không có nghĩa là đang ở màn splash** — đó là tên activity
   chính của TikTok.

Và một bẫy của chính quy trình: **`cargo run --example probe` "treo" thực ra là đang compile.**
Build riêng trước rồi chạy binary.
