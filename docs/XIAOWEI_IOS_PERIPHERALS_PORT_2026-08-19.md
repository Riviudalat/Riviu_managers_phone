# Cổng tính năng xiaowei iPhone + Ngoại vi → Riviu (19/08/2026)

Phân tích hai báo cáo khảo sát và đối chiếu từng tính năng với hiện trạng Riviu, kèm quyết
định port. Nguồn:

- `C:\Users\cattfan\Documents\All\xiaowei-explore-iphone\report-VI.md` — **效卫苹果投屏签名版 3.3.41**
  (điều khiển iPhone hàng loạt: Facebook idb + WireGuard/wintun + BLE + AirPlay + ký app).
- `C:\Users\cattfan\Documents\All\xiaowei-explore-peripherals\report-VI.md` — **效卫苹果投屏外设版 2.3.51**
  (điều khiển bằng ngoại vi vật lý: HID keyboard/mouse, gamepad/joystick, phím macro, USB
  relay, + FFmpeg ghi hình).

**Nguyên tắc (đã chốt với người vận hành):** giữ stack iOS của Riviu (pymobiledevice3 + WDA +
`riviu-signing`) — đã chạy và chuẩn hơn; idb chỉ là tham chiếu. Phần lớn tính năng đi qua trait
`UiSession`/`DeviceDriver` nên tự chạy cả iOS lẫn Android. Chỉ port thứ đặc thù. Tầng cần phần
cứng: dựng **mã hoàn chỉnh, gate compile/test tại chỗ**, để nghiệm thu thật cho fleet.

---

## A. Báo cáo iPhone (report-VI.md)

| Tính năng xiaowei | Cơ chế xiaowei | Trạng thái Riviu | Ghi chú |
|---|---|---|---|
| Cầu tự động hoá iOS ("adb của iPhone") | Facebook **idb** + usbmux + xctest/testmanagerd | **Đã có (tốt hơn)** — `crates/ios-driver` dùng **pymobiledevice3 + WebDriverAgent** (`pmd.rs`, `wda.rs`). | idb là tham chiếu; KHÔNG thay stack. |
| Ký ứng dụng đồng hành ("签名版") | dịch vụ ký của xiaowei | **Đã có** — crate `riviu-signing` + `resign_wda`/`bulk_resign_wda`. | |
| **Khoá/Mở khoá màn hình iPhone hàng loạt** (`useIphoneLockScreen`) | WDA/pmd3 | **XONG phiên này** — `UiSession::set_locked` → iOS `WdaClient::lock`/`unlock` (`/wda/lock`,`/wda/unlock`); lệnh `set_screen_locked`; nút Khoá/Mở khoá trong "Thao tác nhanh". Cross-platform (Android `KEYCODE_SLEEP`/`WAKEUP`). | Máy có passcode dừng ở màn khoá — không vượt khoá (trung thực). |
| Chạm/vuốt/gõ/phím trên iPhone | xctest gestures | **Đã có + mở rộng** — mọi cử chỉ nhóm (A1–A9) đi qua `UiSession` nên chạy iOS. | Delay/offset A1 áp cả iOS. |
| Thu màn hình | AirPlay hoặc idb | **Đã có (một phần)** — Riviu thu qua WDA/MJPEG + pipeline WebCodecs. **AirPlay capture: CHƯA** (xem dưới). | |
| **AirPlay screen capture** (`AutoAirplay`/`CollectionPosition`/`ChangeMirrorName`) | iPhone mirror sang AirPlay receiver trên PC | **INFRA-GATED — chưa dựng.** | Cần **AirPlay receiver server** (mDNS `_airplay._tcp`/`_raop._tcp` + fairplay handshake + H.264/AAC depacketize). Khối lớn; giá trị bổ trợ vì WDA capture đã chạy. Kế hoạch: crate riêng `airplay-receiver`, phát khung vào `ViewHub` như một nguồn thay thế minicap/scrcpy. |
| Hầm mạng tới iPhone | **WireGuard/wintun** (usbmux tunnel) | **Đã có tương đương** — `pmd.rs` lập tunnel usbmux/lockdown của pymobiledevice3. | Không cần wintun; đường tunnel của pmd3 đủ. |
| **BLE discovery/control** | Bluetooth LE (IRK/pairing trong `selfIdentity.plist`) | **INFRA-GATED — chưa dựng.** | Cần crate `btleplug` (WinRT BLE) + thiết bị BLE thật. Niche theo chính báo cáo; để sau cùng. Kế hoạch: module `ble.rs` quét/aggregate quảng bá, phơi qua lệnh `ble_scan`. |
| Cloud phone / licensing / brand | hạ tầng xiaowei | **BỎ (ngoài phạm vi, đã chốt).** | |

## B. Báo cáo Ngoại vi (report-VI.md)

| Tính năng xiaowei | Cơ chế xiaowei | Trạng thái Riviu | Ghi chú |
|---|---|---|---|
| **USB relay** — bật/ngắt nguồn, **reboot cứng** | bo relay CH340 (giao thức "LCUS" 4-byte 9600 8N1) | **XONG phiên này (mã đầy đủ).** `peripherals.rs`: `encode_lcus(channel,on)` (checksum, đã test theo datasheet) + serial I/O (`serialport`) + lệnh `list_serial_ports`/`relay_set_channel`/`relay_pulse_channel` (`energize` = nhấn nút vs ngắt nguồn, hold clamp 50–10000ms). UI tab "Ngoại vi". | Trực tiếp cứu "máy kẹt ở app" (xem `nurture-fleet-state`). Nghiệm thu: cắm bo relay. |
| **Gamepad/joystick → điều khiển nhóm** | đọc controller, map nút → thao tác | **XONG phiên này (mã đầy đủ, chạy được qua Web Gamepad API).** `peripheralMap.ts` (thuần, 10 test): `defaultGamepadBindings` (A→Home, B→Back, X→Đa nhiệm, D-pad→vuốt theo lưới tham chiếu 1000×1000), `resolveButtonAction`, `risingEdges` (bấm 1 lần/nút). UI đọc tay cầm bằng **Web Gamepad API** (WebView2 hỗ trợ sẵn, không cần crate/driver), fan-out qua `group_input` (backend scale toạ độ per-máy). | Không cần `gilrs`. Bind toạ độ/macro tuỳ biến: type `PeripheralAction` đã hỗ trợ; default dùng phím + vuốt (không cần calibrate). |
| **HID keyboard/mouse routing** | định tuyến phím/chuột tới từng máy | **Đã có tương đương + khung** — bàn phím PC gõ thẳng qua overlay/`type_text`; phím phần cứng qua tab "Thao tác nhanh"; định tuyến chung dùng chính `peripheralMap` (button→action→fleet). | Đọc HID thô rời (không phải bàn phím hệ điều hành) cần `hidapi` + thiết bị — hiếm dùng; khung mapping đã sẵn. |
| **Phím macro (宏)** | phím vật lý kích tổ hợp thao tác | **Đã có (A8) + có thể bind.** Engine macro ghi/phát nhóm đã xong (`macro.ts`/`macroStore.ts`/tab Macro). `PeripheralAction::macro` cho phép gán nút→macro. | Kích bằng phím vật lý rời cần lớp đọc HID; nội dung macro đã đầy đủ. |
| **FFmpeg ghi hình/chụp màn hình** | FFmpeg 6.0 | **Đã có tương đương** — pipeline video WebCodecs + `screenshot`/`save_view_snapshot`. | Không bundle FFmpeg (dep khổng lồ); năng lực cốt lõi đã có. |
| Cloud / licensing / brand | hạ tầng xiaowei | **BỎ (ngoài phạm vi).** | |

---

## Tóm tắt phiên 19/08/2026

**Đã port thành mã hoàn chỉnh + gate tại chỗ:** iOS lock/unlock (A-report), USB relay + gamepad
bridge + mapping HID/macro (B-report), cùng tầng root C-lõi, A7 box-select, B Local API, A1
jitter live-drag. Gate: `cargo check`/clippy `-D warnings`/`cargo fmt` sạch; Rust core +
android(164) + ios(143) + app-lib(150, gồm relay+local_api) test pass; frontend **354** test;
tsc/oxlint sạch.

**Còn INFRA-GATED (cần hạ tầng/phần cứng, không dựng-verify tại chỗ được, đã nêu kế hoạch cụ
thể ở trên):** AirPlay receiver capture (iOS), BLE (iOS), và nghiệm thu máy thật cho relay/
gamepad/lock/root. Các tầng root sâu (xwdb Magisk adbd-root, HID/AOA safe-mode APK,
FreeReflection A14+) vẫn cần fleet Magisk + build AOSP/NDK.
