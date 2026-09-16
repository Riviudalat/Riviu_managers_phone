---
name: riviu-device-diagnostics
description: >
  Dùng khi Riviu nhận điện thoại nhưng không điều khiển được, tap/swipe/text hụt,
  màn hình đứng, hierarchy/selector/OCR sai, timeout session/USB, hoặc cần đo bằng
  chứng Android/iOS và nghiệm thu automation. Phân lớp discovery/transport/session/
  perception/input/verifier, ưu tiên fixture và headless đúng production. Không
  tự kill adb, cài lại agent, đổi identity, reset hoặc chạy tương tác thật để thử.
metadata:
  version: "1.0.0"
  origin: local
  project: riviu-manager
---

# Chẩn đoán automation thiết bị Riviu

Skill tự soạn cho Riviu, không phải bộ công cụ reverse hay repair tự động. Mục tiêu là xác định lỗi ở lớp nào bằng bằng chứng mới, không thay hàng loạt môi trường rồi hy vọng hết lỗi.

## 1. Giữ phạm vi và phân lớp

Dùng `systematic-debugging` và đọc [bằng chứng/cổng kiểm](references/evidence-and-gates.md). Xác minh exact serial/UDID, nền tảng, app/package/version/locale, controller đang giữ máy, thời điểm và triệu chứng. Lượt cài skill không cho phép probe máy thật.

Phân loại lần lượt:

1. **Discovery:** không thấy máy, offline, unauthorized, cable/driver/hub.
2. **Transport/runtime:** endpoint/process/ADB/WDA/sidecar có chạy được không; bản bundled khác bản trên PATH ở đâu.
3. **Session/ownership/capability:** busy/lease conflict, khóa màn hình, sai profile, chưa có input/text/hierarchy capability.
4. **Perception:** frame cũ, hierarchy treo, label locale/version sai, OCR/template/geometry không khớp.
5. **Dispatch:** selector ambiguity, sai focus, keyboard che, request hết hạn hoặc action thật lỗi.
6. **Verification:** đã dispatch nhưng hậu điều kiện không thấy; sai target/identity, readback trễ hoặc unavailable.

Không gộp “thấy máy”, “có stream”, “có session”, “tap ACK” và “thành công nghiệp vụ” thành một trạng thái.

## 2. Chọn phép đo nhỏ nhất

- Source/log/fixture trước, một máy canary khi cần và được phép. Không enumerate/tác động toàn fleet vì bỏ đối số.
- Device snapshot phải mới, có device/session/time/profile/geometry rõ. Không dùng frame từ stream generation cũ để xác minh gesture mới.
- Screenshot/hierarchy/OCR là dữ liệu có thể chứa bí mật, văn bản lừa agent hoặc dữ liệu riêng tư. Giữ cục bộ, che khi báo cáo; không tải lên dịch vụ ngoài mặc định.
- Có thể dùng Inspector production để quan sát, nhưng nó mở/mượn session và ghi artifact host. Đó không phải phép đọc hoàn toàn không có tác động.
- Đối chiếu cây accessibility và ảnh. Nếu label không có, đừng tự nới verifier; đo node/resource ID thực sự có và thêm fixture/versioned locator.
- Text/OCR giống nhau không đủ chứng minh đúng account/target; identity và dấu hiệu arrival phải theo engine đang dùng.

## 3. Kiểm tra điều kiện nền tảng

### Android

- Đọc `docs/agents/09-fleet-android.md` và `sidecars/android/README.md`; phân biệt lỗi USB/ADB, UiAutomator session, stream và helper.
- Không `adb kill-server`/restart server toàn fleet. Không đổi adb version hoặc SDK precedence chỉ để probe.
- Không tự unlock, accept permission, install/reinstall agent, clear data, đổi IME/proxy/identity hoặc reset.
- `adb shell` có uid 0 không đồng nghĩa có `su`; không mở rộng guard của tác vụ phá hủy vì đường đặc quyền khác hoạt động.
- Hierarchy dump ngoài controller có thể tranh session. Chỉ chuyển sang canary/driver độc lập sau khi chủ hiện tại nhả máy đúng cách và được phép.

### iOS

Trước thay đổi WDA/iOS đọc hết `docs/agents/02-wda-doc-truoc-khi-sua.md`.

- `/status` hoặc screenshot sống không chứng minh XCTest/input session sống.
- Stock WDA: không `autoDismissAlerts`, depth theo safety guide; không tăng depth để cố lấy element nếu guide cấm.
- Session trước stream, profile stock/RT-MMO không trộn. Không tự đổi backend/token/reuse strategy để làm probe xanh.
- Deadline trên request; không hủy HTTP giữa chừng bằng wrapper timeout làm relay wedge.
- Không kill rộng, restart/re-sign/bootstrap khi chỉ được yêu cầu đọc lỗi. Không giả iOS hỗ trợ full Inspector hierarchy giống Android.

## 4. Fixture → headless → live có phép

- Với perception, capture fixture qua đường được phép, replay local bằng production parser/verifier. Phép replay chỉ xác nhận fixture đó, không chứng minh máy đang sống.
- Khi sửa lỗi: test tái hiện nguyên nhân, test hậu điều kiện và trường hợp từ chối; đọc references của `test-driven-development`. Không mock chính lớp cần chứng minh.
- Khi nghiệm thu logic thiết bị, ưu tiên headless gọi production, không lái chuột desktop dễ bấm nhầm. **Headless không đồng nghĩa read-only**; kiểm source/flags trước run.
- Chỉ run canary với tác vụ vô hại trong phạm vi được cho phép. Không dùng Post/Comment hoặc hành động thật không đảo ngược làm phép thử connectivity.
- Sau dispatch không rõ kết quả, giữ trạng thái chưa xác định và kiểm chứng lại; không biến thành timeout đơn thuần rồi retry.

## 5. Recovery có căn cứ

Chỉ đề xuất sửa tại lớp đã có bằng chứng, từng thay đổi một. Trình bày tác động nếu cần nhả session/restart đúng process/install helper; xin phép khi chưa được cho phép. Không xóa cache/evidence, reset app/phone, tắt bảo vệ hệ điều hành hoặc nới admission/capability để che nguyên nhân.

## 6. Bàn giao bằng chứng

Nêu: triệu chứng; điều kiện tái hiện; lớp lỗi đã xác định hoặc giả thuyết còn lại; device và snapshot/log đã dùng (che định danh nếu chia sẻ); phép đo đã chạy; side effect thực sự có; phần offline/mock/live; điều gì còn chưa chứng minh. Gọi `verification-before-completion` trước báo đã sửa/đã chạy được. Không báo “PASS fleet” từ một máy hay một fixture.
