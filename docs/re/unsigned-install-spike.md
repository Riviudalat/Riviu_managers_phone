# Spike: cài IPA chưa-ký (TrollRestore) — feasibility & thiết kế an toàn

> **Trạng thái: SPIKE / FEASIBILITY.** Không có đường chạy nào chạm thiết bị thật.
> Thao tác restore-based install **bị vô hiệu hoá bằng hằng số biên dịch**
> (`UNSIGNED_INSTALL_ENABLED = false`) và luồng lệnh trả lỗi trước khi gọi bất kỳ
> công cụ nào. Tài liệu này ghi lại khảo sát và thiết kế gate an toàn để một pha
> sau (khi có phần cứng thử nghiệm) có thể bật một cách có kiểm soát.

## 1. Bối cảnh

RouterMMO iOS bundle `TrollRestore.exe`, `PersistenceHelper_Embedded`,
`ideviceinstaller.exe` và Anisette provisioning để cài IPA **chưa-ký** hàng loạt.
Riviu hiện dùng mô hình **signed-agent**: một `RiviuAgent.ipa` đã ký (WDA-based)
với SHA-256 cố định (AGENTS.md §3.15), cài qua `InstallationProxyService` của
pymobiledevice3. Cài chưa-ký theo kiểu restore là một mô hình **khác hẳn** và rủi
ro hơn nhiều bậc.

## 2. TrollRestore hoạt động thế nào (tóm tắt)

TrollRestore không phải một "installer" thông thường. Nó:

1. Tạo một backup Mobilebackup2 tối giản của thiết bị.
2. Chèn/đổi một binary hệ thống (thường là một daemon như `netmuxd`/persistence
   helper) trong backup đó.
3. **Restore** backup đã sửa trở lại thiết bị, lợi dụng việc restore không kiểm
   tra chữ ký của các file trong backup như khi cài app bình thường, để đặt được
   một helper có quyền chạy binary chưa-ký.

Điểm mấu chốt: **đây là một thao tác restore**, không phải install. Nó ghi đè
trạng thái hệ thống. Nếu sai model/iOS, gián đoạn giữa chừng, hoặc helper không
tương thích → có thể để thiết bị ở trạng thái không boot được (brick mềm), cần
khôi phục qua Recovery/DFU.

## 3. Ma trận hỗ trợ (cần xác minh trên phần cứng thật)

TrollRestore phụ thuộc chặt vào phiên bản iOS và dòng máy. Trước khi bật, **phải**
xác minh trên một máy thử (không phải máy production) cho từng cặp (model, iOS)
trong fleet:

| Hạng mục | Ghi chú |
| --- | --- |
| Dải iOS | TrollRestore lịch sử hỗ trợ ~iOS 15.0–16.x và một phần 17.0; các bản vá sau đó của Apple bịt lỗ hổng. **Không giả định**; kiểm tra bản TrollRestore cụ thể được bundle. |
| Model | Khác nhau theo A-chip/SoC; một số cặp có helper riêng. |
| Trạng thái Setup | Một số biến thể yêu cầu thiết bị chưa qua/đã qua Setup Assistant. |
| Passcode | Thường yêu cầu tắt passcode trước khi restore. |

> Kết luận feasibility: **không thể xác nhận trên máy dev này** (không có iPhone
> thật + không có Mac live gate). Đây là lý do pha này chỉ dựng khung, không chạy.

## 4. Thiết kế gate an toàn (đã encode trong mã, ở trạng thái bất hoạt)

Bốn tầng bảo vệ, theo đúng thứ tự trong AGENTS.md và cam kết với người dùng:

1. **Capability typed, mặc định TẮT.** Hằng số biên dịch
   `commands::UNSIGNED_INSTALL_ENABLED = false`. Khi tắt, lệnh trả
   `CommandError::code("UnsignedInstallDisabled", …)` **trước** mọi thao tác.
   Bật = sửa mã có chủ đích, không phải toggle runtime — an toàn nhất cho spike.
2. **Backup-first bắt buộc.** Lệnh yêu cầu một thư mục backup đã tồn tại
   (tái dùng Pha 3 `backup_device` để tạo trước). Không có backup hợp lệ →
   từ chối. Restore-based install luôn phải có đường lùi.
3. **Cô lập khỏi agent production.** Luồng này **không bao giờ** ghi đè
   `sidecars/wda/RiviuAgent.ipa` hay manifest; SHA-256 production giữ nguyên
   (AGENTS.md §3.15). Công cụ TrollRestore/ideviceinstaller sẽ được bundle qua
   resources riêng ở pha sau, gọi qua `process_tree::background_command`
   (CREATE_NO_WINDOW + Job Object) — **chưa** được thêm ở pha spike này.
4. **Xác nhận rõ ràng + telemetry.** UI (khi bật) phải cảnh báo brick + yêu cầu
   xác nhận; mọi lần chạy ghi log đầy đủ.

Ngay cả khi tầng (1) bị bật nhầm, đường thực thi thật **vẫn chưa được nối** —
lệnh trả `CommandError::code("UnsignedInstallSpike", …)` sau khi qua các gate,
nên không có công cụ nào được gọi và không thiết bị nào bị chạm.

## 5. Việc còn lại cho pha thực thi (khi có phần cứng thử)

- [ ] Xác minh ma trận §3 trên ≥1 máy thử cho mỗi (model, iOS) của fleet.
- [ ] Bundle TrollRestore/ideviceinstaller qua `scripts/build_desktop_sidecar.py`
      hoặc resources riêng, có hash cố định + kiểm tra toàn vẹn.
- [ ] Nối `run_unsigned_install` thật qua `process_tree::background_command`,
      sau `backup_device` bắt buộc.
- [ ] UI: panel cảnh báo, mặc định ẩn, xác nhận hai bước.
- [ ] Chạy end-to-end trên máy thử, có đường khôi phục DFU sẵn sàng.
- [ ] Chỉ sau khi ổn định mới cân nhắc mở cho máy production.

## 6. Khuyến nghị

Giữ nguyên trạng thái spike (gate TẮT) cho đến khi có quy trình thử nghiệm phần
cứng chuyên dụng. Lợi ích của cài chưa-ký (không cần Apple ID/ký lại) không đáng
đánh đổi rủi ro brick fleet nếu chưa xác minh kỹ ma trận hỗ trợ.
