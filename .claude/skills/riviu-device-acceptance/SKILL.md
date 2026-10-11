---
name: riviu-device-acceptance
description: >
  Dùng khi nghiệm thu một candidate Riviu trên điện thoại thật (Nuôi, Tương tác, Đăng bài,
  Flow) theo vòng preflight → chạy → theo dõi → thu bằng chứng → khôi phục → ghi kết quả.
  Kiểm từng máy trước khi chạy, một controller duy nhất, không replay Post/Send/Sheet chưa
  rõ, chụp màn hình/hierarchy trước khi phiên đóng app. Không tự mở controller, kill
  adb-server, reboot máy còn effect chưa rõ hay chạy tương tác công khai ngoài phạm vi duyệt.
metadata:
  version: "1.1.0"
  origin: local
  project: riviu-manager
---

# Vòng nghiệm thu trên máy thật

Skill này chỉ là trình tự. Hợp đồng nằm ở owner docs; đọc chúng, đừng chép lại. Lệnh nằm ở
[bộ script nghiệm thu](../../../scripts/device-acceptance/README.md); mọi script có `--help`.
Chỉ chạy khi phạm vi máy/effect/controller đã được duyệt rõ. Lỗi lớp nào chưa rõ thì chuyển
sang `riviu-device-diagnostics`. Vòng sửa lỗi trọn gói (spec → worker → build → máy → báo cáo)
ở `riviu-fix-cycle`.

## 1. Một controller

- Đóng controller cũ: `close.mjs <label> <campaignId,...>`. Nó từ chối khi publish còn
  compose/transfer, còn owner khác `script` hay Nuôi còn chạy, rồi chờ pid thoát; exit 2 =
  chưa thoát: chờ, không kill.
- Mở controller mới: `launch.py <label> <scope.json> --build <build>`. Nó từ chối khi đã có
  `riviu*.exe`, cổng CDP bận hay exe khác bản ghi build, và backup DB chỉ đọc trước khi mở.
- Không mở app thứ hai, GenFarmer hay Appium cạnh nó; không kill/restart adb-server, không
  force-drop lease.
- Không chạy `uiautomator dump` hay probe ngoài trong khi controller chạy: nó tranh
  UiAutomation với session của Riviu. Screencap thì được.

## 2. Preflight từng máy

`preflight.mjs <label> <scope.json>` (chỉ đọc) ghi theo serial: adb `device`, auto-rotate
tắt, khóa màn hình tắt, tải máy, helper không còn bảo trì chờ, cửa sổ focus không phải
dialog sót. Thêm bằng tay:

- đúng package/version/locale TikTok;
- `top -b -n 1` và swap không lệch hẳn so với máy cùng đời
  ([sức khỏe máy](../../../docs/agents/09-fleet-android.md#sức-khỏe-máy-trước-code));
- không còn Post/Send/Sheet nào ở trạng thái chưa rõ trên máy đó.

Máy không đạt thì ghi `skipped` kèm lý do, không loại khỏi mẫu số. Sửa baseline, dọn máy hay
khôi phục helper chỉ qua cờ tường minh (`--apply-baseline`, `--cleanup-overloaded`,
`--helper-recover <serial>`) khi đã được duyệt, rồi preflight lại với nhãn mới.

## 3. Chạy và theo dõi

- Runner: `run-nurture.mjs`, `inspect-source.mjs` + `run-publish.py`
  ([publish acceptance](../../../docs/development/publish-acceptance.md),
  [runbook agent](../../../docs/agents/agent-runbook.md#nghiệm-thu-publish-và-bàn-giao)),
  `run-interaction.mjs`. Mỗi runner ghi intent trước effect; intent đã có thì chỉ theo dõi.
- Theo dõi: `pubwatch.py`, `rotwatch.sh`, `status.mjs`; UI thật: `native-ui-shots.mjs`.
- Theo dõi outcome, không theo ACK
  ([nguyên tắc](../../../docs/agents/07-nguyen-tac.md)). Post/Send/Sheet chưa rõ chỉ
  observe, không bao giờ replay. Retry chỉ bằng `retry-publish.mjs` (failedBeforeDispatch,
  chưa có effect intent) hoặc `retry-interaction.mjs` (lỗi trước effect).

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
