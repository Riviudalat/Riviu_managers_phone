---
name: riviu-device-acceptance
description: >
  Dùng khi nghiệm thu một candidate Riviu trên điện thoại thật (Nuôi, Tương tác, Đăng bài,
  Flow) theo vòng preflight → chạy → theo dõi → thu bằng chứng → khôi phục → ghi kết quả.
  Kiểm từng máy trước khi chạy, một controller duy nhất, không replay Post/Send/Sheet chưa
  rõ, chụp màn hình/hierarchy trước khi phiên đóng app. Không tự mở controller, kill
  adb-server khi controller chạy, reboot máy còn effect chưa rõ hay chạy tương tác công khai
  ngoài phạm vi duyệt.
metadata:
  version: "1.3.0"
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
- Không mở app thứ hai, GenFarmer hay Appium cạnh nó; không force-drop lease. adb-server treo
  thì controller tự khởi động lại có giới hạn (HX-ADB, nhật ký `adb.server.restart*`); không
  kill/restart tay khi controller đang chạy, chỉ khi đã đóng controller.
- Không chạy `uiautomator dump` hay probe ngoài trong khi controller chạy: nó tranh
  UiAutomation với session của Riviu. Screencap thì được.

## 2. Preflight từng máy

Trước khi mở controller: `sweep-media.py <label> <scope.json> --apply` dọn album/stage
`riviu-*` của dòng đã kết thúc. Bộ quét của controller tắt dưới dev-acceptance scope; r112
(14/10) mỗi máy còn 16–31 album và bước chọn album quá hạn trên 3/20 máy.

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

## 7. Ca đã gặp (tuần 7–13/10/2026) và cách tránh

Mỗi mục dưới đây từng làm mất 20–60 phút. Đọc trước khi mở controller.

### Scope và report dir của runner

- **Preflight cần scope ở trạng thái inactive:** `campaignIds: []` và `capabilities` vắng
  (hoặc toàn `false`). Scope có `campaignIds`/`capabilities: true` của đợt trước thì preflight
  từ chối ngay ("cần campaignIds rỗng", "chỉ nhận capability false hoặc absent").
- **Controller ghim scope lúc mở.** Một lượt Start thất bại để lại scope active với requestId
  chết; sửa file về inactive **chưa đủ**, controller vẫn chặn ("file phạm vi … đã khác lúc mở
  controller"). Phải: sửa file → `close.mjs` → `launch.py` lại.
- **Mỗi lần chạy lại dùng `--report-dir` mới** (`publish`, `publish2`, …). File
  `preflight-request.json` của lần hỏng làm lần sau chết với "--dev-scope đã đổi sau
  preflight"; nếu phải dùng lại thư mục thì đưa file đó vào Recycle Bin.
- **Template bundle = đúng tập đã quét.** Mỗi máy một thư mục bundle riêng trong `--source`;
  một thư mục không gánh được hai máy (`bundleIdsMatchTemplate:false`). Chạy
  `inspect-source.mjs` hai lần: lần đầu lấy id, lần hai khớp.
- Submit trả `pendingDeadline` (exit 2) là bình thường. Observe chết với
  `page.evaluate: Target page … closed` chỉ là mất CDP, chiến dịch vẫn chạy: đọc DB
  (`publish_assignments`, `publish_sheet_outbox`, `publish_dispatch_jobs`) chỉ đọc
  (`?mode=ro`) để theo dõi, `py -3.12 -I -X utf8` để in tiếng Việt.
- IPC tới controller có thể treo >2 phút khi 30 máy đang soạn: bọc `timeout 90`; trạng thái
  thì lấy từ DB, không từ IPC.
- **Không chạy `run-publish.py` bên trong Monitor/watch có hạn.** Watch hết hạn (30 phút)
  giết cả cây tiến trình, kể cả runner đang ở giữa submit (14/10, r108: preflight 6 phút +
  submit bị giết, may là trước khi Start được gửi). Chạy runner bằng tác vụ nền độc lập;
  Monitor chỉ đọc file/DB.

### Khôi phục không chạm máy

- Dòng `uncertain`/`needsReview` (`processingWaitExhausted`, `clipboardUnchanged`…):
  `publish_recovery_capabilities` → `publish_resume_verification
  {assignmentId, confirmed:true, expectedRevision}`. Không bao giờ retry các dòng này.
- Dòng `failed_before_dispatch`: `retry-publish.mjs <campaign> <vị trí 1-based trong
  scope.deviceIds,…> <label> --scope … --out …`. Recovery tự thử 3 lần trước; chỉ retry tay
  khi job đã `compose:finished` mà dòng vẫn failed.
- Helper báo `recovery_required`/`HelperMaintenanceCleanupPending` sau khi controller khởi
  động lại, bàn phím mặc định là `com.riviu.agent/.RiviuIme`: từ `bc3139c2` maintenance tự
  trả bàn phím đã ghi. Dữ liệu cũ không có bản ghi: `adb -s <serial> shell ime set <bàn phím
  gốc>` (đội máy dùng `com.genfarmer.uiautomator/.AdbKeyboard`) → `agent_helper_maintenance_prepare`
  → `execute` (confirmed, qualifyReplacement); receipt `superseded` +
  `packageProcessAbsent:true` là xong.
- Máy kẹt `offline` sau reboot do Riviu: từ `6c5572fb` tự `adb -s <serial> reconnect`. Một máy
  offline không phải lý do `reconnect` toàn bộ hay `kill-server`.
- Popup mới sau Post (email updates, "Continue editing this post?"): chụp màn hình vào
  `target/apply-apk-20261009/`, giao worker có `riviu-tiktok-apk` thêm vào catalog
  (Shadow → Act khi có dump thật). Không tự bấm nút có cam kết.

### Tải khi chạy 30 máy

- 30 máy soạn bài cùng lúc, có stream hình trực tiếp: `/source` 5–18 s (bình thường <1 s),
  `surfaceflinger ≥60%` CPU, load average 7–8 trên SM-G955F; ~1/3 dòng hỏng lượt đầu
  (chọn ảnh/nhạc quá hạn) rồi recovery tự đăng được. Máy tính host không nghẽn (Riviu ~10%
  CPU). Khi chưa có cap đồng thời trong code: chạy theo đợt ≤10 máy hoặc chấp nhận một
  vòng retry.
- WebView của controller lên 3 GB sau vài giờ; đóng controller giữa các đợt bằng `close.mjs`.
- Chỉ một build Rust tại một thời điểm trên máy 32 GB khi controller đang chạy; build thứ hai
  bị hệ điều hành giết.
- Đợt tham chiếu: `fff151ba` 30/30, `a62c6b72` 29/30 lên Sheet
  (`target/apply-apk-20261009/OPEN-ISSUES-publish-20261013.md`).
