## 7. Nguyên tắc khi sửa code này

- **Không báo thành công thứ chưa xác nhận.** Bản cũ ghi `done` cho phiên xử lý 0
  video; `ensure_ready()` luôn trả `Ok(())` che lỗi; `watch_and_clear_popups()`
  chỉ `sleep`. Đừng tạo lại kiểu đó.
- **Không tap mù.** Detector không chắc thì không tap. Toạ độ ở nửa dưới màn hình
  đặc biệt nguy hiểm (thanh nav, nút Home).
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
