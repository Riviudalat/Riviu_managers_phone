# Các bước của vòng sửa lỗi

Ngân sách là trần để nhận ra mình đang kẹt, không phải mục tiêu. Vượt trần thì ghi lý do và
quyết định tiếp hay dừng; không âm thầm kéo dài. Thời gian build đo trên máy dev chính
(Windows, 2026-10-09..11): 3–12 phút mỗi lần build native debug.

Lệnh đầy đủ ở [README bộ script](../../../../scripts/device-acceptance/README.md). `<r>` là
nhãn vòng (vd. `r7`); mọi đầu ra nằm dưới `target/device-acceptance/`.

## 1. Yêu cầu → spec (10 phút)

- Diễn lại yêu cầu thành một kết quả kiểm được và phạm vi file.
- Tìm bằng chứng đầu vào (log, screenshot, serial, thời điểm) và ghi đường dẫn vào spec.
- Viết spec theo [mẫu](../../riviu-orca-coordinator/references/spec-template.md). Việc nhỏ
  một file mà coordinator tự sửa nhanh hơn thì không cần worker.

## 2. Worker (theo worker; coordinator 10 phút)

[Vòng đời worker](../../riviu-orca-coordinator/references/lifecycle.md): start → chờ/ack →
đọc worker_done → cherry-pick → release → dọn worktree vào Recycle Bin → đưa "Bài học cho
docs" vào owner doc.

## 3. Cổng tích hợp trên main (15–30 phút)

Theo [Cổng theo thay đổi](../../../../docs/development/testing.md#cổng-theo-thay-đổi): cổng
hẹp của phần đã đổi trước, rồi cổng của crate/frontend bị chạm. Cargo chạy tuần tự. Đỏ vì
thay đổi này: quay lại bước 2 với spec sửa. Đỏ vì nợ cũ: ghi lại, hỏi người dùng.

**Quyết định:** cần vòng máy (xem SKILL.md)? Không → bước 9 với bằng chứng đúng tầng.

## 4. Build native (3–12 phút)

```powershell
npm run build --prefix apps/desktop
py -3.12 -I scripts/device-acceptance/build-native.py <r>
```

Cần `exitCode 0` và `sourceStable true`. `sourceStable false` (exit 3): ai đó đã sửa source
trong lúc build; không dùng exe, build lại với nhãn mới khi source đứng yên.

## 5. Đổi controller (5 phút)

```powershell
node scripts/device-acceptance/close.mjs end-<r-1> <campaignId,...>
py -3.12 -I scripts/device-acceptance/launch.py <r> <scope.json> --build <r>
node scripts/device-acceptance/ipc.mjs list_devices
```

- `close.mjs` từ chối khi publish còn compose/transfer, còn owner khác `script` hay Nuôi còn
  chạy: chờ việc đó xong, không ép. Exit 2 = pid chưa thoát: chờ, xem log, không kill.
- Scope publish: `deviceIds` phải bằng `--udids` của runner đúng thứ tự
  ([publish acceptance](../../../../docs/development/publish-acceptance.md)).

## 6. Preflight (5–10 phút)

```powershell
node scripts/device-acceptance/preflight.mjs <r> <scope.json>
```

Máy `skipped` vẫn ở mẫu số. Sửa baseline (`--apply-baseline`), dọn máy quá tải
(`--cleanup-overloaded`) hay khôi phục helper (`--helper-recover <serial>`) chỉ khi đã được
duyệt cho vòng này; sau đó preflight lại với nhãn mới. Máy quá tải dai dẳng: theo
[sức khỏe máy](../../../../docs/agents/09-fleet-android.md#sức-khỏe-máy-trước-code).

## 7. Chạy và theo dõi

| Chế độ | Lệnh | Ngân sách |
|---|---|---|
| Nuôi | `run-nurture.mjs --config … --out …` | `durationMinutes` + 5 phút |
| Đăng bài | `inspect-source.mjs`, rồi `run-publish.py <label> --template …` | observe ≤ 90 phút |
| Tương tác | `run-interaction.mjs <targets> --config … --out … --publish-dir …` | `maxWatchMinutes` |

Theo dõi song song: `pubwatch.py`, `rotwatch.sh`, `status.mjs`. Dòng hỏng chỉ retry bằng
`retry-publish.mjs` (failedBeforeDispatch) hoặc `retry-interaction.mjs` (lỗi trước effect).
Một máy dừng hay kẹt: chụp màn hình trước khi chạm vào nó
([testing](../../../../docs/development/testing.md#phân-tầng-bằng-chứng)); không chạy
`uiautomator dump` ngoài trong khi controller chạy.

Đếm `verified`/`failed`/`skipped`/`uncertain` theo serial. Không replay `uncertain`.

## 8. Kiểm UI thật (5 phút)

```powershell
$env:SIZES = '1280x720,1366x768,1920x1080'; node scripts/device-acceptance/native-ui-shots.mjs target/device-acceptance/<r>/ui
```

Xem từng ảnh; đối chiếu [UI contract](../../../../docs/ui-reference-matrix.md). Số canvas ổn
định nghĩa là tile live không bị dựng lại liên tục.

## 9. Báo cáo (10 phút)

Tiếng Việt, cho người vận hành:

- đã sửa gì (commit trên main), cổng nào xanh;
- bảng kết quả theo máy: trạng thái, link bài chuẩn khi `verified`, lý do khi
  `skipped`/`failed`, việc còn `uncertain`;
- đường dẫn bằng chứng dưới `target/device-acceptance/<r>/`;
- phần chưa kiểm và bước tiếp theo đề xuất.

Push, release hay đăng bản cài chỉ sau khi người dùng duyệt.

## Điều kiện dừng

- Hai vòng sửa không đổi kết quả của cùng một lỗi → dừng vòng, điều tra lớp lỗi.
- Bất kỳ `uncertain` nào → observe tới khi rõ hoặc người dùng quyết; không chạy lại máy đó.
- Controller không thoát, adb mất máy hàng loạt, hay cần reboot máy còn effect chưa rõ → hỏi.
