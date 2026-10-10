## 7. Nguyên tắc khi sửa code này

- **Không báo thành công thứ chưa xác nhận.** Bản cũ ghi `done` cho phiên xử lý 0
  video; `ensure_ready()` luôn trả `Ok(())` che lỗi; `watch_and_clear_popups()`
  chỉ `sleep`. Đừng tạo lại kiểu đó.
- **Không tap mù.** Detector không chắc thì không tap. Toạ độ ở nửa dưới màn hình
  đặc biệt nguy hiểm (thanh nav, nút Home).
- **ACK không phải hiệu ứng, kể cả với tap.** Trên máy chậm, tap contact có thể bị bỏ
  qua hoặc chỉ hiện sau 9–13 s (tab Hot, #47) hay ≥110 s (#44, máy quá tải, xem
  [fleet](09-fleet-android.md#sức-khỏe-máy-trước-code)). Chứng minh hiệu ứng trên khung
  hình mới. Chỉ tap lại sau cửa sổ settle đã đo và khi khung mới chứng minh dương tính
  rằng tap đầu chưa ăn. Không bao giờ tap lại một toggle (hàng nhạc) trên khung chưa
  chứng minh. Hằng số đo nằm ở `crates/core/src/tiktok_sound/visual.rs`.
- **Mọi hằng số hình học phải kèm số đo thật** trong comment, ghi rõ đo trên máy nào.
- **Ngân sách recovery phải hữu hạn** và phải log rõ đang tiêu cái gì.
- Log hướng tới người vận hành, tiếng Việt, nói đúng chuyện đang xảy ra.
- **Trần không phải mục tiêu.** `--videos` là giới hạn trên; phiên chạy theo đồng
  hồ dừng khi hết giờ với trần còn nguyên, và đó vẫn là phiên trọn vẹn. Từng có
  bug báo `partial` cho phiên 47 video hoàn toàn khoẻ vì so với trần 400.
- **Cửa sổ xác nhận phải tính theo tốc độ stream thật** (~7 FPS, chỉ đẩy khi đổi).
  Cửa sổ quá gắt làm hành động thật bị báo thất bại rồi lặp lại — với vuốt thì
  hậu quả là nhảy mất video.

---
