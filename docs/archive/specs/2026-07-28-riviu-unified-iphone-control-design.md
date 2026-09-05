# Riviu Unified iPhone Control Design

**Status:** Huong thiet ke da duoc chot ngay 28/07/2026.

## 1. Muc tieu

Riviu cung cap mot trai nghiem dieu khien duy nhat cho tung iPhone va ca fleet.
Nguoi van han chi thay mot nut Agent, mot trang dieu khien va mot trang thai thiet
bi. Ben duoi, san pham dung dung kenh phu hop cho tung loai tac vu:

- `Riviu Agent` tren iPhone cho man hinh, gesture, text, session va MJPEG.
- `Riviu Device Bridge` tren desktop cho cac dich vu USB cua iOS.
- `Riviu MDM` duoc giu thanh phase sau cho cac lenh quan tri supervised.

Hai lop dau la pham vi trien khai hien tai. MDM duoc ghi ro trong kien truc de
khong phai dap lai he thong khi can them sau nay.

## 2. Cac huong da can nhac

### A. Mot IPA lam tat ca

Uu diem la mo hinh trien khai de hieu. Diem yeu la sandbox cua iOS va XCUITest
khong phai noi phu hop cho backup, install service, syslog, pairing hay giao thuc
RSD cua iOS moi. Huong nay gan agent qua chat vao mot doi iOS va lam vong doi
session de hong theo.

### B. Desktop-only qua USB

Uu diem la app management, backup va diagnostics nam o dung kenh lockdown/DVT.
Diem yeu la khong co text channel da live-confirm voi TikTok, gesture/UI state kem
on dinh va MJPEG khong co cung lifecycle voi automation session.

### C. Mot san pham, hai engine chuyen trach - duoc chon

Riviu Agent xu ly UI automation; Device Bridge xu ly iOS device services. Hai
engine cung phoi hop qua mot `DeviceController`, mot capability model va mot lock
theo UDID. Day la huong co blast radius nho nhat, giu duoc comment hien tai va co
cho cho iOS/RSD moi.

## 3. Pham vi quyen van han

### 3.1 Man hinh va input

- MJPEG stream, screenshot, screen recording theo phien.
- Tap, double tap, long press, swipe, drag va multi-touch khi agent ho tro.
- Nhap Unicode, clipboard get/set, Home, lock/unlock, shake.
- Nut volume, orientation, appearance va dismiss system alert.
- Lay active app, UI source co gioi han va tap theo element khi app expose tree.
- Dieu khien mot may hoac dong bo mot nhom may.

### 3.2 Ung dung

- Liet ke app da cai va metadata co the doc qua installation service.
- Cai IPA, go app, launch, activate, terminate va mo deeplink.
- Quan ly va nang cap dung Riviu Agent; xac minh bundle, version, protocol, auth
  va MJPEG thay vi chi kiem tra bundle co ton tai.
- Rollback ve artifact agent da biet tot khi ban nang cap khong qua preflight.

### 3.3 File, media va clipboard

- Day anh/video vao Photos qua route agent da co.
- Day file vao app container khi app cho phep File Sharing/House Arrest.
- Doc/ghi clipboard qua agent va xac minh foreground requirement.
- Theo doi tien do, checksum, huy transfer va retry theo chunk.

### 3.4 Trang thai va dieu khien thiet bi

- Ten may, model, iOS build, pin, dung luong, connection type va Developer Mode.
- Reboot, shutdown khi device service ho tro, lock/unlock va cac nut he thong.
- Simulated location, locale/timezone/profile hien thi khi kenh hien tai cho phep.
- Pairing/trust/DDI diagnostics va health cua tung transport.

### 3.5 Diagnostics, backup va fleet

- Syslog tail/download, request trace, agent/relay/stream health va artifact dump.
- Backup/restore qua mobilebackup2 voi progress, cancel va kiem tra dung luong.
- Queue theo UDID, group action, per-device concurrency lock va audit log.
- Capability matrix tren UI de lenh chi hien khi thiet bi xac nhan co ho tro.

## 4. Pham vi MDM de phase sau

`AdminControl` duoc dinh nghia nhu mot interface rong trong kien truc, nhung chua
co implementation o dot nay. Phase MDM se chua:

- Device Lock, remote erase, restart/shutdown qua MDM.
- Clear passcode bang escrowed `UnlockToken`.
- Supervision, restrictions, configuration profiles va OS update policy.
- Automated Device Enrollment, Activation Lock escrow/bypass code va managed app.

Mat khau Apple Account va passcode hien tai van do iOS giu. Control plane quan ly
qua lenh rotate/clear/restriction va escrow token, khong dua bi mat do vao Riviu DB.

## 5. Kien truc

```text
React UI
   |
Tauri commands
   |
DeviceController (capabilities + per-UDID lock + audit)
   |------------------------------|
   |                              |
UnifiedAgentManager           DeviceBridge
   |                              |
Riviu Agent IPA              usbmux/lockdown/DVT/RSD
   |- UI input                    |- app install/list/remove
   |- trusted text                |- file/media services
   |- MJPEG                       |- backup/restore
   `- clipboard/location          `- info/log/reboot/pairing

Deferred: AdminControl -> Riviu MDM -> APNs/device-management channel
```

Khong dua Device Bridge vao IPA. Khong dua UI-specific session lifecycle vao
lockdown sidecar. `DeviceController` la noi duy nhat phoi hop hai kenh va bao dam
thu tu session-truoc-stream.

## 6. Capability va protocol

Moi device co mot snapshot capability bat bien trong mot lan refresh:

```json
{
  "udid": "DEVICE_ID",
  "iosVersion": "16.7.15",
  "transport": "legacy-usbmux",
  "agent": {
    "bundleId": "com.riviu.managersphone.agent",
    "agentVersion": "1.0.0",
    "protocolVersion": 1,
    "features": ["stream", "tap", "swipe", "text", "clipboard"]
  },
  "features": {
    "apps": true,
    "mediaPush": true,
    "backup": true,
    "simulatedLocation": true,
    "admin": false
  }
}
```

Agent health phai tra `agentVersion`, `protocolVersion` va `features`. Desktop
khong suy tinh nang tu model iPhone hoac chi tu so iOS. Lenh khong co capability
tra ve loi co type `UnsupportedCapability` va khong thu fallback mu.

## 7. Vong doi mot agent duy nhat

1. Desktop scan thiet bi va tao transport adapter theo capability probe.
2. Agent manager doc OS credential, kiem tra IPA manifest va route auth bao ve.
3. Neu agent thieu/sai version, manager cai artifact tuong thich va giu artifact
   cu de rollback.
4. Manager launch agent voi control port, MJPEG port va token qua environment.
5. Session duoc tao truoc, sau do moi mo MJPEG stream.
6. Comment-enabled job dung fresh text session sau khi TikTok foreground.
7. Loi session chi thay session; loi transport da phan loai moi recycle relay va
   agent. Moi transition cap nhat capability/health tren UI.

Stock `Riviumanagersphone.ipa` khong con la agent mac dinh trong product flow. No
duoc giu tam thoi nhu rollback artifact trong qua trinh chuyen sang mot
`RiviuAgent.ipa` duy nhat.

## 8. Ho tro iPhone va iOS moi

- `LegacyUsbmuxTransport` phuc vu fleet iOS 15/16 hien tai.
- `RsdTransport` phuc vu cac device/iOS yeu cau Remote Service Discovery/tunnel.
- Transport duoc chon bang probe, khong bang danh sach model hard-code.
- Agent release manifest anh xa iOS/Xcode range sang artifact da test va co
  checksum. Moi lan update giu ban N-1 de rollback.
- Protocol agent versioned; desktop chi tu dong update khi major protocol tuong
  thich, con major mismatch phai hien trang thai ro rang.
- CI co contract test cho ca hai transport; live gate dung it nhat mot device
  legacy va mot device iOS moi truoc khi promote agent.

## 9. UI

`FocusStream` van la man hinh dieu khien chinh, duoc chia thanh tab gon:

- **Screen:** stream va toolbar icon cho Home, lock, volume, orientation,
  screenshot, record va reboot.
- **Apps:** list/search, install, launch, terminate va uninstall.
- **Files & Media:** upload queue, Photos va app containers.
- **Device:** thong tin, pin, storage, location/appearance va agent status.
- **Diagnostics:** syslog, trace, pairing/DDI va export support bundle.
- **Backup:** tao, liet ke, restore va progress.

Lenh destructive nhu uninstall, reboot, shutdown va restore dung confirm co ten
device. Group action hien so device bi anh huong truoc khi submit.

## 10. Bao mat va audit

- Agent token, Apple credential va backup key nam trong OS credential store.
- Token khong xuat hien trong argv, log, trace, DB hoac UI.
- Moi lenh write co `operationId`, actor, UDID, capability, start/end va outcome.
- Mot async lock theo UDID bao ve agent, relay, stream, app service va backup.
- File/IPA duoc xac minh checksum truoc install; backup restore kiem tra manifest
  va dung luong truoc khi bat dau.

## 11. Xu ly loi

- Loi co type: `Capability`, `AgentAuth`, `AgentProtocol`, `Session`, `Transport`,
  `DeviceLocked`, `Pairing`, `Transfer`, `Backup` va `Cancelled`.
- Retry chi ap dung cho thao tac idempotent hoac loi chac chan chua den device.
- Transfer va backup co cancellation token; UI khong bao thanh cong truoc ACK va
  verification tu kenh doc.
- Agent update that bai se rollback artifact cu, restore relay/session/stream theo
  dung thu tu va ghi audit.

## 12. Milestone trien khai

1. **Unified Agent Runtime:** cau hinh keyring, mot Agent status, cai/verify dung
   IPA, bo fallback stock im lang, preflight text va fresh-session recovery.
2. **Capability Control Plane:** types, probes, typed errors, audit va Tauri API.
3. **Screen & System Controls:** clipboard, lock, buttons, orientation, appearance,
   location, active app va toolbar UI.
4. **Apps & Transfers:** app inventory/deeplink, media push va app-container file.
5. **Backup & Diagnostics:** mobilebackup2, progress/cancel, syslog va support bundle.
6. **Future-iOS Adapter:** RSD transport, agent compatibility manifest va rollback.
7. **Deferred MDM:** mot spec/plan rieng khi fleet san sang supervision/enrollment.

Moi milestone phai ship phan mem chay duoc va co test rieng; khong gom tat ca vao
mot thay doi khong the review.

## 13. Kiem thu va tieu chi hoan thanh

- Unit test cho capability merge, route payload, retry policy va state machine.
- Contract test sidecar cho legacy/RSD fixtures va moi subcommand.
- Mock integration test desktop -> controller -> agent/bridge.
- Live test tren iPhone 8 hien tai: stream, gesture, Unicode text/comment, app
  install/list/launch/remove, clipboard, media push, reboot va backup/restore.
- Live compatibility gate tren mot iPhone/iOS moi truoc khi bat RSD cho fleet.
- Tat ca write action chi tang counter/bao thanh cong sau read-back verification.
- Khi agent update hong, device quay lai artifact cu va stream dieu khien duoc.

Phase 1-2 duoc xem la dat khi nguoi dung co the van han day du cac nhom Screen,
Apps, Files & Media, Device, Diagnostics va Backup tu mot app Riviu, khong can mo
RouterMMO hay dat environment bang tay. Cac capability MDM hien `Deferred` thay vi
gia lap bang tap toa do.
