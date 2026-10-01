# Root cause, phòng vệ và chờ có điều kiện

Bản local chọn lọc dựa trên các references Superpowers đã đọc. Không kèm script
tìm polluter, ví dụ phụ thuộc Lace, pressure fixtures hoặc thao tác signing.

## Theo dấu ngược

1. Tại nơi kết quả sai, xác định dữ liệu/identity sai đã tới từ boundary nào.
2. Đi ngược từng caller bằng source/trace, không đoán từ message hiển thị.
3. Chọn điểm đầu tiên invariant bị phá; sửa ở owner của invariant, không chỉ nơi crash.
4. Giữ guard downstream cần thiết; thêm defensive check không thay root-cause fix.
5. Kiểm input hợp lệ trước đây vẫn chạy, missing/ambiguous/stale từ chối đúng.

Ví dụ metadata an toàn: `requestId`, `operation`, `sourceCommit`, `epoch`, elapsed,
status và `credentialPresent: true/false`. Không in giá trị credential, path nhạy cảm,
account riêng hoặc nội dung gửi. Không enumerate kho secret để kiểm có khóa.

## Defense in depth

- Validator ở ingress, contract domain, guard trước effect và settlement CAS có vai
  trò khác nhau; tránh validate một lần rồi dùng proof mãi qua mutation/restart.
- Path containment không kiểm bằng string prefix. Dùng helper canonical/path-component
  đã có, chặn symlink/reparse, xem race tại điểm mở/ghi. `tmp-other` không thuộc `tmp`.
- Ownership phải kiểm đúng caller/device/session/process incarnation, không chỉ tên,
  PID hay cổng. Ambiguous/không đọc được identity phải fail closed.
- Không catch lỗi rồi trả success/empty để yên UI. Preserve typed failure và evidence.

## Chờ có điều kiện

- Chỉ poll quan sát có contract, một total deadline và cancellation; cadence theo chi
  phí thật. Không dùng 10ms ví dụ in-memory cho USB/HTTP/SQLite.
- Async predicate phải await kết quả; Promise/future có mặt không có nghĩa ready.
- Unknown, incomplete hoặc transport error không chứng minh absence. Kiểm identity
  và freshness trước dùng match; snapshot mới có thể thuộc phiên/app khác.
- Không thay mọi sleep: duration xem video, nghỉ, backoff có ý nghĩa nghiệp vụ riêng.
- Không bọc timeout ngoài gesture đã gửi; để primitive drain và đối soát. Polling
  không được gọi tap/Send/Post như predicate “thử tới khi được”.

## Khi test lỗi không ổn định

Giữ input/seed/source/version và timing thực. Chạy test riêng chỉ để phân biệt shared
state/tải, không dùng rerun may mắn làm chứng nhận. Test order, cache reuse và leaked
worker cần fake clock/process hoặc scope tách biệt. Đừng nuốt exit status của test
runner; không có test được chọn phải báo rõ, không ghi PASS.
