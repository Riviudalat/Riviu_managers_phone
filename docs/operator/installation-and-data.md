# Cài đặt, chuyển máy và dữ liệu

[Hướng dẫn vận hành](../operator-guide.md)

## Bốn loại dữ liệu khác nhau

| Loại | Theo Git/đăng nhập Orca? | Cách xử lý |
|---|---|---|
| Source và skill project | Theo đúng checkout/branch Git | Kiểm commit; không suy version executable từ source |
| Installer/runtime | Không tự theo Git | Lấy artifact đúng version, kiểm hash và cài theo hướng dẫn |
| DB, media staging, evidence | Dữ liệu local của Riviu | Sao lưu nhất quán, không xóa để xử lý pending |
| Token Google/AI/agent | Kho credential hệ điều hành | Đăng nhập/cấu hình riêng trên PC mới; không copy token qua chat |

Windows mặc định dùng Known Folder `dirs::data_dir()` cộng `riviu-managers-phone`;
macOS thường là `~/Library/Application Support/riviu-managers-phone`. Đường thực tế
có thể khác theo chế độ chạy. Không dùng APPDATA override để giả rằng DB đã cô lập.

## Máy mới

1. Cài bản Riviu đúng nền tảng. Installer Windows chưa ký Authenticode, macOS ad-hoc;
   xác minh nguồn/hash trước khi chấp thuận cảnh báo hệ điều hành.
2. Cài USB driver hệ điều hành/nhà cung cấp phù hợp. Android cần USB debugging và
   xác nhận trên máy; iPhone cần Trust, Apple support và provisioning tương thích.
3. Không mở hai controller cạnh tranh cùng phone. Kiểm roster và Chẩn đoán trước.
4. Đăng nhập Google riêng, chọn đúng link/tab. OAuth client config trong binary
   không mang tài khoản hoặc refresh token của máy build.
5. Nếu tab Sheet shared-v2 đã có writer khác, làm theo quy trình join/upgrade;
   không xóa lock vì chờ lâu. Campaign cũ không tự chuyển sang target mới.
6. Nghiệm thu giới hạn trên đúng PC; mở app thành công chưa chứng minh phone/Sheet.

## Sao lưu

- Dừng nhận việc mới, chờ worker/lease drain và đóng app đúng quy trình. Ghi version,
  commit/bản cài, thời điểm và trạng thái các lượt chưa rõ.
- Nhờ kỹ thuật dùng SQLite backup API hoặc `VACUUM INTO` đúng DB đã xác định; chỉ
  copy file DB thô khi đã xác minh không còn writer và trạng thái WAL đã được xử lý.
  Không copy `riviu.db` đang chạy mà bỏ `-wal` rồi coi là backup đầy đủ.
- Giữ media staging và artifacts cần cho các receipt chưa chốt; giữ source nội dung
  riêng. Kiểm hash/kích thước và thử mở bản sao offline, không trên controller thật.
- Backup chứa dữ liệu tài khoản/nội dung: lưu ở nơi có kiểm soát truy cập, không commit
  vào Git hoặc gửi dịch vụ ngoài. Credential OS không nằm đầy đủ trong backup DB.

## Phục hồi và hạ phiên bản

Không có hướng dẫn một lệnh an toàn cho mọi trạng thái. Kỹ thuật phải kiểm schema,
version, pending effect, outbox và artifact trước khi thay DB. Giữ bản hiện tại để
rollback. Không mở binary cũ lên DB đã migrate chỉ vì installer cũ còn chạy được.

**Script rollback lịch sử không phải công cụ sửa dữ liệu hiện tại.** Đặc biệt
`docs/verification/nurture-human-v2-20260806/rollback-db.sh` phục hồi **toàn bộ DB**,
không chỉ settings; chạy trên DB vận hành có thể mất campaign/receipt mới. Chỉ tái
hiện trên bản sao với input lịch sử tương ứng, không chạy từ hướng dẫn này.

Không dựng lại những public effect bị thiếu trong backup bằng cách chạy campaign
lại. Trước tiên đối soát bài/bình luận đã gửi và receipt từ nguồn có thẩm quyền.

## Khi có lỗi

Thu tối thiểu:

- Version/executable thực sự đang chạy, hệ điều hành và cách cài.
- Thời điểm, thao tác vừa bấm, operation/campaign/request ID.
- Số/alias máy, owner hiện tại, package/version/locale, mã lỗi và giai đoạn.
- Log backend hoặc trace được export qua app; ảnh/XML phải được xử lý dữ liệu riêng
  trước khi chia sẻ. Không xuất token, password hoặc DB vận hành vào report.

Phân biệt **không có ACK**, **chưa được nhận**, **đang xử lý**, **đã gửi nhưng chưa xác
minh**, **đã xác minh nhưng nợ Sheet**. Cùng request ID dùng để đọc lại trạng thái;
không tạo ID mới chỉ để vượt pending. Không kill ADB, xóa intent/Sheet lock, reinstall
hoặc restart phone đang upload để thử vận may. Tra [Publish recovery](publish.md).
