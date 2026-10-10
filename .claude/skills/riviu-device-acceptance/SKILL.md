---
name: riviu-device-acceptance
description: >
  Dùng khi nghiệm thu một candidate Riviu trên điện thoại thật (Nuôi, Tương tác, Đăng bài,
  Flow) theo vòng preflight → chạy → theo dõi → thu bằng chứng → khôi phục → ghi kết quả.
  Kiểm từng máy trước khi chạy, một controller duy nhất, không replay Post/Send/Sheet chưa
  rõ, chụp màn hình/hierarchy trước khi phiên đóng app. Không tự mở controller, kill
  adb-server, reboot máy còn effect chưa rõ hay chạy tương tác công khai ngoài phạm vi duyệt.
metadata:
  version: "1.0.0"
  origin: local
  project: riviu-manager
---

# Vòng nghiệm thu trên máy thật

Skill này chỉ là trình tự. Hợp đồng nằm ở owner docs; đọc chúng, đừng chép lại. Chỉ chạy
khi phạm vi máy/effect/controller đã được duyệt rõ. Lỗi lớp nào chưa rõ thì chuyển sang
`riviu-device-diagnostics`.

## 1. Preflight từng máy

Ghi theo serial, không theo số máy đoán:

- Đã kết nối, đúng controller đang giữ máy, đúng package/version/locale TikTok.
- Auto-rotate tắt, khóa màn hình tắt.
- `top -b -n 1` và swap (`/proc/meminfo`) không lệch hẳn so với máy cùng đời
  ([sức khỏe máy](../../../docs/agents/09-fleet-android.md#sức-khỏe-máy-trước-code)).
- Không còn Post/Send/Sheet nào ở trạng thái chưa rõ trên máy đó.

Máy không đạt thì ghi `skipped` kèm lý do, không loại khỏi mẫu số.

## 2. Một controller

- Chỉ một controller Riviu giữ USB; không mở app thứ hai, GenFarmer hay Appium cạnh nó.
- Không kill/restart adb-server, không force-drop lease.
- Không chạy `uiautomator dump` hay probe ngoài trong khi controller chạy: nó tranh
  UiAutomation với session của Riviu.

## 3. Chạy và theo dõi

- Chạy đúng lệnh/harness của owner doc
  ([publish acceptance](../../../docs/development/publish-acceptance.md),
  [runbook agent](../../../docs/agents/agent-runbook.md#nghiệm-thu-publish-và-bàn-giao)).
- Theo dõi outcome, không theo ACK
  ([nguyên tắc](../../../docs/agents/07-nguyen-tac.md)). Post/Send/Sheet chưa rõ chỉ
  observe, không bao giờ replay.

## 4. Khi dừng hoặc kẹt: bằng chứng trước

Chụp màn hình và hierarchy **trước** khi phiên đóng app hay bạn chạm vào máy
([testing](../../../docs/development/testing.md#phân-tầng-bằng-chứng)). Giữ log, serial,
thời điểm và bước đang chạy. Raw ảnh/dump giữ ngoài Git.

## 5. Khôi phục

- Helper hỏng: **Khôi phục helper** trên đúng ô máy
  ([thiết bị](../../../docs/operator/devices.md#riviu-helper-trên-android)).
- Một máy quá tải (root mù, tap bị nuốt, CPU/swap lệch máy cùng đời): reboot chỉ khi không
  còn effect chưa rõ, rồi kiểm lại auto-rotate/khóa màn hình và chạy Khôi phục helper trước
  job kế tiếp.
- Không cài lại agent, đổi identity hay reset ngoài phạm vi đã duyệt.

## 6. Ghi kết quả

Mỗi nghĩa vụ (máy × việc) đúng một trạng thái:

| Trạng thái | Khi nào |
|---|---|
| `verified` | Có proof theo owner doc (vd. link chuẩn + tài khoản + caption + giờ cho Đăng bài) |
| `failed` | Có bằng chứng effect không xảy ra hoặc sai |
| `skipped` | Không chạy, kèm lý do (preflight, blocked, ngoài phạm vi) |
| `uncertain` | Đã gửi nhưng chưa quan sát được kết quả; chỉ observe, không retry |

ACK, screenshot hay HTTP 200 đơn lẻ không đủ cho `verified`. Bàn giao ghi lệnh thật, exit,
serial-keyed outcome và phần chưa kiểm.
