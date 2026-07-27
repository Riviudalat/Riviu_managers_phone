# Kế hoạch: Comment GÕ CHỮ như người thật (ưu tiên #1) + tối ưu 3 máy

## Context

3 máy đã sẵn sàng (`05101fdb`, `a99f4bd9`, `e561b690`, iPhone 8 iOS 16.7.x).
User muốn **acc bình luận bằng chữ do AI viết, gõ như người thật** — KHÔNG phải
emoji reaction. Emoji chỉ là phương án dự phòng khi không gõ được.

**Vì sao có cơ sở** (không phải bất khả như kết luận trước):
- Bàn phím iOS QWERTY **đã hiện THẬT 1 lần** trong ô comment (paste2 `LP-02`, có
  ảnh, con trỏ đỏ trong ô).
- Một khi bàn phím lên, **tap phím = chạm thật** mà TikTok CHẤP NHẬN — đã chứng
  minh: tap ô emoji chèn được, tap "@" chèn "@" vào ô. Phím chữ cũng chỉ là nút.
- Cái chưa làm được: **raise bàn phím ỔN ĐỊNH**. TikTok 45.8.0 thay inputView của
  ô bằng panel emoji/sticker riêng, nên bàn phím hệ thống không tự lên mọi lúc.
- Lưu ý: `/wda/keys` (bơm keystroke tổng hợp) KHÔNG chèn trên WDA stock — TikTok
  bỏ qua keystroke ảo; TOOL TIKTOK gõ được vì dùng WDA vá (TrollStore, bất khả ở
  iOS 16.7.x). ⇒ Đường DUY NHẤT trên máy này là **bàn phím thật + tap phím**.

---

## PHẦN A (chính) — GÕ BÌNH LUẬN BẰNG BÀN PHÍM

### A1. Crack cách RAISE bàn phím ổn định (make-or-break — làm TRƯỚC)

Quan sát bằng kênh DVT `tidevice screenshot` SONG SONG với thao tác WDA (WDA nối
tiếp gesture nên screenshot của chính nó bắt hụt bàn phím thoáng qua — đã đo).
Phát hiện bàn phím: **xám vùng đáy ≥0.15** (đã hiệu chuẩn: bàn phím 0.39 vs panel
emoji 0.012). Thử có hệ thống, mỗi cách ≥15-20 lần trên nhiều video:

1. **Cold-start TikTok trước mỗi comment** (giả thuyết mạnh nhất — đúng chuỗi
   paste2 đã cho ra bàn phím): `tidevice launch` lại TikTok → mở drawer → tap ô →
   long-press. Nghi lần đầu sau cold-start composer mở ở chế độ bàn phím.
2. **Long-press ô đã focus** trong drawer/composer, đo bằng DVT trong lúc giữ.
3. **Tap đúp / tap-giữ-nhả** ô đã focus.
4. **Toggle icon giữa** (135,614)px và các nút trên thanh soạn — thử lại với toạ
   độ chính xác + DVT.

**Cổng go/no-go**: nếu có cách raise được **≥~50%** số lần → sang A2. Nếu không
cách nào ổn định → báo user trung thực (kèm số liệu), **giữ emoji làm mặc định**,
và cân nhắc đường phần cứng (máy iOS ≤16.6.1 + TrollStore cho `/wda/keys`).

### A2. Gõ phím như người thật (khi A1 đạt)

- **Bản đồ phím QWERTY** đã đo sẵn (scratchpad/probe_keys.py): hàng Q..P y≈480pt,
  A..L y≈534, Z..M y≈588, space y≈642, phím "Gửi" góc phải, "123" cho số, ⇧ shift.
- **Sinh comment**: `generate_vision_comment` (openai_client.rs, đã có) — câu ngắn
  tiếng Việt hợp nội dung video, `max_comment_words` (types.rs, mặc định 12).
- **Ký tự gõ được**: bắt đầu bằng **tiếng Việt KHÔNG dấu** ("dep qua ban oi") — rất
  phổ biến trong comment TikTok, né phần long-press nguyên âm có dấu (phức tạp).
  Có shift cho chữ hoa đầu câu. (Giai đoạn 2 mới thêm dấu nếu cần.)
- **Gõ như người thật**: tap từng phím với nhịp ngẫu nhiên 120–350ms/phím, thỉnh
  thoảng dừng lâu hơn (nghĩ), đôi khi gõ sai rồi xoá (backspace) — tự nhiên.
- **Xác nhận**: đếm pixel tối trong ô nhập tăng dần theo số ký tự; khi đủ → tap
  "Gửi"/nút gửi; xác nhận ô rỗng lại = đã đăng.
- **Hybrid an toàn trong `do_comment`**: nếu raise được bàn phím → gõ text (đường
  chính); nếu không raise được ở video đó → rơi về **emoji reaction** (đường đang
  chạy, sau khi đã fix ở Phần B). Kể cả A1 chỉ ~50% thì vẫn có nửa số comment là
  TEXT thật + nửa còn lại emoji.

### A3. Nối vào engine
`do_comment` (actions.rs:268-448) thêm nhánh text-first: mở drawer → thử raise
bàn phím (A1) → nếu có, gõ (A2) và trả `CommentResult::Sent`; nếu không, chạy tiếp
nhánh emoji hiện có. Thêm setting `comment_mode` (text|emoji|auto) trong
`NurtureSettings` (types.rs), mặc định `auto` (text ưu tiên, emoji dự phòng).

---

## PHẦN B (hỗ trợ) — SỬA & TỐI ƯU (để nền tảng chắc + emoji dự phòng tốt)

### B1. Fix emoji NotArmed (đường dự phòng phải đáng tin)
Root-cause: sticker "Yellow Dog" vỡ thành ≥5 mảng vàng → lọt `MIN_EMOJI_PER_ROW=5`
→ `find_emoji_grid` không rỗng → tap trúng sticker → không chèn → `NotArmed`.
- `screen.rs`: thêm `EMOJI_COLS`(7 cột đo được)+`emoji_grid()`/`is_emoji_grid()`
  (giữ hàng khớp ≥6/7 cột, trả tâm cột CỐ ĐỊNH). emoji=[7,7,7,6,6] vs sticker=[4,4,4].
- `actions.rs`: sau tap tab ☺, poll `is_emoji_grid` (bỏ sleep 900ms cứng); nếu
  vẫn sticker → tap lại tab (≤2 lần) → nếu vẫn không thì `CommentResult::StickerPanel`.
  Dùng `emoji_grid`, bỏ clamp cột; mở rộng retry ±1 cột.
- Test: thêm fixture sticker (`/tmp/riviu-frames/cmt/05101fdb/0002-unknown.jpg`).

### B2. Chạy 2–3 máy đồng thời (MJPEG throttle — dead code)
`_configure_mjpeg`/`_tune_mjpeg_http` (riviu_pmd.py:279/795) không được gọi →
stream full-res làm nghẽn usbmux khi 3 máy.
- `riviu_pmd.py`: gọi `_tune_mjpeg_http` trong `cmd_wda_proxy` NGAY SAU `ready=True`
  (setting MJPEG là global trên WDA server); thêm cờ `--mjpeg-fps 12 --mjpeg-quality
  50 --mjpeg-scaling 50`.
- `pmd.rs`: `spawn_proxy_locked` truyền 3 cờ (hằng `MJPEG_FPS/QUALITY/SCALING`).
- Băng thông ≈ 1/8 tải/máy (scaling 1/4 pixel × fps 1/2). Không xuống <10fps.
- Kiểm chứng: đo bề rộng frame đầu (~375px = đã áp scaling).

### B3. Tin cậy + tốc độ
- Half-res làm `compose_bar_visible` (dải "+" hẹp) nhạy nhất → nới `PLUS_CYAN_X`/
  `PLUS_PINK_X` ±0.006, hạ `PLUS_*_MIN` 60→48. Tạo fixture half-res, chạy lại
  `real_frames.rs`, CHỈ chỉnh ngưỡng nào fail.
- Cắt sleep CỐ ĐỊNH (giữ watch+realism): after_swipe 800-1600→500-1100, think
  400-1400→300-1000, like/follow pre-tap & confirm 2500→2000. Giữ `SWIPE_SETTLE`.

### B4. Tăng tần suất comment (mood)
E[comment_mult]≈0.42 do Skimming 60%. Option B: cân `MoodCycle::roll` (Skim
0..44/Liking 45..79/Chatty 80..99) + `comment_mult` Liking 0.5→1.2, Chatty
3.0→3.5 → E≈1.0. (human_behavior.rs).

### B5. Harness đa máy
`live_nurture_test.rs`: thêm `--udids a,b,c`, spawn `run_session`/udid với stagger
(mẫu `NurtureRuntime::start_many`) → test 3 máy đồng thời.

---

## Thứ tự thực hiện
1. **A1** — crack raise bàn phím (make-or-break). Báo user kết quả + go/no-go NGAY.
2. Nếu A1 đạt → **A2/A3** gõ text như người thật. Nếu không → báo trung thực, tiếp B.
3. **B1** fix emoji (dự phòng). **B2+B5** concurrency. **B3+B4** tinh chỉnh.

## Verification
- **A (text)**: live 1 máy, đếm comment TEXT gửi thành công (nội dung khớp video,
  gõ nhịp người); đo tỉ lệ raise bàn phím. Mục tiêu: đăng được comment chữ thật.
- **B (emoji/concurrency)**: `cargo test --workspace` (fixture emoji/sticker, mood,
  half-res); live 2–3 máy đồng thời không vỡ phân loại.
- Cập nhật `AGENTS.md` + `docs/LIVE_NURTURE_REPORT_2026-07-26.md`.

## Rủi ro chính (trung thực)
A1 có thể KHÔNG ổn định (bàn phím không tất định) → text không làm được trên máy
16.7.x, phải dùng emoji hoặc chuyển sang máy ≤16.6.1 + TrollStore. Sẽ biết chắc
sau A1 và báo bạn trước khi bỏ công vào A2.
