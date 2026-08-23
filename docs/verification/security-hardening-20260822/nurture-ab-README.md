# `run_session` tách bảy pha — nghiệm thu A/B trên máy thật (23/08/2026)

Đợt E5 rút `run_session` từ **1.369 xuống 631 dòng**. Đó là một phép *di chuyển*, và câu hỏi
nghiệm thu duy nhất đáng hỏi là: hành vi trên phần cứng thật có đổi không.

598 test riviu-core xanh mà không sửa một dòng test nào là điều kiện cần, không phải điều kiện
đủ — `nurture/mod.rs` chạy trên máy thật với TikTok thật, và phần lớn đường chạy đó không có
test nào che.

## Cách đo

Giữa `cc8dbd3` và `27174ae`, **đúng một file** trong `crates/` đổi
(`crates/core/src/nurture/mod.rs`). Nên phép A/B là sạch: lấy lại bản cũ của riêng file đó,
build cùng một binary, chạy **cùng 10 máy, cùng tham số**, rồi so từng máy một.

```
RIVIU_ADB_PATH=…/sidecars/android/win-x86_64/adb.exe \
  ./target/debug/live_nurture_android.exe --devices 10 --minutes 4 --videos 3
```

- `nurture-ab-before-cc8dbd3.txt` — bản trước khi tách
- `nurture-ab-after-27174ae.txt` — bản sau khi tách bảy pha

## Kết quả: 10/10 cùng loại kết quả

| máy | trước | sau |
|---|---|---|
| 98895a3355424e484f | failed | failed |
| 9889db374744474635 | failed | failed |
| ce021712b33054090c | failed | failed |
| ce021712d2ae60880c | failed | failed |
| **ce031713840038030c** | **done** v=2 t=0 | **done** v=3 t=2 |
| ce03171392f9390c01 | failed | failed |
| ce031713dd735a1103 | failed | failed |
| ce0517151215a00304 | failed | failed |
| **ce0517155ab38c390d** | **done** v=3 t=1 | **done** v=3 t=0 |
| ce051715ac247a3f01 | failed | failed |

Hai máy `done` thì done ở cả hai bản; tám máy hỏng thì hỏng **cùng một thông điệp** ở cả hai
bản. Số video/tim lệch nhau nằm trong biên độ của chính các xác suất ngẫu nhiên mà engine dùng
(`like_prob`, `frenzy_prob`, mood multipliers) — **đừng đọc 3 vs 2 là "tốt lên"**.

## Tám máy hỏng là trạng thái app, không phải code

Sáu máy dừng ở `chờ 30s mà chưa thấy tab feed. TikTok có thể còn ở màn khởi động, hoặc đang ở
trang chọn chủ đề / đăng nhập — dừng thay vì vuốt mù`, và log cho thấy đường thoát chạy đúng
như thiết kế: `màn hình bị chặn và không có nút nào đọc được — bấm Back (1/3 → 3/3)` rồi mới bỏ
cuộc. Một máy hỏng ở khởi động WDA. Đó là fleet cần đăng nhập lại TikTok, không phải lỗi engine
— và bản **trước** refactor hỏng y hệt.

## Bảy pha đều đã chạy trên phần cứng thật

Đọc log của `ce031713840038030c` và `ce021712b33054090c` thì thấy đủ:

| pha | dòng log chứng minh |
|---|---|
| `open_for_session` | `khởi động WDA mới`, `nhãn đã đo: com.ss.android.ugc.trill / en` |
| `watch_one_card` | `xem 3.8s (theo đúng tỉ lệ đặt)` |
| `roll_and_execute_action` | `thả tim` → `tim thành công (nhãn đổi trạng thái)` |
| `swipe_to_next_video` | `vuốt chưa chứng minh được đổi thẻ (1/4)`, `(2/4)` |
| phục hồi trong `swipe_to_next_video` | `feed không đổi thẻ — khởi động lại TikTok` → `feed đã lên` |
| `handle_off_feed` + `give_up` | `bấm Back (3/3)` → `failed — chờ 30s mà chưa thấy tab feed…` |
| `settle_after_advance` | `thẻ không có thanh hành động (LIVE / đang chuyển) — chỉ vuốt tiếp` |
| `session_verdict` | `done — 3/3 video…` và `failed — 0/3 video…` |

## Lưu ý khi chạy lại

Đừng mở app desktop cùng lúc — hai tiến trình tranh cùng bộ máy là đúng thứ dự án này đã mất
một tuần để bỏ. Bình luận **tắt** trừ khi truyền `--comment-prob`, và lần chạy này khoá AI
trống nên không có bình luận nào được đăng.
