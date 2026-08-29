<!-- Sinh bởi scripts/build_agents_index.py. Đừng sửa tay: CI kiểm bản này -->
<!-- khớp với đầu ra của script. Thêm mục xong thì chạy lại script. -->

# Chỉ mục tài liệu agent

`AGENTS.md` ở gốc repo là **cửa vào**; nội dung thật nằm ở đây. File này là bản đồ:
cho một số mục, nó nói mục đó ở file nào.

## Cách phân giải một trích dẫn

Mã nguồn trích tài liệu này hơn **200 chỗ**, phần lớn dưới dạng `AGENTS.md §9.5`.
Những trích dẫn đó **vẫn đúng**: số mục là **định danh vĩnh viễn**, không đổi khi file
bị chia. Tra số trong hai bảng dưới để biết nó ở đâu.

**Viết trích dẫn bằng dấu `§`, không bằng chữ “mục”.** Đây không phải thẩm mỹ: trong
tiếng Việt “mục” vừa nghĩa *section* vừa nghĩa *item*, và tài liệu này dùng cả hai nghĩa
thật — “bảng resource không có mục 38.3.2” nói về một dòng trong bảng nhãn, không về một
mục ở đây. Nên một cổng khoá vào chữ đó sinh dương tính giả **vì cấu trúc**, không phải
vì tình cờ. Dấu `§` thì không nhập nhằng, và đó là thứ cổng đọc.

Trích dẫn **theo số dòng** thì không sống được qua việc chia file — nhưng chúng đã chết
trước đó rồi, và đó là điều đáng ghi. Cả **sáu** chỗ trong repo đều đã trỏ lệch 29–33
dòng trước khi ai chạm vào file:

| trích dẫn | ở đâu | nội dung thật ở | lệch |
|---|---|---|---|
| `AGENTS.md 691-692` | `screen.rs`, `nurture/mod.rs` ×2 | dòng 721–722, §3.12 | 30 |
| `AGENTS.md 968-973` | `ios-driver/src/pmd.rs` ×2 | dòng 1001–1003, §3.14 | 33 |
| `AGENTS.md 1470-1472` | `.gitattributes` | dòng 1499–1502, §3.18.1 | 29 |

Hai cái đầu vẫn rơi trong đúng mục nên còn đọc được; cái thứ ba rơi sang một đoạn nói về
chuyện khác hẳn (PyInstaller loại IPython) trong khi nó được trích để giải thích việc
chuẩn hoá CRLF. Cả sáu đã đổi sang số mục vào 27/08/2026. **Tên symbol và số mục sống
qua refactor; số dòng thì không.**

Hai cổng CI giữ điều này, cả hai trong `apps/desktop/src-tauri/src/lib.rs`:
`every_agents_section_citation_resolves` (mọi `§x` phải trỏ tới một mục thật) và
`agents_md_stays_a_door` (`AGENTS.md` phải ở lại ngắn).

## Mục tham chiếu (§1–§10)

Đọc theo thứ tự này nếu mới nhận dự án.

| § | Chủ đề | File | Dòng |
|---|---|---|---|
| §1 | Dự án này là gì | [`01-du-an-va-ten.md`](01-du-an-va-ten.md) | 51 |
| §2 | Đọc mục này TRƯỚC KHI sửa bất cứ thứ gì liên quan tới WDA | [`02-wda-doc-truoc-khi-sua.md`](02-wda-doc-truoc-khi-sua.md) | 132 |
| §3 | Kiến trúc | [`03-kien-truc.md`](03-kien-truc.md) | 1382 |
| §4 | Chạy và test | [`04-chay-va-test.md`](04-chay-va-test.md) | 138 |
| §5 | Trạng thái bình luận | [`05-trang-thai-binh-luan.md`](05-trang-thai-binh-luan.md) | 464 |
| §5 | Trạng thái bình luận | [`05-trang-thai-binh-luan.md`](05-trang-thai-binh-luan.md) | 464 |
| §6 | Cách hiệu chỉnh detector | [`06-hieu-chinh-detector.md`](06-hieu-chinh-detector.md) | 12 |
| §7 | Nguyên tắc khi sửa code này | [`07-nguyen-tac.md`](07-nguyen-tac.md) | 18 |
| §8 | Unified Agent Runtime | [`08-unified-agent-runtime.md`](08-unified-agent-runtime.md) | 436 |
| §9 | Fleet Android | [`08-unified-agent-runtime.md`](08-unified-agent-runtime.md) | 436 |
| §9 | Fleet Android | [`09-fleet-android.md`](09-fleet-android.md) | 214 |
| §10 | Mở đường cho thiết bị mới | [`08-unified-agent-runtime.md`](08-unified-agent-runtime.md) | 436 |
| §10 | Mở đường cho thiết bị mới | [`10-thiet-bi-moi.md`](10-thiet-bi-moi.md) | 64 |
| §11 | API comment preview (05/08/2026) | [`08-unified-agent-runtime.md`](08-unified-agent-runtime.md) | 436 |
| §12 | Stream preview scaling (05/08/2026) | [`08-unified-agent-runtime.md`](08-unified-agent-runtime.md) | 436 |
| §13 | Standalone Riviu Agent full interaction install (05/08/2026) | [`08-unified-agent-runtime.md`](08-unified-agent-runtime.md) | 436 |
| §14 | Photo carousel publish campaign (05/08/2026) | [`08-unified-agent-runtime.md`](08-unified-agent-runtime.md) | 436 |

**§2 là mục phải đọc trước khi sửa bất cứ thứ gì liên quan tới WDA.** Nó là mục duy nhất
trong tài liệu này mà bỏ qua có thể làm hỏng thiết bị thật.

Mục con của §3 và §8 (§3.12, §14.2, …) nằm trong cùng file với mục cha; tra ở bảng dưới
nếu một trích dẫn nêu số con.

## Nhật ký §9.x

146 mục, trong 6 file dưới `diary/`. **Thứ tự trong file là thứ tự viết, không phải thứ
tự số và cũng không phải thứ tự thời gian** — trong bản gốc §9.43 nằm giữa §9.20 và
§9.21, và §9.4 nằm sau §9.17. Đó là lý do bảng này sắp theo **số**: để tra được.

Tên file mang khoảng ngày **đã sắp**, nên hai file có thể trùng khoảng — đó là hệ quả
thật của việc các mục không được viết theo thứ tự, không phải thứ nên che.

### Số bị dùng hai lần — đọc trước khi tin một trích dẫn

9 số có nhiều hơn một mục, nên một trích dẫn `§9.43` **không đủ để xác định mục nào**:

- **§1** (2 mục): **hai mục KHÁC NHAU, khác ngày** — đây là một va chạm thật
  - [1. Tấm ghép là hình học iPhone 8 đem áp lên Android](diary/05-1308-2408.md#1-tấm-ghép-là-hình-học-iphone-8-đem-áp-lên-android)
  - [1. `AndroidDriverConfig::default()` ở **16** example, không phải 6](diary/06-2408-2708.md#1-androiddriverconfigdefault-ở-16-example-không-phải-6)
- **§2** (2 mục): **hai mục KHÁC NHAU, khác ngày** — đây là một va chạm thật
  - [2. Băm cả khung **không bao giờ** nhận ra hai khung giống nhau](diary/05-1308-2408.md#2-băm-cả-khung-không-bao-giờ-nhận-ra-hai-khung-giống-nhau)
  - [2. Type Interaction không có cổng parity — giờ có](diary/06-2408-2708.md#2-type-interaction-không-có-cổng-parity-giờ-có)
- **§3** (2 mục): **hai mục KHÁC NHAU, khác ngày** — đây là một va chạm thật
  - [3. Câu chữ được viết **trước** khi xem ảnh](diary/05-1308-2408.md#3-câu-chữ-được-viết-trước-khi-xem-ảnh)
  - [3. `flow::evidence` flake — đã sửa, không còn "chạy lại là xanh"](diary/06-2408-2708.md#3-flowevidence-flake-đã-sửa-không-còn-chạy-lại-là-xanh)
- **§4** (2 mục): **hai mục KHÁC NHAU, khác ngày** — đây là một va chạm thật
  - [4. Con số nào không ai đọc thì không kiểm được](diary/05-1308-2408.md#4-con-số-nào-không-ai-đọc-thì-không-kiểm-được)
  - [4. Hai thứ tôi **không** làm theo cách dễ](diary/06-2408-2708.md#4-hai-thứ-tôi-không-làm-theo-cách-dễ)
- **§9.43** (2 mục): **hai mục KHÁC NHAU, khác ngày** — đây là một va chạm thật
  - [9.43 Bài ảnh, chương bốn: cử chỉ quá giống người thì pager không nhận (18/08/2026)](diary/01-1008-1808.md#943-bài-ảnh-chương-bốn-cử-chỉ-quá-giống-người-thì-pager-không-nhận-18082026)
  - [9.43 Hai lối xoá còn lại: đã đo, đều đóng — và một lối khác mở ra (13/08/2026)](diary/04-1308-1608.md#943-hai-lối-xoá-còn-lại-đã-đo-đều-đóng-và-một-lối-khác-mở-ra-13082026)
- **§9.44** (2 mục): **hai mục KHÁC NHAU, khác ngày** — đây là một va chạm thật
  - [9.44 Hộp thoại không phải của TikTok, và giới hạn của việc tự khắc phục (18/08/2026)](diary/01-1008-1808.md#944-hộp-thoại-không-phải-của-tiktok-và-giới-hạn-của-việc-tự-khắc-phục-18082026)
  - [9.44 Đường Đăng bài cho máy Android đi qua, và bốn version chỉ kiểm ba (13/08/2026)](diary/04-1308-1608.md#944-đường-đăng-bài-cho-máy-android-đi-qua-và-bốn-version-chỉ-kiểm-ba-13082026)
- **§9.45** (2 mục): **hai mục KHÁC NHAU, khác ngày** — đây là một va chạm thật
  - [9.45 Bình luận chạy lần đầu — và tên lỗi chỉ sai hướng suốt 45% số lượt (19/08/2026)](diary/02-1208-1908.md#945-bình-luận-chạy-lần-đầu-và-tên-lỗi-chỉ-sai-hướng-suốt-45-số-lượt-19082026)
  - [9.45 Tag đầu tiên: job release chưa bao giờ chạy, và nó hỏng (13/08/2026)](diary/04-1308-1608.md#945-tag-đầu-tiên-job-release-chưa-bao-giờ-chạy-và-nó-hỏng-13082026)
- **§9.105** (2 mục): 1 mục “tiếp” — cùng một chủ đề viết nhiều đợt, đây là **cố ý**
  - [§9.105 — Mention thật cần phím thật; view tích luỹ; và một cổng đo bắn vào splash (24/08/2026)](diary/05-1308-2408.md#9105-mention-thật-cần-phím-thật-view-tích-luỹ-và-một-cổng-đo-bắn-vào-splash-24082026)
  - [§9.105 tiếp — bốn máy đó vẫn ở đúng bài, và ba lần tôi nói khác đều là lỗi dụng cụ](diary/05-1308-2408.md#9105-tiếp-bốn-máy-đó-vẫn-ở-đúng-bài-và-ba-lần-tôi-nói-khác-đều-là-lỗi-dụng-cụ)
- **§9.115** (7 mục): 6 mục “tiếp” — cùng một chủ đề viết nhiều đợt, đây là **cố ý**
  - [9.115 Bằng chứng lấy từ web, không lấy từ máy — và ảnh cuối là ảnh quan trọng nhất (26/08/2026)](diary/06-2408-2708.md#9115-bằng-chứng-lấy-từ-web-không-lấy-từ-máy-và-ảnh-cuối-là-ảnh-quan-trọng-nhất-26082026)
  - [§9.115 tiếp — lời thoại: có rồi, và cái prompt cũ đã ăn mất nó hai lần (26/08/2026)](diary/06-2408-2708.md#9115-tiếp-lời-thoại-có-rồi-và-cái-prompt-cũ-đã-ăn-mất-nó-hai-lần-26082026)
  - [§9.115 tiếp — video giờ được *xem*, và tấm ghép nói ra nó xem được mấy giây (26/08/2026)](diary/06-2408-2708.md#9115-tiếp-video-giờ-được-xem-và-tấm-ghép-nói-ra-nó-xem-được-mấy-giây-26082026)
  - [§9.115 tiếp — `video_gate` đã chạy máy thật, và nó lôi ra hai lỗi (26/08/2026)](diary/06-2408-2708.md#9115-tiếp-video_gate-đã-chạy-máy-thật-và-nó-lôi-ra-hai-lỗi-26082026)
  - [§9.115 tiếp — cột `context_json` giờ có người đọc (26/08/2026)](diary/06-2408-2708.md#9115-tiếp-cột-context_json-giờ-có-người-đọc-26082026)
  - [§9.115 tiếp — tính năng này đã "xong" mà **không chạy** trên bản cài (26/08/2026)](diary/06-2408-2708.md#9115-tiếp-tính-năng-này-đã-xong-mà-không-chạy-trên-bản-cài-26082026)
  - [§9.115 tiếp — dọn bốn việc treo, và cái nào cũng có cổng (26/08/2026)](diary/06-2408-2708.md#9115-tiếp-dọn-bốn-việc-treo-và-cái-nào-cũng-có-cổng-26082026)

### Bảng tra

| § | Mục | File |
|---|---|---|
| §1 ⚠️ | [1. Tấm ghép là hình học iPhone 8 đem áp lên Android](diary/05-1308-2408.md#1-tấm-ghép-là-hình-học-iphone-8-đem-áp-lên-android) | `diary/05-1308-2408.md` |
| §1 ⚠️ | [1. `AndroidDriverConfig::default()` ở **16** example, không phải 6](diary/06-2408-2708.md#1-androiddriverconfigdefault-ở-16-example-không-phải-6) | `diary/06-2408-2708.md` |
| §2 ⚠️ | [2. Băm cả khung **không bao giờ** nhận ra hai khung giống nhau](diary/05-1308-2408.md#2-băm-cả-khung-không-bao-giờ-nhận-ra-hai-khung-giống-nhau) | `diary/05-1308-2408.md` |
| §2 ⚠️ | [2. Type Interaction không có cổng parity — giờ có](diary/06-2408-2708.md#2-type-interaction-không-có-cổng-parity-giờ-có) | `diary/06-2408-2708.md` |
| §3 ⚠️ | [3. Câu chữ được viết **trước** khi xem ảnh](diary/05-1308-2408.md#3-câu-chữ-được-viết-trước-khi-xem-ảnh) | `diary/05-1308-2408.md` |
| §3 ⚠️ | [3. `flow::evidence` flake — đã sửa, không còn "chạy lại là xanh"](diary/06-2408-2708.md#3-flowevidence-flake-đã-sửa-không-còn-chạy-lại-là-xanh) | `diary/06-2408-2708.md` |
| §4 ⚠️ | [4. Con số nào không ai đọc thì không kiểm được](diary/05-1308-2408.md#4-con-số-nào-không-ai-đọc-thì-không-kiểm-được) | `diary/05-1308-2408.md` |
| §4 ⚠️ | [4. Hai thứ tôi **không** làm theo cách dễ](diary/06-2408-2708.md#4-hai-thứ-tôi-không-làm-theo-cách-dễ) | `diary/06-2408-2708.md` |
| §9.1 | [Nurture Android: cùng policy, khác cách nhìn (10/08/2026)](diary/01-1008-1808.md#91-nurture-android-cùng-policy-khác-cách-nhìn-10082026) | `diary/01-1008-1808.md` |
| §9.2 | [ĐÍNH CHÍNH (11/08/2026): clipboard KHÔNG chặn Interaction](diary/01-1008-1808.md#92-đính-chính-11082026-clipboard-không-chặn-interaction) | `diary/01-1008-1808.md` |
| §9.3 | [Hai bẫy môi trường đo được (11/08/2026) — cả hai từng trông như lỗi locator](diary/01-1008-1808.md#93-hai-bẫy-môi-trường-đo-được-11082026-cả-hai-từng-trông-như-lỗi-locator) | `diary/01-1008-1808.md` |
| §9.4 | [`platform` + `os_version` (11/08/2026) — và ba chỗ cố ý KHÔNG đổi](diary/01-1008-1808.md#94-platform-os_version-11082026-và-ba-chỗ-cố-ý-không-đổi) | `diary/01-1008-1808.md` |
| §9.5 | [Hàng comment trong drawer (11/08/2026) — nhãn nằm ở `text`, không phải `content-desc`](diary/01-1008-1808.md#95-hàng-comment-trong-drawer-11082026-nhãn-nằm-ở-text-không-phải-content-desc) | `diary/01-1008-1808.md` |
| §9.6 | [Package TikTok theo từng máy (11/08/2026)](diary/01-1008-1808.md#96-package-tiktok-theo-từng-máy-11082026) | `diary/01-1008-1808.md` |
| §9.7 | [Composer reply (11/08/2026) — bốn câu hỏi, bốn câu trả lời đo được](diary/01-1008-1808.md#97-composer-reply-11082026-bốn-câu-hỏi-bốn-câu-trả-lời-đo-được) | `diary/01-1008-1808.md` |
| §9.8 | [`tiktok_drawer` — tách dùng chung, không nhân bản](diary/01-1008-1808.md#98-tiktok_drawer-tách-dùng-chung-không-nhân-bản) | `diary/01-1008-1808.md` |
| §9.9 | [Gate actor Interaction: theo *tính chất*, không theo nền tảng](diary/01-1008-1808.md#99-gate-actor-interaction-theo-tính-chất-không-theo-nền-tảng) | `diary/01-1008-1808.md` |
| §9.10 | [MediaStore (11/08/2026) — `adb push` là đủ, không cần scan](diary/01-1008-1808.md#910-mediastore-11082026-adb-push-là-đủ-không-cần-scan) | `diary/01-1008-1808.md` |
| §9.11 | [Bottom tab bar (11/08/2026)](diary/01-1008-1808.md#911-bottom-tab-bar-11082026) | `diary/01-1008-1808.md` |
| §9.12 | [BẪY MÔI TRƯỜNG: Git Bash mangle đường dẫn `adb push`](diary/01-1008-1808.md#912-bẫy-môi-trường-git-bash-mangle-đường-dẫn-adb-push) | `diary/01-1008-1808.md` |
| §9.13 | [Resource id nút Gửi ĐÃ đổi giữa hai phiên bản app (11/08/2026)](diary/01-1008-1808.md#913-resource-id-nút-gửi-đã-đổi-giữa-hai-phiên-bản-app-11082026) | `diary/01-1008-1808.md` |
| §9.14 | [`TargetDriver`: một refactor và một lỗi nó tự gây ra (11/08/2026)](diary/01-1008-1808.md#914-targetdriver-một-refactor-và-một-lỗi-nó-tự-gây-ra-11082026) | `diary/01-1008-1808.md` |
| §9.15 | [`open_url` mở link TikTok vào HỘP THOẠI CHỌN APP, không vào TikTok (11/08/2026)](diary/01-1008-1808.md#915-open_url-mở-link-tiktok-vào-hộp-thoại-chọn-app-không-vào-tiktok-11082026) | `diary/01-1008-1808.md` |
| §9.16 | [GATE H5 PASSED — reply gắn đúng cha, hai máy thật (11/08/2026)](diary/01-1008-1808.md#916-gate-h5-passed-reply-gắn-đúng-cha-hai-máy-thật-11082026) | `diary/01-1008-1808.md` |
| §9.17 | [Link nào mở được: chỉ máy trả lời được, host thì không (11/08/2026)](diary/01-1008-1808.md#917-link-nào-mở-được-chỉ-máy-trả-lời-được-host-thì-không-11082026) | `diary/01-1008-1808.md` |
| §9.18 | [Nurture Android chạy được qua app — và hai lỗi chỉ có máy thật mới thấy (12/08/2026)](diary/01-1008-1808.md#918-nurture-android-chạy-được-qua-app-và-hai-lỗi-chỉ-có-máy-thật-mới-thấy-12082026) | `diary/01-1008-1808.md` |
| §9.19 | [Ba chỗ ở §9.18 đã sửa, kèm số đo (12/08/2026)](diary/01-1008-1808.md#919-ba-chỗ-ở-918-đã-sửa-kèm-số-đo-12082026) | `diary/01-1008-1808.md` |
| §9.20 | [Vuốt ngang bài ảnh trên Android — và ba kết luận sai phải sửa để làm được (12/08/2026)](diary/01-1008-1808.md#920-vuốt-ngang-bài-ảnh-trên-android-và-ba-kết-luận-sai-phải-sửa-để-làm-được-12082026) | `diary/01-1008-1808.md` |
| §9.21 | [Agent còn sống mà cây đã chết: `/status` không phải bằng chứng (12/08/2026)](diary/02-1208-1908.md#921-agent-còn-sống-mà-cây-đã-chết-status-không-phải-bằng-chứng-12082026) | `diary/02-1208-1908.md` |
| §9.22 | [Toàn quyền: `human_limits` mặc định TẮT (12/08/2026)](diary/02-1208-1908.md#922-toàn-quyền-human_limits-mặc-định-tắt-12082026) | `diary/02-1208-1908.md` |
| §9.23 | [Cử chỉ: đường vuốt thật thay vì một đoạn thẳng (12/08/2026)](diary/02-1208-1908.md#923-cử-chỉ-đường-vuốt-thật-thay-vì-một-đoạn-thẳng-12082026) | `diary/02-1208-1908.md` |
| §9.24 | [Vị trí chạm: cụm có lệch, không phải random đều (12/08/2026)](diary/02-1208-1908.md#924-vị-trí-chạm-cụm-có-lệch-không-phải-random-đều-12082026) | `diary/02-1208-1908.md` |
| §9.25 | [Nhịp thời gian: bỏ luật chống lặp và một cái lỗ trong histogram (12/08/2026)](diary/02-1208-1908.md#925-nhịp-thời-gian-bỏ-luật-chống-lặp-và-một-cái-lỗ-trong-histogram-12082026) | `diary/02-1208-1908.md` |
| §9.26 | [Interaction: thả tim, và bình luận thủ công (12/08/2026)](diary/02-1208-1908.md#926-interaction-thả-tim-và-bình-luận-thủ-công-12082026) | `diary/02-1208-1908.md` |
| §9.27 | [Đảo quyết định: minicap vào bộ cài, không để ngoài repo (12/08/2026)](diary/02-1208-1908.md#927-đảo-quyết-định-minicap-vào-bộ-cài-không-để-ngoài-repo-12082026) | `diary/02-1208-1908.md` |
| §9.28 | [Đóng gói xong: số đo, và bốn thứ suýt hỏng im lặng (12/08/2026)](diary/02-1208-1908.md#928-đóng-gói-xong-số-đo-và-bốn-thứ-suýt-hỏng-im-lặng-12082026) | `diary/02-1208-1908.md` |
| §9.29 | [Sidecar iOS hỏng mà app báo khoẻ: sự im lặng là HAI lỗi (12/08/2026)](diary/02-1208-1908.md#929-sidecar-ios-hỏng-mà-app-báo-khoẻ-sự-im-lặng-là-hai-lỗi-12082026) | `diary/02-1208-1908.md` |
| §9.30 | [Interaction chạy thật lần đầu qua app — và cửa arrival FAIL OPEN (13/08/2026)](diary/02-1208-1908.md#930-interaction-chạy-thật-lần-đầu-qua-app-và-cửa-arrival-fail-open-13082026) | `diary/02-1208-1908.md` |
| §9.31 | [H6-a ĐẠT sau ba lần chạy — và link chết trông giống hệt lỗi code (13/08/2026)](diary/02-1208-1908.md#931-h6-a-đạt-sau-ba-lần-chạy-và-link-chết-trông-giống-hệt-lỗi-code-13082026) | `diary/02-1208-1908.md` |
| §9.32 | [H6-b và H6-c ĐẠT trong một lần chạy (13/08/2026)](diary/02-1208-1908.md#932-h6-b-và-h6-c-đạt-trong-một-lần-chạy-13082026) | `diary/02-1208-1908.md` |
| §9.33 | [Ba lỗi làm chế độ AI không debug được (13/08/2026)](diary/05-1308-2408.md#933-ba-lỗi-làm-chế-độ-ai-không-debug-được-13082026) | `diary/05-1308-2408.md` |
| §9.34 | [H6-d ĐẠT sau khi sửa ba chỗ ở 9.33 (13/08/2026)](diary/02-1208-1908.md#934-h6-d-đạt-sau-khi-sửa-ba-chỗ-ở-933-13082026) | `diary/02-1208-1908.md` |
| §9.35 | [Tách `graceful_shutdown`, và một lo ngại bị nói quá (13/08/2026)](diary/05-1308-2408.md#935-tách-graceful_shutdown-và-một-lo-ngại-bị-nói-quá-13082026) | `diary/05-1308-2408.md` |
| §9.36 | [Đăng bài: phần không cần máy đã làm, và phép đo quyết định (13/08/2026)](diary/02-1208-1908.md#936-đăng-bài-phần-không-cần-máy-đã-làm-và-phép-đo-quyết-định-13082026) | `diary/02-1208-1908.md` |
| §9.37 | [Trang bài của mình: hai dấu hiệu dương, và KHÔNG có nút xoá nào có nhãn (13/08/2026)](diary/05-1308-2408.md#937-trang-bài-của-mình-hai-dấu-hiệu-dương-và-không-có-nút-xoá-nào-có-nhãn-13082026) | `diary/05-1308-2408.md` |
| §9.38 | [CI đỏ bốn lần vì đúng cái bẫy tôi đã tự cảnh báo (13/08/2026)](diary/05-1308-2408.md#938-ci-đỏ-bốn-lần-vì-đúng-cái-bẫy-tôi-đã-tự-cảnh-báo-13082026) | `diary/05-1308-2408.md` |
| §9.39 | [Auto-update: khoá, và hai chỗ cố ý KHÔNG tự động (13/08/2026)](diary/05-1308-2408.md#939-auto-update-khoá-và-hai-chỗ-cố-ý-không-tự-động-13082026) | `diary/05-1308-2408.md` |
| §9.40 | [M4 ĐẠT — caption đọc được nguyên văn, và ngưỡng cắt ở đâu (13/08/2026)](diary/05-1308-2408.md#940-m4-đạt-caption-đọc-được-nguyên-văn-và-ngưỡng-cắt-ở-đâu-13082026) | `diary/05-1308-2408.md` |
| §9.41 | [Updater xong đường phát hành: `latest.json`, và thứ tự lúc cài (13/08/2026)](diary/05-1308-2408.md#941-updater-xong-đường-phát-hành-latestjson-và-thứ-tự-lúc-cài-13082026) | `diary/05-1308-2408.md` |
| §9.42 | [Ngưỡng thời gian của `classification_stays_fast_enough_for_the_watcher` (13/08/2026)](diary/05-1308-2408.md#942-ngưỡng-thời-gian-của-classification_stays_fast_enough_for_the_watcher-13082026) | `diary/05-1308-2408.md` |
| §9.43 ⚠️ | [Bài ảnh, chương bốn: cử chỉ quá giống người thì pager không nhận (18/08/2026)](diary/01-1008-1808.md#943-bài-ảnh-chương-bốn-cử-chỉ-quá-giống-người-thì-pager-không-nhận-18082026) | `diary/01-1008-1808.md` |
| §9.43 ⚠️ | [Hai lối xoá còn lại: đã đo, đều đóng — và một lối khác mở ra (13/08/2026)](diary/04-1308-1608.md#943-hai-lối-xoá-còn-lại-đã-đo-đều-đóng-và-một-lối-khác-mở-ra-13082026) | `diary/04-1308-1608.md` |
| §9.44 ⚠️ | [Hộp thoại không phải của TikTok, và giới hạn của việc tự khắc phục (18/08/2026)](diary/01-1008-1808.md#944-hộp-thoại-không-phải-của-tiktok-và-giới-hạn-của-việc-tự-khắc-phục-18082026) | `diary/01-1008-1808.md` |
| §9.44 ⚠️ | [Đường Đăng bài cho máy Android đi qua, và bốn version chỉ kiểm ba (13/08/2026)](diary/04-1308-1608.md#944-đường-đăng-bài-cho-máy-android-đi-qua-và-bốn-version-chỉ-kiểm-ba-13082026) | `diary/04-1308-1608.md` |
| §9.45 ⚠️ | [Bình luận chạy lần đầu — và tên lỗi chỉ sai hướng suốt 45% số lượt (19/08/2026)](diary/02-1208-1908.md#945-bình-luận-chạy-lần-đầu-và-tên-lỗi-chỉ-sai-hướng-suốt-45-số-lượt-19082026) | `diary/02-1208-1908.md` |
| §9.45 ⚠️ | [Tag đầu tiên: job release chưa bao giờ chạy, và nó hỏng (13/08/2026)](diary/04-1308-1608.md#945-tag-đầu-tiên-job-release-chưa-bao-giờ-chạy-và-nó-hỏng-13082026) | `diary/04-1308-1608.md` |
| §9.46 | [Ngân sách 10 mili-giây trong test, và lần thứ hai cùng một loại lỗi (13/08/2026)](diary/04-1308-1608.md#946-ngân-sách-10-mili-giây-trong-test-và-lần-thứ-hai-cùng-một-loại-lỗi-13082026) | `diary/04-1308-1608.md` |
| §9.47 | [v0.1.1 đã phát hành, và chuỗi updater nghiệm thu từ ngoài (13/08/2026)](diary/04-1308-1608.md#947-v011-đã-phát-hành-và-chuỗi-updater-nghiệm-thu-từ-ngoài-13082026) | `diary/04-1308-1608.md` |
| §9.48 | [Điều khiển từ máy tính không được park stream (14/08/2026)](diary/02-1208-1908.md#948-điều-khiển-từ-máy-tính-không-được-park-stream-14082026) | `diary/02-1208-1908.md` |
| §9.49 | [GenFarmer mượt vì codec + canvas, không vì CSS (14/08/2026)](diary/02-1208-1908.md#949-genfarmer-mượt-vì-codec-canvas-không-vì-css-14082026) | `diary/02-1208-1908.md` |
| §9.50 | [Đường xem H.264 / canvas — xem ≠ bằng chứng (14/08/2026)](diary/02-1208-1908.md#950-đường-xem-h264-canvas-xem-bằng-chứng-14082026) | `diary/02-1208-1908.md` |
| §9.51 | [Riviu Agent trên Android — không phải toàn quyền (14/08/2026)](diary/02-1208-1908.md#951-riviu-agent-trên-android-không-phải-toàn-quyền-14082026) | `diary/02-1208-1908.md` |
| §9.52 | [Helper APK `com.riviu.agent` — clipboard + MediaStore, IME phải trả lại (14/08/2026)](diary/02-1208-1908.md#952-helper-apk-comriviuagent-clipboard-mediastore-ime-phải-trả-lại-14082026) | `diary/02-1208-1908.md` |
| §9.53 | [Overlay tap trượt vì map cả ô đen, không phải canvas (14/08/2026)](diary/02-1208-1908.md#953-overlay-tap-trượt-vì-map-cả-ô-đen-không-phải-canvas-14082026) | `diary/02-1208-1908.md` |
| §9.54 | [Overlay lag: restart encoder + khoá pointer + render mỗi frame (14/08/2026)](diary/02-1208-1908.md#954-overlay-lag-restart-encoder-khoá-pointer-render-mỗi-frame-14082026) | `diary/02-1208-1908.md` |
| §9.55 | [Default AI OpenRouter Luna, và scrcpy chết vì sai form codec option (14/08/2026)](diary/04-1308-1608.md#955-default-ai-openrouter-luna-và-scrcpy-chết-vì-sai-form-codec-option-14082026) | `diary/04-1308-1608.md` |
| §9.56 | [Tile đen vì máy ngủ; và ba đính chính cho hồ sơ đổi tên (14/08/2026)](diary/04-1308-1608.md#956-tile-đen-vì-máy-ngủ-và-ba-đính-chính-cho-hồ-sơ-đổi-tên-14082026) | `diary/04-1308-1608.md` |
| §9.57 | [Danh sách app trên máy: `cmd package`, và nhãn thì không có (14/08/2026)](diary/04-1308-1608.md#957-danh-sách-app-trên-máy-cmd-package-và-nhãn-thì-không-có-14082026) | `diary/04-1308-1608.md` |
| §9.58 | [Layout theo GenFarmer: tab nhóm + menu chuột phải, và cách tìm ra đúng file (14/08/2026)](diary/04-1308-1608.md#958-layout-theo-genfarmer-tab-nhóm-menu-chuột-phải-và-cách-tìm-ra-đúng-file-14082026) | `diary/04-1308-1608.md` |
| §9.59 | [Bon hanh dong thiet bi, va ba thu do duoc lat nguoc thiet ke (14/08/2026)](diary/04-1308-1608.md#959-bon-hanh-dong-thiet-bi-va-ba-thu-do-duoc-lat-nguoc-thiet-ke-14082026) | `diary/04-1308-1608.md` |
| §9.60 | [`adb forward` song lau hon app, va vi sao no lam man hinh den (14/08/2026)](diary/04-1308-1608.md#960-adb-forward-song-lau-hon-app-va-vi-sao-no-lam-man-hinh-den-14082026) | `diary/04-1308-1608.md` |
| §9.61 | [`tracing` khong co sink: mot gio chan doan bi mu (14/08/2026)](diary/04-1308-1608.md#961-tracing-khong-co-sink-mot-gio-chan-doan-bi-mu-14082026) | `diary/04-1308-1608.md` |
| §9.62 | [`dblclick` trong `driver.ps1`: hai click roi rac khong phai mot double-click (14/08/2026)](diary/04-1308-1608.md#962-dblclick-trong-driverps1-hai-click-roi-rac-khong-phai-mot-double-click-14082026) | `diary/04-1308-1608.md` |
| §9.63 | [Overlay cuoi cung co encode rieng, va con so 900 trong ke hoach cua toi la sai (15/08/2026)](diary/03-1508-2108.md#963-overlay-cuoi-cung-co-encode-rieng-va-con-so-900-trong-ke-hoach-cua-toi-la-sai-15082026) | `diary/03-1508-2108.md` |
| §9.64 | [Man den bao gom mot cai treo cua chinh dien thoai, va mot diem mu 8 phut (15/08/2026)](diary/04-1308-1608.md#964-man-den-bao-gom-mot-cai-treo-cua-chinh-dien-thoai-va-mot-diem-mu-8-phut-15082026) | `diary/04-1308-1608.md` |
| §9.65 | [Keyframe khong phai bang chung co SPS — va vi sao chan doan im lang suot 3 vong (15/08/2026)](diary/04-1308-1608.md#965-keyframe-khong-phai-bang-chung-co-sps-va-vi-sao-chan-doan-im-lang-suot-3-vong-15082026) | `diary/04-1308-1608.md` |
| §9.66 | [Vite khong chuyen tiep console cua Web Worker — ba vong chan doan bi mu vi dieu nay](diary/04-1308-1608.md#966-vite-khong-chuyen-tiep-console-cua-web-worker-ba-vong-chan-doan-bi-mu-vi-dieu-nay) | `diary/04-1308-1608.md` |
| §9.67 | [Detector stall tu restart la vong phan hoi duong — cang nhieu may cang chet (15/08/2026)](diary/04-1308-1608.md#967-detector-stall-tu-restart-la-vong-phan-hoi-duong-cang-nhieu-may-cang-chet-15082026) | `diary/04-1308-1608.md` |
| §9.68 | [`BROADCAST_CAP` phai doc nhu mot toc do, khong phai mot kich thuoc (15/08/2026)](diary/04-1308-1608.md#968-broadcast_cap-phai-doc-nhu-mot-toc-do-khong-phai-mot-kich-thuoc-15082026) | `diary/04-1308-1608.md` |
| §9.69 | [App bao nguoi van hanh cai hai APK ma no khong he ship (16/08/2026)](diary/03-1508-2108.md#969-app-bao-nguoi-van-hanh-cai-hai-apk-ma-no-khong-he-ship-16082026) | `diary/03-1508-2108.md` |
| §9.70 | [Overlay quyet dinh ca cu keo tu DUNG HAI DIEM (16/08/2026)](diary/03-1508-2108.md#970-overlay-quyet-dinh-ca-cu-keo-tu-dung-hai-diem-16082026) | `diary/03-1508-2108.md` |
| §9.71 | [Bat `control=true` lam mat video ca 20 may — va no chan IM LANG (16/08/2026)](diary/03-1508-2108.md#971-bat-controltrue-lam-mat-video-ca-20-may-va-no-chan-im-lang-16082026) | `diary/03-1508-2108.md` |
| §9.72 | [Tran dong thoi cho recovery: da do, va phep do BAC BO ly do ban dau (16/08/2026)](diary/04-1308-1608.md#972-tran-dong-thoi-cho-recovery-da-do-va-phep-do-bac-bo-ly-do-ban-dau-16082026) | `diary/04-1308-1608.md` |
| §9.73 | [Mot kenh moi may — va cai bay chi mot test socket that moi thay (16/08/2026)](diary/04-1308-1608.md#973-mot-kenh-moi-may-va-cai-bay-chi-mot-test-socket-that-moi-thay-16082026) | `diary/04-1308-1608.md` |
| §9.74 | [`app_process` chet o 255 byte argv — nguyen nhan that su cua §9.71 (16/08/2026)](diary/04-1308-1608.md#974-app_process-chet-o-255-byte-argv-nguyen-nhan-that-su-cua-971-16082026) | `diary/04-1308-1608.md` |
| §9.75 | [Import/Export anh-video hai chieu, va cai bay `.thumbnails` (16/08/2026)](diary/04-1308-1608.md#975-importexport-anh-video-hai-chieu-va-cai-bay-thumbnails-16082026) | `diary/04-1308-1608.md` |
| §9.76 | [Phan 3 xong: socket control, `RESET_VIDEO`, va mot ket luan tu bac bo (16/08/2026)](diary/03-1508-2108.md#976-phan-3-xong-socket-control-reset_video-va-mot-ket-luan-tu-bac-bo-16082026) | `diary/03-1508-2108.md` |
| §9.77 | [Do "khong muot" thay vi doan: CPU khong phai thu phanh, va cho no that su nam (17/08/2026)](diary/03-1508-2108.md#977-do-khong-muot-thay-vi-doan-cpu-khong-phai-thu-phanh-va-cho-no-that-su-nam-17082026) | `diary/03-1508-2108.md` |
| §9.78 | [Keo truc tiep qua socket control — va cai bay "no chay roi" (17/08/2026)](diary/03-1508-2108.md#978-keo-truc-tiep-qua-socket-control-va-cai-bay-no-chay-roi-17082026) | `diary/03-1508-2108.md` |
| §9.79 | [Duong phuc hoi agent KHONG VOI TOI DUOC, va cooldown cho no (17/08/2026)](diary/03-1508-2108.md#979-duong-phuc-hoi-agent-khong-voi-toi-duoc-va-cooldown-cho-no-17082026) | `diary/03-1508-2108.md` |
| §9.80 | [Cham cung di socket control — de agent thoi la diem chet duy nhat (17/08/2026)](diary/03-1508-2108.md#980-cham-cung-di-socket-control-de-agent-thoi-la-diem-chet-duy-nhat-17082026) | `diary/03-1508-2108.md` |
| §9.81 | [95% thoi gian mo mot view nam trong MOT dong shell (17/08/2026)](diary/03-1508-2108.md#981-95-thoi-gian-mo-mot-view-nam-trong-mot-dong-shell-17082026) | `diary/03-1508-2108.md` |
| §9.82 | [Thay nong producer: giu hinh cu toi khi hinh moi co keyframe (17/08/2026)](diary/03-1508-2108.md#982-thay-nong-producer-giu-hinh-cu-toi-khi-hinh-moi-co-keyframe-17082026) | `diary/03-1508-2108.md` |
| §9.83 | [Dang bai day MOI bundle sang MOI may — va hai backend hieu `source_root` khac nhau (17/08/2026)](diary/03-1508-2108.md#983-dang-bai-day-moi-bundle-sang-moi-may-va-hai-backend-hieu-source_root-khac-nhau-17082026) | `diary/03-1508-2108.md` |
| §9.84 | [Go han dang nhap: mat khau plaintext trong cot ten `password_hash` (17/08/2026)](diary/03-1508-2108.md#984-go-han-dang-nhap-mat-khau-plaintext-trong-cot-ten-password_hash-17082026) | `diary/03-1508-2108.md` |
| §9.85 | [Flow chay that tren Android: cai `inspect_device_for_target` con thieu (17/08/2026)](diary/03-1508-2108.md#985-flow-chay-that-tren-android-cai-inspect_device_for_target-con-thieu-17082026) | `diary/03-1508-2108.md` |
| §9.86 | [27 loi cua dot soat doi khang: nhung gi dang nho lai (17/08/2026)](diary/03-1508-2108.md#986-27-loi-cua-dot-soat-doi-khang-nhung-gi-dang-nho-lai-17082026) | `diary/03-1508-2108.md` |
| §9.87 | [Chay that tren 20 may bat duoc hai loi khong test nao bat duoc (17/08/2026)](diary/03-1508-2108.md#987-chay-that-tren-20-may-bat-duoc-hai-loi-khong-test-nao-bat-duoc-17082026) | `diary/03-1508-2108.md` |
| §9.88 | [Menu chức năng từng máy: đo được gì trên máy thật, và bốn cái bẫy (21/08/2026)](diary/03-1508-2108.md#988-menu-chức-năng-từng-máy-đo-được-gì-trên-máy-thật-và-bốn-cái-bẫy-21082026) | `diary/03-1508-2108.md` |
| §9.89 | [Lúc phóng to cũng phải có đủ chức năng, và nhãn + icon app thật (21/08/2026)](diary/03-1508-2108.md#989-lúc-phóng-to-cũng-phải-có-đủ-chức-năng-và-nhãn-icon-app-thật-21082026) | `diary/03-1508-2108.md` |
| §9.90 | ["Ba dòng này không chạy" — cả ba đều chạy, và đó mới là vấn đề (21/08/2026)](diary/03-1508-2108.md#990-ba-dòng-này-không-chạy-cả-ba-đều-chạy-và-đó-mới-là-vấn-đề-21082026) | `diary/03-1508-2108.md` |
| §9.91 | [Hover mở submenu, một vùng cuộn, và `[object Object]` (21/08/2026)](diary/03-1508-2108.md#991-hover-mở-submenu-một-vùng-cuộn-và-object-object-21082026) | `diary/03-1508-2108.md` |
| §9.92 | [Một danh sách, và cái `max-height` cắt mất App List (21/08/2026)](diary/03-1508-2108.md#992-một-danh-sách-và-cái-max-height-cắt-mất-app-list-21082026) | `diary/03-1508-2108.md` |
| §9.93 | [Một nhãn bị hỏi "là sao?", và màu xanh trong một sản phẩm màu cam (21/08/2026)](diary/03-1508-2108.md#993-một-nhãn-bị-hỏi-là-sao-và-màu-xanh-trong-một-sản-phẩm-màu-cam-21082026) | `diary/03-1508-2108.md` |
| §9.94 | [Bốn thanh kéo chia nhau một trăm phần trăm (21/08/2026)](diary/05-1308-2408.md#994-bốn-thanh-kéo-chia-nhau-một-trăm-phần-trăm-21082026) | `diary/05-1308-2408.md` |
| §9.95 | [Thanh kéo đổi thang đo, và một công tắc tắt vẫn bị thu tiền (21/08/2026)](diary/05-1308-2408.md#995-thanh-kéo-đổi-thang-đo-và-một-công-tắc-tắt-vẫn-bị-thu-tiền-21082026) | `diary/05-1308-2408.md` |
| §9.96 | [`[object Object]` ở 47 chỗ, và ba lỗi mà chỉ e2e nhìn thấy (21/08/2026)](diary/05-1308-2408.md#996-object-object-ở-47-chỗ-và-ba-lỗi-mà-chỉ-e2e-nhìn-thấy-21082026) | `diary/05-1308-2408.md` |
| §9.97 | [Mô hình đe doạ cho hai cổng nghe, và chín lỗ đã bịt (22/08/2026)](diary/05-1308-2408.md#997-mô-hình-đe-doạ-cho-hai-cổng-nghe-và-chín-lỗ-đã-bịt-22082026) | `diary/05-1308-2408.md` |
| §9.98 | [Bốn lỗi mà chỉ việc dọn mới lôi ra, và năm mục tôi từ chối làm (23/08/2026)](diary/05-1308-2408.md#998-bốn-lỗi-mà-chỉ-việc-dọn-mới-lôi-ra-và-năm-mục-tôi-từ-chối-làm-23082026) | `diary/05-1308-2408.md` |
| §9.99 | [Sáu máy kẹt sau một trang không ai gỡ được, và cái thang phải chuyển nhà (23/08/2026)](diary/05-1308-2408.md#999-sáu-máy-kẹt-sau-một-trang-không-ai-gỡ-được-và-cái-thang-phải-chuyển-nhà-23082026) | `diary/05-1308-2408.md` |
| §9.100 | [Hai máy khoá màn hình, một câu báo lỗi nói sai, và thanh tiến trình đầu tiên (23/08/2026)](diary/05-1308-2408.md#9100-hai-máy-khoá-màn-hình-một-câu-báo-lỗi-nói-sai-và-thanh-tiến-trình-đầu-tiên-23082026) | `diary/05-1308-2408.md` |
| §9.101 | [Giá tiền tự bịa, một cổng vision hết hạn, và cái field `vision_body` không gửi (23/08/2026)](diary/05-1308-2408.md#9101-giá-tiền-tự-bịa-một-cổng-vision-hết-hạn-và-cái-field-vision_body-không-gửi-23082026) | `diary/05-1308-2408.md` |
| §9.102 | [Tại sao 3 máy không lướt ngang — và đo ra thì nhãn có, tôi đọc sai một lần (23/08/2026)](diary/05-1308-2408.md#9102-tại-sao-3-máy-không-lướt-ngang-và-đo-ra-thì-nhãn-có-tôi-đọc-sai-một-lần-23082026) | `diary/05-1308-2408.md` |
| §9.103 | [Bằng chứng cho bình luận: tấm ghép, khung trùng, và thứ tự](diary/05-1308-2408.md#9103-bằng-chứng-cho-bình-luận-tấm-ghép-khung-trùng-và-thứ-tự) | `diary/05-1308-2408.md` |
| §9.104 | [`card_is_still` cũng băm cả khung; và ba máy chung một AP không có mạng](diary/05-1308-2408.md#9104-card_is_still-cũng-băm-cả-khung-và-ba-máy-chung-một-ap-không-có-mạng) | `diary/05-1308-2408.md` |
| §9.105 ⚠️ | [Mention thật cần phím thật; view tích luỹ; và một cổng đo bắn vào splash (24/08/2026)](diary/05-1308-2408.md#9105-mention-thật-cần-phím-thật-view-tích-luỹ-và-một-cổng-đo-bắn-vào-splash-24082026) | `diary/05-1308-2408.md` |
| §9.105 ⚠️ | [tiếp — bốn máy đó vẫn ở đúng bài, và ba lần tôi nói khác đều là lỗi dụng cụ](diary/05-1308-2408.md#9105-tiếp-bốn-máy-đó-vẫn-ở-đúng-bài-và-ba-lần-tôi-nói-khác-đều-là-lỗi-dụng-cụ) | `diary/05-1308-2408.md` |
| §9.106 | [Lịch tự chạy của nuôi TikTok (24/08/2026)](diary/05-1308-2408.md#9106-lịch-tự-chạy-của-nuôi-tiktok-24082026) | `diary/05-1308-2408.md` |
| §9.107 | [Khung giờ cho lịch nuôi, và nút chọn tất cả (24/08/2026)](diary/06-2408-2708.md#9107-khung-giờ-cho-lịch-nuôi-và-nút-chọn-tất-cả-24082026) | `diary/06-2408-2708.md` |
| §9.108 | [Chạy Tương tác ở quy mô 20 máy (25/08/2026)](diary/06-2408-2708.md#9108-chạy-tương-tác-ở-quy-mô-20-máy-25082026) | `diary/06-2408-2708.md` |
| §9.109 | [Bài nhiều ảnh: bình luận viết từ ảnh 1 (25/08/2026)](diary/06-2408-2708.md#9109-bài-nhiều-ảnh-bình-luận-viết-từ-ảnh-1-25082026) | `diary/06-2408-2708.md` |
| §9.110 | [Tiền thật của một bình luận, và 6/10 token là chữ nghĩ thầm (25/08/2026)](diary/06-2408-2708.md#9110-tiền-thật-của-một-bình-luận-và-610-token-là-chữ-nghĩ-thầm-25082026) | `diary/06-2408-2708.md` |
| §9.111 | [Gộp một lượt nháp cho cả link, và cái gộp bị phép đo loại bỏ (25/08/2026)](diary/06-2408-2708.md#9111-gộp-một-lượt-nháp-cho-cả-link-và-cái-gộp-bị-phép-đo-loại-bỏ-25082026) | `diary/06-2408-2708.md` |
| §9.112 | [Fan-out Riêng lẻ đã giết chống trùng, và bốn bình luận giống nhau đã lên thật (25/08/2026)](diary/06-2408-2708.md#9112-fan-out-riêng-lẻ-đã-giết-chống-trùng-và-bốn-bình-luận-giống-nhau-đã-lên-thật-25082026) | `diary/06-2408-2708.md` |
| §9.113 | [Gộp bị từ chối thì hỏi gộp lần nữa; và hai test đỏ vì CRLF (26/08/2026)](diary/06-2408-2708.md#9113-gộp-bị-từ-chối-thì-hỏi-gộp-lần-nữa-và-hai-test-đỏ-vì-crlf-26082026) | `diary/06-2408-2708.md` |
| §9.114 | [5/5 máy, và cái máy hỏng bốn lượt liền là stream kẹt chứ không phải code (26/08/2026)](diary/06-2408-2708.md#9114-55-máy-và-cái-máy-hỏng-bốn-lượt-liền-là-stream-kẹt-chứ-không-phải-code-26082026) | `diary/06-2408-2708.md` |
| §9.115 ⚠️ | [Bằng chứng lấy từ web, không lấy từ máy — và ảnh cuối là ảnh quan trọng nhất (26/08/2026)](diary/06-2408-2708.md#9115-bằng-chứng-lấy-từ-web-không-lấy-từ-máy-và-ảnh-cuối-là-ảnh-quan-trọng-nhất-26082026) | `diary/06-2408-2708.md` |
| §9.115 ⚠️ | [tiếp — cột `context_json` giờ có người đọc (26/08/2026)](diary/06-2408-2708.md#9115-tiếp-cột-context_json-giờ-có-người-đọc-26082026) | `diary/06-2408-2708.md` |
| §9.115 ⚠️ | [tiếp — dọn bốn việc treo, và cái nào cũng có cổng (26/08/2026)](diary/06-2408-2708.md#9115-tiếp-dọn-bốn-việc-treo-và-cái-nào-cũng-có-cổng-26082026) | `diary/06-2408-2708.md` |
| §9.115 ⚠️ | [tiếp — lời thoại: có rồi, và cái prompt cũ đã ăn mất nó hai lần (26/08/2026)](diary/06-2408-2708.md#9115-tiếp-lời-thoại-có-rồi-và-cái-prompt-cũ-đã-ăn-mất-nó-hai-lần-26082026) | `diary/06-2408-2708.md` |
| §9.115 ⚠️ | [tiếp — tính năng này đã "xong" mà **không chạy** trên bản cài (26/08/2026)](diary/06-2408-2708.md#9115-tiếp-tính-năng-này-đã-xong-mà-không-chạy-trên-bản-cài-26082026) | `diary/06-2408-2708.md` |
| §9.115 ⚠️ | [tiếp — video giờ được *xem*, và tấm ghép nói ra nó xem được mấy giây (26/08/2026)](diary/06-2408-2708.md#9115-tiếp-video-giờ-được-xem-và-tấm-ghép-nói-ra-nó-xem-được-mấy-giây-26082026) | `diary/06-2408-2708.md` |
| §9.115 ⚠️ | [tiếp — `video_gate` đã chạy máy thật, và nó lôi ra hai lỗi (26/08/2026)](diary/06-2408-2708.md#9115-tiếp-video_gate-đã-chạy-máy-thật-và-nó-lôi-ra-hai-lỗi-26082026) | `diary/06-2408-2708.md` |
| §9.116 | ["Nhận điện thoại rồi mà điều khiển không được" — và cái app đã không nói (26/08/2026)](diary/06-2408-2708.md#9116-nhận-điện-thoại-rồi-mà-điều-khiển-không-được-và-cái-app-đã-không-nói-26082026) | `diary/06-2408-2708.md` |
| §9.117 | ["Mở thư mục máy còn mở không được" — hai lỗi, và cái log chôn cả hai (26/08/2026)](diary/06-2408-2708.md#9117-mở-thư-mục-máy-còn-mở-không-được-hai-lỗi-và-cái-log-chôn-cả-hai-26082026) | `diary/06-2408-2708.md` |
| §9.118 | [Một lần sập giờ để lại một dòng — hai nửa, và cả hai đang trống (26/08/2026)](diary/06-2408-2708.md#9118-một-lần-sập-giờ-để-lại-một-dòng-hai-nửa-và-cả-hai-đang-trống-26082026) | `diary/06-2408-2708.md` |
| §9.119 | [Pha 1: hết lỗi đã biết — và bốn chỗ tôi nói quá, ghi lại cho đúng (26–27/08/2026)](diary/06-2408-2708.md#9119-pha-1-hết-lỗi-đã-biết-và-bốn-chỗ-tôi-nói-quá-ghi-lại-cho-đúng-2627082026) | `diary/06-2408-2708.md` |
| §9.120 | [Tài liệu: một file 10.385 dòng thành một kho, và bảy khẳng định trái với mã (27/08/2026)](diary/06-2408-2708.md#9120-tài-liệu-một-file-10385-dòng-thành-một-kho-và-bảy-khẳng-định-trái-với-mã-27082026) | `diary/06-2408-2708.md` |
| §9.121 | [Codex review sáu lượt: 20 lỗi, và ba lần tôi tự bắt mình (27/08/2026)](diary/06-2408-2708.md#9121-codex-review-sáu-lượt-20-lỗi-và-ba-lần-tôi-tự-bắt-mình-27082026) | `diary/06-2408-2708.md` |
| §9.122 | [Một chữ "đã root" trả lời hai câu hỏi, và 9/20 máy trả lời ngược nhau (28/08/2026)](diary/06-2408-2708.md#9122-một-chữ-đã-root-trả-lời-hai-câu-hỏi-và-920-máy-trả-lời-ngược-nhau-28082026) | `diary/06-2408-2708.md` |
| §9.123 | [Codex bốn lượt cho vùng Flow: 19 lỗi, và ba lần test của tôi là test rỗng (28/08/2026)](diary/06-2408-2708.md#9123-codex-bốn-lượt-cho-vùng-flow-19-lỗi-và-ba-lần-test-của-tôi-là-test-rỗng-28082026) | `diary/06-2408-2708.md` |
| §9.124 | [Một lượt review "thất bại" nằm chờ trên đĩa 26 KB, và ba đường mất việc chưa lưu (28/08/2026)](diary/06-2408-2708.md#9124-một-lượt-review-thất-bại-nằm-chờ-trên-đĩa-26-kb-và-ba-đường-mất-việc-chưa-lưu-28082026) | `diary/06-2408-2708.md` |
| §9.125 | [Fleet cắm lại: ba cổng đạt, badge 46.2.42 đo được, và ba tool lạ trên một máy (28/08/2026)](diary/06-2408-2708.md#9125-fleet-cắm-lại-ba-cổng-đạt-badge-46242-đo-được-và-ba-tool-lạ-trên-một-máy-28082026) | `diary/06-2408-2708.md` |
| §9.126 | [Composer đo lần đầu trên build 16/20 máy: ô thư viện nằm ngược phía, và cờ "đã chọn đủ ảnh" là cái Eleme](diary/06-2408-2708.md#9126-composer-đo-lần-đầu-trên-build-1620-máy-ô-thư-viện-nằm-ngược-phía-và-cờ-đã-chọn-đủ-ảnh-là-cái-elementbox-không-nhìn-thấy-29082026) | `diary/06-2408-2708.md` |
| §9.127 | [Codex bốn vùng cho đường đăng bài: 44 lỗi, và mười lăm test của tôi là test rỗng (29/08/2026)](diary/06-2408-2708.md#9127-codex-bốn-vùng-cho-đường-đăng-bài-44-lỗi-và-mười-lăm-test-của-tôi-là-test-rỗng-29082026) | `diary/06-2408-2708.md` |
| §9.128 | [Đường đăng bài nối xong, và một lần tôi bác bỏ nhầm phát hiện của người review (30/08/2026)](diary/06-2408-2708.md#9128-đường-đăng-bài-nối-xong-và-một-lần-tôi-bác-bỏ-nhầm-phát-hiện-của-người-review-30082026) | `diary/06-2408-2708.md` |

---

Trước 27/08/2026 tất cả những mục trên nằm trong **một** file 10.385 dòng, không mục lục,
số mục không theo thứ tự file. Nó đã lừa được chính người viết nó nhiều lần trong tuần đó
— xem §9.120. Việc chia file không đổi một chữ nào của nội dung: phép chia được kiểm bằng
cách dựng lại và so từng dòng với bản gốc.
