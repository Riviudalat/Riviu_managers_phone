<!-- Sinh bởi scripts/build_agents_index.py. Không sửa tay. -->

# Chỉ mục tài liệu agent

`AGENTS.md` là cửa vào. [Kho tài liệu](../README.md) chia hướng dẫn hiện tại, lịch sử và bằng chứng.

## Đọc trước

- [Hướng dẫn tiếp nhận](agent-runbook.md): ranh giới thay đổi, cổng và bàn giao.
- [Vận hành](../operator-guide.md): đầu vào, kết quả và bước tiếp theo của 12 trang.
- [Phát triển](../developer-guide.md): chủ sở hữu, hợp đồng và lệnh kiểm tra.
- Đọc toàn bộ §2 trước mọi thay đổi WDA/iOS. Không suy rộng bằng chứng Android sang iOS.

Số mục là định danh vĩnh viễn. Trích bằng `AGENTS.md §x`, không bằng số dòng.
Nhật ký là bản ghi có ngày; mục mới hơn chỉ thay thế các kết luận mà nó nêu rõ.

## Mục tham chiếu (§1–§10)

| § | Nội dung | File |
|---|---|---|
| §1 | [1. Dự án này là gì](01-du-an-va-ten.md#1-dự-án-này-là-gì) | `01-du-an-va-ten.md` |
| §2 | [2. Đọc mục này TRƯỚC KHI sửa bất cứ thứ gì liên quan tới WDA](02-wda-doc-truoc-khi-sua.md#2-đọc-mục-này-trước-khi-sửa-bất-cứ-thứ-gì-liên-quan-tới-wda) | `02-wda-doc-truoc-khi-sua.md` |
| §3 | [3. Kiến trúc](03-kien-truc.md#3-kiến-trúc) | `03-kien-truc.md` |
| §4 | [4. Chạy và test](04-chay-va-test.md#4-chạy-và-test) | `04-chay-va-test.md` |
| §5 | [5. Trạng thái bình luận](05-trang-thai-binh-luan.md#5-trạng-thái-bình-luận) | `05-trang-thai-binh-luan.md` |
| §6 | [6. Cách hiệu chỉnh detector (quy trình đã dùng, nên lặp lại)](06-hieu-chinh-detector.md#6-cách-hiệu-chỉnh-detector-quy-trình-đã-dùng-nên-lặp-lại) | `06-hieu-chinh-detector.md` |
| §7 | [7. Nguyên tắc khi sửa code này](07-nguyen-tac.md#7-nguyên-tắc-khi-sửa-code-này) | `07-nguyen-tac.md` |
| §8 | [8. Unified Agent Runtime (28/07/2026)](08-unified-agent-runtime.md#8-unified-agent-runtime-28072026) | `08-unified-agent-runtime.md` |
| §9 * | [9. Fleet Android (09/08/2026)](09-fleet-android.md#9-fleet-android-09082026) | `09-fleet-android.md` |
| §10 * | [10. Mở đường cho thiết bị mới (09/08/2026)](10-thiet-bi-moi.md#10-mở-đường-cho-thiết-bị-mới-09082026) | `10-thiet-bi-moi.md` |

## Mục con và checkpoint kế thừa

| § | Nội dung | File |
|---|---|---|
| §1.1 | [1.1 Tên: đổi ở desktop, KHÔNG đổi ở artifact iPhone (13/08/2026)](01-du-an-va-ten.md#11-tên-đổi-ở-desktop-không-đổi-ở-artifact-iphone-13082026) | `01-du-an-va-ten.md` |
| §2.1 | [2.1 KHÔNG bật `autoDismissAlerts` trong session capabilities](02-wda-doc-truoc-khi-sua.md#21-không-bật-autodismissalerts-trong-session-capabilities) | `02-wda-doc-truoc-khi-sua.md` |
| §2.2 | [2.2 Stock WDA PHẢI prime session trước mọi lệnh khác](02-wda-doc-truoc-khi-sua.md#22-stock-wda-phải-prime-session-trước-mọi-lệnh-khác) | `02-wda-doc-truoc-khi-sua.md` |
| §2.3 | [2.3 `snapshotMaxDepth` của stock WDA PHẢI là 1](02-wda-doc-truoc-khi-sua.md#23-snapshotmaxdepth-của-stock-wda-phải-là-1) | `02-wda-doc-truoc-khi-sua.md` |
| §2.4 | [2.4 Thứ tự khởi động: session TRƯỚC, stream SAU](02-wda-doc-truoc-khi-sua.md#24-thứ-tự-khởi-động-session-trước-stream-sau) | `02-wda-doc-truoc-khi-sua.md` |
| §2.5 | [2.5 Không bọc request WDA bằng `tokio::time::timeout`](02-wda-doc-truoc-khi-sua.md#25-không-bọc-request-wda-bằng-tokiotimetimeout) | `02-wda-doc-truoc-khi-sua.md` |
| §2.6 | [2.6 Không tạo session với `bundleId=SpringBoard` hoặc `forceAppLaunch=true`](02-wda-doc-truoc-khi-sua.md#26-không-tạo-session-với-bundleidspringboard-hoặc-forceapplaunchtrue) | `02-wda-doc-truoc-khi-sua.md` |
| §2.7 | [2.7 Chỉ recycle transport khi gesture thật lỗi với lớp transport](02-wda-doc-truoc-khi-sua.md#27-chỉ-recycle-transport-khi-gesture-thật-lỗi-với-lớp-transport) | `02-wda-doc-truoc-khi-sua.md` |
| §2.8 | [2.8 Không `pkill` rộng](02-wda-doc-truoc-khi-sua.md#28-không-pkill-rộng) | `02-wda-doc-truoc-khi-sua.md` |
| §2.9 | [2.9 Tắt 3uTools khi test](02-wda-doc-truoc-khi-sua.md#29-tắt-3utools-khi-test) | `02-wda-doc-truoc-khi-sua.md` |
| §2.9.1 | [2.9.1 Windows phải sở hữu cả cây tiến trình bằng Job Object](02-wda-doc-truoc-khi-sua.md#291-windows-phải-sở-hữu-cả-cây-tiến-trình-bằng-job-object) | `02-wda-doc-truoc-khi-sua.md` |
| §2.10 | [2.10 RT-MMO là backend riêng, không trộn với stock WDA](02-wda-doc-truoc-khi-sua.md#210-rt-mmo-là-backend-riêng-không-trộn-với-stock-wda) | `02-wda-doc-truoc-khi-sua.md` |
| §3.1 | [3.1 Đọc màn hình qua frame stream, không qua WDA](03-kien-truc.md#31-đọc-màn-hình-qua-frame-stream-không-qua-wda) | `03-kien-truc.md` |
| §3.2 | [3.2 Mọi hành động đều được xác nhận từ frame](03-kien-truc.md#32-mọi-hành-động-đều-được-xác-nhận-từ-frame) | `03-kien-truc.md` |
| §3.3 | [3.3 Toạ độ nút được dò trên từng frame](03-kien-truc.md#33-toạ-độ-nút-được-dò-trên-từng-frame) | `03-kien-truc.md` |
| §3.4 | [3.4 Watcher popup](03-kien-truc.md#34-watcher-popup) | `03-kien-truc.md` |
| §3.5 | [3.5 Supervisor theo UDID](03-kien-truc.md#35-supervisor-theo-udid) | `03-kien-truc.md` |
| §3.6 | [3.6 Nhịp hành vi](03-kien-truc.md#36-nhịp-hành-vi) | `03-kien-truc.md` |
| §3.7 | [3.7 Roadmap điều khiển iPhone thống nhất (chốt 28/07/2026)](03-kien-truc.md#37-roadmap-điều-khiển-iphone-thống-nhất-chốt-28072026) | `03-kien-truc.md` |
| §3.8 | [3.8 Hướng bỏ phụ thuộc RT-MMO (chốt 29/07/2026)](03-kien-truc.md#38-hướng-bỏ-phụ-thuộc-rt-mmo-chốt-29072026) | `03-kien-truc.md` |
| §3.9 | [3.9 Project 2 Riviu Agent candidate (checkpoint Mac 04/08/2026)](03-kien-truc.md#39-project-2-riviu-agent-candidate-checkpoint-mac-04082026) | `03-kien-truc.md` |
| §3.10 | [3.10 Handoff bat buoc khi mo du an tren Mac](03-kien-truc.md#310-handoff-bat-buoc-khi-mo-du-an-tren-mac) | `03-kien-truc.md` |
| §3.11 | [3.11 Proxy/supervision checkpoint (29/07/2026)](03-kien-truc.md#311-proxysupervision-checkpoint-29072026) | `03-kien-truc.md` |
| §3.12 | [3.12 TikTok Interaction Campaign (reviewed design 29/07/2026)](03-kien-truc.md#312-tiktok-interaction-campaign-reviewed-design-29072026) | `03-kien-truc.md` |
| §3.13 | [3.13 Interaction Gate 0 checkpoint Windows (30/07/2026)](03-kien-truc.md#313-interaction-gate-0-checkpoint-windows-30072026) | `03-kien-truc.md` |
| §3.14 | [3.14 Flow V2 visual automation design (30/07/2026)](03-kien-truc.md#314-flow-v2-visual-automation-design-30072026) | `03-kien-truc.md` |
| §3.15 | [3.15 Main integration va trang thai san pham trung thuc (31/07/2026)](03-kien-truc.md#315-main-integration-va-trang-thai-san-pham-trung-thuc-31072026) | `03-kien-truc.md` |
| §3.16 | [3.16 Active priority: Interaction Campaign + Flow V2 (31/07/2026)](03-kien-truc.md#316-active-priority-interaction-campaign--flow-v2-31072026) | `03-kien-truc.md` |
| §3.17 | [3.17 Flow V2 F2 va F3 fixture checkpoint (31/07/2026)](03-kien-truc.md#317-flow-v2-f2-va-f3-fixture-checkpoint-31072026) | `03-kien-truc.md` |
| §3.18 | [3.18 Desktop self-contained packaging va CI/CD (31/07/2026)](03-kien-truc.md#318-desktop-self-contained-packaging-va-cicd-31072026) | `03-kien-truc.md` |
| §3.18.1 | [3.18.1 Mac local bundle checkpoint (04/08/2026)](03-kien-truc.md#3181-mac-local-bundle-checkpoint-04082026) | `03-kien-truc.md` |
| §4.0 | [4.0 Provisioning fleet — GIỚI HẠN TÀI KHOẢN (đo 2026-07-27, 20 máy)](04-chay-va-test.md#40-provisioning-fleet--giới-hạn-tài-khoản-đo-2026-07-27-20-máy) | `04-chay-va-test.md` |
| §4.1 | [4.1 Đưa một máy mới vào dùng](04-chay-va-test.md#41-đưa-một-máy-mới-vào-dùng) | `04-chay-va-test.md` |
| §5.1 | [5.1 Bình luận chữ qua RT-MMO: ĐÃ LIVE XÁC NHẬN](05-trang-thai-binh-luan.md#51-bình-luận-chữ-qua-rt-mmo-đã-live-xác-nhận) | `05-trang-thai-binh-luan.md` |
| §5.2 | [5.2 Xác nhận tim ✅](05-trang-thai-binh-luan.md#52-xác-nhận-tim-) | `05-trang-thai-binh-luan.md` |
| §5.2b | [5.2b Thẻ không có thanh hành động — **bẫy lớn nhất trên máy mới** ✅](05-trang-thai-binh-luan.md#52b-thẻ-không-có-thanh-hành-động--bẫy-lớn-nhất-trên-máy-mới-) | `05-trang-thai-binh-luan.md` |
| §5.3 | [5.3 Trang "Chọn chủ đề" chưa có capture thật ⚠️](05-trang-thai-binh-luan.md#53-trang-chọn-chủ-đề-chưa-có-capture-thật-️) | `05-trang-thai-binh-luan.md` |
| §5.4 | [5.4 Phòng LIVE](05-trang-thai-binh-luan.md#54-phòng-live) | `05-trang-thai-binh-luan.md` |
| §9 * | [9. Context-grounded comment (04/08/2026)](08-unified-agent-runtime.md#9-context-grounded-comment-04082026) | `08-unified-agent-runtime.md` |
| §10 * | [10. Interaction Campaign implementation checkpoint (04/08/2026)](08-unified-agent-runtime.md#10-interaction-campaign-implementation-checkpoint-04082026) | `08-unified-agent-runtime.md` |
| §11 | [11. API comment preview (05/08/2026)](08-unified-agent-runtime.md#11-api-comment-preview-05082026) | `08-unified-agent-runtime.md` |
| §12 | [12. Stream preview scaling (05/08/2026)](08-unified-agent-runtime.md#12-stream-preview-scaling-05082026) | `08-unified-agent-runtime.md` |
| §13 | [13. Standalone Riviu Agent full interaction install (05/08/2026)](08-unified-agent-runtime.md#13-standalone-riviu-agent-full-interaction-install-05082026) | `08-unified-agent-runtime.md` |
| §14 | [14. Photo carousel publish campaign (05/08/2026)](08-unified-agent-runtime.md#14-photo-carousel-publish-campaign-05082026) | `08-unified-agent-runtime.md` |
| §14.1 | [14.1 Live checkpoint 06/08/2026](08-unified-agent-runtime.md#141-live-checkpoint-06082026) | `08-unified-agent-runtime.md` |
| §14.2 | [14.2 Live verifier checkpoint 06/08/2026](08-unified-agent-runtime.md#142-live-verifier-checkpoint-06082026) | `08-unified-agent-runtime.md` |
| §14.3 | [14.3 Gate B/C a99 checkpoint 06/08/2026](08-unified-agent-runtime.md#143-gate-bc-a99-checkpoint-06082026) | `08-unified-agent-runtime.md` |
| §14.4 | [14.4 Human-like nurture checkpoint 06/08/2026](08-unified-agent-runtime.md#144-human-like-nurture-checkpoint-06082026) | `08-unified-agent-runtime.md` |
| §14.5 | [14.5 Interaction/nurture stability contract (01/09/2026; xem §9.137)](08-unified-agent-runtime.md#145-interactionnurture-stability-contract-01092026-xem-9137) | `08-unified-agent-runtime.md` |
| §14.6 | [14.6 Windows clean-host và effect closure (01/09/2026; xem §9.138)](08-unified-agent-runtime.md#146-windows-clean-host-và-effect-closure-01092026-xem-9138) | `08-unified-agent-runtime.md` |
| §14.7 | [14.7 Nurture session shutdown và log theo số máy (02/09/2026; xem §9.139)](08-unified-agent-runtime.md#147-nurture-session-shutdown-và-log-theo-số-máy-02092026-xem-9139) | `08-unified-agent-runtime.md` |
| §14.8 | [14.8 Android package, AutoSwipe và fleet diagnostics (03/09/2026; xem §9.140)](08-unified-agent-runtime.md#148-android-package-autoswipe-và-fleet-diagnostics-03092026-xem-9140) | `08-unified-agent-runtime.md` |
| §14.9 | [14.9 Remount canvas không được làm mất bootstrap H.264 (03/09/2026; xem §9.141)](08-unified-agent-runtime.md#149-remount-canvas-không-được-làm-mất-bootstrap-h264-03092026-xem-9141) | `08-unified-agent-runtime.md` |
| §14.10 | [14.10 Profile automation, Save và điều phối TikTok (04/09/2026; xem §9.142)](08-unified-agent-runtime.md#1410-profile-automation-save-và-điều-phối-tiktok-04092026-xem-9142) | `08-unified-agent-runtime.md` |
| §14.11 | [14.11 Mobile MCP chỉ là dụng cụ canary, không phải control plane (04/09/2026; xem §9.143)](08-unified-agent-runtime.md#1411-mobile-mcp-chỉ-là-dụng-cụ-canary-không-phải-control-plane-04092026-xem-9143) | `08-unified-agent-runtime.md` |
| §14.12 | [14.12 UI production, trạng thái bền và cleanup công khai (05/09/2026; xem §9.145)](08-unified-agent-runtime.md#1412-ui-production-trạng-thái-bền-và-cleanup-công-khai-05092026-xem-9145) | `08-unified-agent-runtime.md` |
| §14.13 | [14.13 Phạm vi tự động hóa độc lập và ánh xạ Publish theo tập con (05/09/2026; xem §9.146)](08-unified-agent-runtime.md#1413-phạm-vi-tự-động-hóa-độc-lập-và-ánh-xạ-publish-theo-tập-con-05092026-xem-9146) | `08-unified-agent-runtime.md` |
| §14.14 | [14.14 Phản hồi UI không che công việc và thanh lệnh thống nhất (05/09/2026; xem §9.147)](08-unified-agent-runtime.md#1414-phản-hồi-ui-không-che-công-việc-và-thanh-lệnh-thống-nhất-05092026-xem-9147) | `08-unified-agent-runtime.md` |
| §14.15 | [14.15 Hình học form Nuôi TikTok và vị trí cuộn khi chuyển workspace (05/09/2026; xem §9.148)](08-unified-agent-runtime.md#1415-hình-học-form-nuôi-tiktok-và-vị-trí-cuộn-khi-chuyển-workspace-05092026-xem-9148) | `08-unified-agent-runtime.md` |
| §14.16 | [14.16 Bản nháp, hồ sơ và tiến độ vận hành có chủ thể (06/09/2026; xem §9.150)](08-unified-agent-runtime.md#1416-bản-nháp-hồ-sơ-và-tiến-độ-vận-hành-có-chủ-thể-06092026-xem-9150) | `08-unified-agent-runtime.md` |

## Số mục kế thừa có nhiều chủ sở hữu

Dấu `*` yêu cầu đọc tên file và tiêu đề, không chọn mục đầu tiên theo số.
§9/§10 ở file Fleet/Thiết bị mới là mục tham chiếu chính; các checkpoint cùng số trong §8 giữ tên và vị trí lịch sử.
§9.43, §9.44, §9.45 là các mục khác ngày. §9.105 và §9.115 có các phần tiếp cùng chủ đề.
Khi trích các mục này, ghi thêm ngày/tiêu đề và liên kết trực tiếp. Mục mới dùng số mới; không mở rộng danh sách ngoại lệ để che va chạm.

## Mới nhất

- [§9.152: §9.152 UI cam trắng, phạm vi tác vụ và biên Local API (06/09/2026)](diary/06-2408-2708.md#9152-ui-cam-trắng-phạm-vi-tác-vụ-và-biên-local-api-06092026)
- [§9.151: §9.151 Tài liệu theo tác vụ, chỉ mục có ngữ nghĩa và dữ liệu trùng được chứng minh (06/09/2026)](diary/06-2408-2708.md#9151-tài-liệu-theo-tác-vụ-chỉ-mục-có-ngữ-nghĩa-và-dữ-liệu-trùng-được-chứng-minh-06092026)
- [§9.150: §9.150 Bản nháp, hồ sơ và tiến độ fleet không còn phụ thuộc tab đang mở (06/09/2026)](diary/06-2408-2708.md#9150-bản-nháp-hồ-sơ-và-tiến-độ-fleet-không-còn-phụ-thuộc-tab-đang-mở-06092026)
- [§9.149: §9.149 Form hồ sơ đồng nhất và trạng thái sẵn sàng Nuôi TikTok (05/09/2026)](diary/06-2408-2708.md#9149-form-hồ-sơ-đồng-nhất-và-trạng-thái-sẵn-sàng-nuôi-tiktok-05092026)
- [§9.148: §9.148 Một icon 16×32 px kéo lệch cả hàng Nuôi TikTok (05/09/2026)](diary/06-2408-2708.md#9148-một-icon-1632-px-kéo-lệch-cả-hàng-nuôi-tiktok-05092026)

## Nhật ký §9.x

| § | Nội dung | File |
|---|---|---|
| §9.1 | [9.1 Nurture Android: cùng policy, khác cách nhìn (10/08/2026)](diary/01-1008-1808.md#91-nurture-android-cùng-policy-khác-cách-nhìn-10082026) | `diary/01-1008-1808.md` |
| §9.2 | [9.2 ĐÍNH CHÍNH (11/08/2026): clipboard KHÔNG chặn Interaction](diary/01-1008-1808.md#92-đính-chính-11082026-clipboard-không-chặn-interaction) | `diary/01-1008-1808.md` |
| §9.3 | [9.3 Hai bẫy môi trường đo được (11/08/2026) — cả hai từng trông như lỗi locator](diary/01-1008-1808.md#93-hai-bẫy-môi-trường-đo-được-11082026--cả-hai-từng-trông-như-lỗi-locator) | `diary/01-1008-1808.md` |
| §9.4 | [9.4 `platform` + `os_version` (11/08/2026) — và ba chỗ cố ý KHÔNG đổi](diary/01-1008-1808.md#94-platform--os_version-11082026--và-ba-chỗ-cố-ý-không-đổi) | `diary/01-1008-1808.md` |
| §9.5 | [9.5 Hàng comment trong drawer (11/08/2026) — nhãn nằm ở `text`, không phải `content-desc`](diary/01-1008-1808.md#95-hàng-comment-trong-drawer-11082026--nhãn-nằm-ở-text-không-phải-content-desc) | `diary/01-1008-1808.md` |
| §9.6 | [9.6 Package TikTok theo từng máy (11/08/2026)](diary/01-1008-1808.md#96-package-tiktok-theo-từng-máy-11082026) | `diary/01-1008-1808.md` |
| §9.7 | [9.7 Composer reply (11/08/2026) — bốn câu hỏi, bốn câu trả lời đo được](diary/01-1008-1808.md#97-composer-reply-11082026--bốn-câu-hỏi-bốn-câu-trả-lời-đo-được) | `diary/01-1008-1808.md` |
| §9.8 | [9.8 `tiktok_drawer` — tách dùng chung, không nhân bản](diary/01-1008-1808.md#98-tiktok_drawer--tách-dùng-chung-không-nhân-bản) | `diary/01-1008-1808.md` |
| §9.9 | [9.9 Gate actor Interaction: theo *tính chất*, không theo nền tảng](diary/01-1008-1808.md#99-gate-actor-interaction-theo-tính-chất-không-theo-nền-tảng) | `diary/01-1008-1808.md` |
| §9.10 | [9.10 MediaStore (11/08/2026) — `adb push` là đủ, không cần scan](diary/01-1008-1808.md#910-mediastore-11082026--adb-push-là-đủ-không-cần-scan) | `diary/01-1008-1808.md` |
| §9.11 | [9.11 Bottom tab bar (11/08/2026)](diary/01-1008-1808.md#911-bottom-tab-bar-11082026) | `diary/01-1008-1808.md` |
| §9.12 | [9.12 BẪY MÔI TRƯỜNG: Git Bash mangle đường dẫn `adb push`](diary/01-1008-1808.md#912-bẫy-môi-trường-git-bash-mangle-đường-dẫn-adb-push) | `diary/01-1008-1808.md` |
| §9.13 | [9.13 Resource id nút Gửi ĐÃ đổi giữa hai phiên bản app (11/08/2026)](diary/01-1008-1808.md#913-resource-id-nút-gửi-đã-đổi-giữa-hai-phiên-bản-app-11082026) | `diary/01-1008-1808.md` |
| §9.14 | [9.14 `TargetDriver`: một refactor và một lỗi nó tự gây ra (11/08/2026)](diary/01-1008-1808.md#914-targetdriver-một-refactor-và-một-lỗi-nó-tự-gây-ra-11082026) | `diary/01-1008-1808.md` |
| §9.15 | [9.15 `open_url` mở link TikTok vào HỘP THOẠI CHỌN APP, không vào TikTok (11/08/2026)](diary/01-1008-1808.md#915-open_url-mở-link-tiktok-vào-hộp-thoại-chọn-app-không-vào-tiktok-11082026) | `diary/01-1008-1808.md` |
| §9.16 | [9.16 GATE H5 PASSED — reply gắn đúng cha, hai máy thật (11/08/2026)](diary/01-1008-1808.md#916-gate-h5-passed--reply-gắn-đúng-cha-hai-máy-thật-11082026) | `diary/01-1008-1808.md` |
| §9.17 | [9.17 Link nào mở được: chỉ máy trả lời được, host thì không (11/08/2026)](diary/01-1008-1808.md#917-link-nào-mở-được-chỉ-máy-trả-lời-được-host-thì-không-11082026) | `diary/01-1008-1808.md` |
| §9.18 | [9.18 Nurture Android chạy được qua app — và hai lỗi chỉ có máy thật mới thấy (12/08/2026)](diary/01-1008-1808.md#918-nurture-android-chạy-được-qua-app--và-hai-lỗi-chỉ-có-máy-thật-mới-thấy-12082026) | `diary/01-1008-1808.md` |
| §9.19 | [9.19 Ba chỗ ở §9.18 đã sửa, kèm số đo (12/08/2026)](diary/01-1008-1808.md#919-ba-chỗ-ở-918-đã-sửa-kèm-số-đo-12082026) | `diary/01-1008-1808.md` |
| §9.20 | [9.20 Vuốt ngang bài ảnh trên Android — và ba kết luận sai phải sửa để làm được (12/08/2026)](diary/01-1008-1808.md#920-vuốt-ngang-bài-ảnh-trên-android--và-ba-kết-luận-sai-phải-sửa-để-làm-được-12082026) | `diary/01-1008-1808.md` |
| §9.21 | [9.21 Agent còn sống mà cây đã chết: `/status` không phải bằng chứng (12/08/2026)](diary/02-1208-1908.md#921-agent-còn-sống-mà-cây-đã-chết-status-không-phải-bằng-chứng-12082026) | `diary/02-1208-1908.md` |
| §9.22 | [9.22 Toàn quyền: `human_limits` mặc định TẮT (12/08/2026)](diary/02-1208-1908.md#922-toàn-quyền-human_limits-mặc-định-tắt-12082026) | `diary/02-1208-1908.md` |
| §9.23 | [9.23 Cử chỉ: đường vuốt thật thay vì một đoạn thẳng (12/08/2026)](diary/02-1208-1908.md#923-cử-chỉ-đường-vuốt-thật-thay-vì-một-đoạn-thẳng-12082026) | `diary/02-1208-1908.md` |
| §9.24 | [9.24 Vị trí chạm: cụm có lệch, không phải random đều (12/08/2026)](diary/02-1208-1908.md#924-vị-trí-chạm-cụm-có-lệch-không-phải-random-đều-12082026) | `diary/02-1208-1908.md` |
| §9.25 | [9.25 Nhịp thời gian: bỏ luật chống lặp và một cái lỗ trong histogram (12/08/2026)](diary/02-1208-1908.md#925-nhịp-thời-gian-bỏ-luật-chống-lặp-và-một-cái-lỗ-trong-histogram-12082026) | `diary/02-1208-1908.md` |
| §9.26 | [9.26 Interaction: thả tim, và bình luận thủ công (12/08/2026)](diary/02-1208-1908.md#926-interaction-thả-tim-và-bình-luận-thủ-công-12082026) | `diary/02-1208-1908.md` |
| §9.27 | [9.27 Đảo quyết định: minicap vào bộ cài, không để ngoài repo (12/08/2026)](diary/02-1208-1908.md#927-đảo-quyết-định-minicap-vào-bộ-cài-không-để-ngoài-repo-12082026) | `diary/02-1208-1908.md` |
| §9.28 | [9.28 Đóng gói xong: số đo, và bốn thứ suýt hỏng im lặng (12/08/2026)](diary/02-1208-1908.md#928-đóng-gói-xong-số-đo-và-bốn-thứ-suýt-hỏng-im-lặng-12082026) | `diary/02-1208-1908.md` |
| §9.29 | [9.29 Sidecar iOS hỏng mà app báo khoẻ: sự im lặng là HAI lỗi (12/08/2026)](diary/02-1208-1908.md#929-sidecar-ios-hỏng-mà-app-báo-khoẻ-sự-im-lặng-là-hai-lỗi-12082026) | `diary/02-1208-1908.md` |
| §9.30 | [9.30 Interaction chạy thật lần đầu qua app — và cửa arrival FAIL OPEN (13/08/2026)](diary/02-1208-1908.md#930-interaction-chạy-thật-lần-đầu-qua-app--và-cửa-arrival-fail-open-13082026) | `diary/02-1208-1908.md` |
| §9.31 | [9.31 H6-a ĐẠT sau ba lần chạy — và link chết trông giống hệt lỗi code (13/08/2026)](diary/02-1208-1908.md#931-h6-a-đạt-sau-ba-lần-chạy--và-link-chết-trông-giống-hệt-lỗi-code-13082026) | `diary/02-1208-1908.md` |
| §9.32 | [9.32 H6-b và H6-c ĐẠT trong một lần chạy (13/08/2026)](diary/02-1208-1908.md#932-h6-b-và-h6-c-đạt-trong-một-lần-chạy-13082026) | `diary/02-1208-1908.md` |
| §9.33 | [9.33 Ba lỗi làm chế độ AI không debug được (13/08/2026)](diary/05-1308-2408.md#933-ba-lỗi-làm-chế-độ-ai-không-debug-được-13082026) | `diary/05-1308-2408.md` |
| §9.34 | [9.34 H6-d ĐẠT sau khi sửa ba chỗ ở 9.33 (13/08/2026)](diary/02-1208-1908.md#934-h6-d-đạt-sau-khi-sửa-ba-chỗ-ở-933-13082026) | `diary/02-1208-1908.md` |
| §9.35 | [9.35 Tách `graceful_shutdown`, và một lo ngại bị nói quá (13/08/2026)](diary/05-1308-2408.md#935-tách-graceful_shutdown-và-một-lo-ngại-bị-nói-quá-13082026) | `diary/05-1308-2408.md` |
| §9.36 | [9.36 Đăng bài: phần không cần máy đã làm, và phép đo quyết định (13/08/2026)](diary/02-1208-1908.md#936-đăng-bài-phần-không-cần-máy-đã-làm-và-phép-đo-quyết-định-13082026) | `diary/02-1208-1908.md` |
| §9.37 | [9.37 Trang bài của mình: hai dấu hiệu dương, và KHÔNG có nút xoá nào có nhãn (13/08/2026)](diary/05-1308-2408.md#937-trang-bài-của-mình-hai-dấu-hiệu-dương-và-không-có-nút-xoá-nào-có-nhãn-13082026) | `diary/05-1308-2408.md` |
| §9.38 | [9.38 CI đỏ bốn lần vì đúng cái bẫy tôi đã tự cảnh báo (13/08/2026)](diary/05-1308-2408.md#938-ci-đỏ-bốn-lần-vì-đúng-cái-bẫy-tôi-đã-tự-cảnh-báo-13082026) | `diary/05-1308-2408.md` |
| §9.39 | [9.39 Auto-update: khoá, và hai chỗ cố ý KHÔNG tự động (13/08/2026)](diary/05-1308-2408.md#939-auto-update-khoá-và-hai-chỗ-cố-ý-không-tự-động-13082026) | `diary/05-1308-2408.md` |
| §9.40 | [9.40 M4 ĐẠT — caption đọc được nguyên văn, và ngưỡng cắt ở đâu (13/08/2026)](diary/05-1308-2408.md#940-m4-đạt--caption-đọc-được-nguyên-văn-và-ngưỡng-cắt-ở-đâu-13082026) | `diary/05-1308-2408.md` |
| §9.41 | [9.41 Updater xong đường phát hành: `latest.json`, và thứ tự lúc cài (13/08/2026)](diary/05-1308-2408.md#941-updater-xong-đường-phát-hành-latestjson-và-thứ-tự-lúc-cài-13082026) | `diary/05-1308-2408.md` |
| §9.42 | [9.42 Ngưỡng thời gian của `classification_stays_fast_enough_for_the_watcher` (13/08/2026)](diary/05-1308-2408.md#942-ngưỡng-thời-gian-của-classification_stays_fast_enough_for_the_watcher-13082026) | `diary/05-1308-2408.md` |
| §9.43 * | [9.43 Bài ảnh, chương bốn: cử chỉ quá giống người thì pager không nhận (18/08/2026)](diary/01-1008-1808.md#943-bài-ảnh-chương-bốn-cử-chỉ-quá-giống-người-thì-pager-không-nhận-18082026) | `diary/01-1008-1808.md` |
| §9.43 * | [9.43 Hai lối xoá còn lại: đã đo, đều đóng — và một lối khác mở ra (13/08/2026)](diary/04-1308-1608.md#943-hai-lối-xoá-còn-lại-đã-đo-đều-đóng--và-một-lối-khác-mở-ra-13082026) | `diary/04-1308-1608.md` |
| §9.44 * | [9.44 Hộp thoại không phải của TikTok, và giới hạn của việc tự khắc phục (18/08/2026)](diary/01-1008-1808.md#944-hộp-thoại-không-phải-của-tiktok-và-giới-hạn-của-việc-tự-khắc-phục-18082026) | `diary/01-1008-1808.md` |
| §9.44 * | [9.44 Đường Đăng bài cho máy Android đi qua, và bốn version chỉ kiểm ba (13/08/2026)](diary/04-1308-1608.md#944-đường-đăng-bài-cho-máy-android-đi-qua-và-bốn-version-chỉ-kiểm-ba-13082026) | `diary/04-1308-1608.md` |
| §9.45 * | [9.45 Bình luận chạy lần đầu — và tên lỗi chỉ sai hướng suốt 45% số lượt (19/08/2026)](diary/02-1208-1908.md#945-bình-luận-chạy-lần-đầu--và-tên-lỗi-chỉ-sai-hướng-suốt-45-số-lượt-19082026) | `diary/02-1208-1908.md` |
| §9.45 * | [9.45 Tag đầu tiên: job release chưa bao giờ chạy, và nó hỏng (13/08/2026)](diary/04-1308-1608.md#945-tag-đầu-tiên-job-release-chưa-bao-giờ-chạy-và-nó-hỏng-13082026) | `diary/04-1308-1608.md` |
| §9.46 | [9.46 Ngân sách 10 mili-giây trong test, và lần thứ hai cùng một loại lỗi (13/08/2026)](diary/04-1308-1608.md#946-ngân-sách-10-mili-giây-trong-test-và-lần-thứ-hai-cùng-một-loại-lỗi-13082026) | `diary/04-1308-1608.md` |
| §9.47 | [9.47 v0.1.1 đã phát hành, và chuỗi updater nghiệm thu từ ngoài (13/08/2026)](diary/04-1308-1608.md#947-v011-đã-phát-hành-và-chuỗi-updater-nghiệm-thu-từ-ngoài-13082026) | `diary/04-1308-1608.md` |
| §9.48 | [9.48 Điều khiển từ máy tính không được park stream (14/08/2026)](diary/02-1208-1908.md#948-điều-khiển-từ-máy-tính-không-được-park-stream-14082026) | `diary/02-1208-1908.md` |
| §9.49 | [9.49 GenFarmer mượt vì codec + canvas, không vì CSS (14/08/2026)](diary/02-1208-1908.md#949-genfarmer-mượt-vì-codec--canvas-không-vì-css-14082026) | `diary/02-1208-1908.md` |
| §9.50 | [9.50 Đường xem H.264 / canvas — xem ≠ bằng chứng (14/08/2026)](diary/02-1208-1908.md#950-đường-xem-h264--canvas--xem--bằng-chứng-14082026) | `diary/02-1208-1908.md` |
| §9.51 | [9.51 Riviu Agent trên Android — không phải toàn quyền (14/08/2026)](diary/02-1208-1908.md#951-riviu-agent-trên-android--không-phải-toàn-quyền-14082026) | `diary/02-1208-1908.md` |
| §9.52 | [9.52 Helper APK `com.riviu.agent` — clipboard + MediaStore, IME phải trả lại (14/08/2026)](diary/02-1208-1908.md#952-helper-apk-comriviuagent--clipboard--mediastore-ime-phải-trả-lại-14082026) | `diary/02-1208-1908.md` |
| §9.53 | [9.53 Overlay tap trượt vì map cả ô đen, không phải canvas (14/08/2026)](diary/02-1208-1908.md#953-overlay-tap-trượt-vì-map-cả-ô-đen-không-phải-canvas-14082026) | `diary/02-1208-1908.md` |
| §9.54 | [9.54 Overlay lag: restart encoder + khoá pointer + render mỗi frame (14/08/2026)](diary/02-1208-1908.md#954-overlay-lag-restart-encoder--khoá-pointer--render-mỗi-frame-14082026) | `diary/02-1208-1908.md` |
| §9.55 | [9.55 Default AI OpenRouter Luna, và scrcpy chết vì sai form codec option (14/08/2026)](diary/04-1308-1608.md#955-default-ai-openrouter-luna-và-scrcpy-chết-vì-sai-form-codec-option-14082026) | `diary/04-1308-1608.md` |
| §9.56 | [9.56 Tile đen vì máy ngủ; và ba đính chính cho hồ sơ đổi tên (14/08/2026)](diary/04-1308-1608.md#956-tile-đen-vì-máy-ngủ-và-ba-đính-chính-cho-hồ-sơ-đổi-tên-14082026) | `diary/04-1308-1608.md` |
| §9.57 | [9.57 Danh sách app trên máy: `cmd package`, và nhãn thì không có (14/08/2026)](diary/04-1308-1608.md#957-danh-sách-app-trên-máy-cmd-package-và-nhãn-thì-không-có-14082026) | `diary/04-1308-1608.md` |
| §9.58 | [9.58 Layout theo GenFarmer: tab nhóm + menu chuột phải, và cách tìm ra đúng file (14/08/2026)](diary/04-1308-1608.md#958-layout-theo-genfarmer-tab-nhóm--menu-chuột-phải-và-cách-tìm-ra-đúng-file-14082026) | `diary/04-1308-1608.md` |
| §9.59 | [9.59 Bon hanh dong thiet bi, va ba thu do duoc lat nguoc thiet ke (14/08/2026)](diary/04-1308-1608.md#959-bon-hanh-dong-thiet-bi-va-ba-thu-do-duoc-lat-nguoc-thiet-ke-14082026) | `diary/04-1308-1608.md` |
| §9.60 | [9.60 `adb forward` song lau hon app, va vi sao no lam man hinh den (14/08/2026)](diary/04-1308-1608.md#960-adb-forward-song-lau-hon-app-va-vi-sao-no-lam-man-hinh-den-14082026) | `diary/04-1308-1608.md` |
| §9.61 | [9.61 `tracing` khong co sink: mot gio chan doan bi mu (14/08/2026)](diary/04-1308-1608.md#961-tracing-khong-co-sink-mot-gio-chan-doan-bi-mu-14082026) | `diary/04-1308-1608.md` |
| §9.62 | [9.62 `dblclick` trong `driver.ps1`: hai click roi rac khong phai mot double-click (14/08/2026)](diary/04-1308-1608.md#962-dblclick-trong-driverps1-hai-click-roi-rac-khong-phai-mot-double-click-14082026) | `diary/04-1308-1608.md` |
| §9.63 | [9.63 Overlay cuoi cung co encode rieng, va con so 900 trong ke hoach cua toi la sai (15/08/2026)](diary/03-1508-2108.md#963-overlay-cuoi-cung-co-encode-rieng-va-con-so-900-trong-ke-hoach-cua-toi-la-sai-15082026) | `diary/03-1508-2108.md` |
| §9.64 | [9.64 Man den bao gom mot cai treo cua chinh dien thoai, va mot diem mu 8 phut (15/08/2026)](diary/04-1308-1608.md#964-man-den-bao-gom-mot-cai-treo-cua-chinh-dien-thoai-va-mot-diem-mu-8-phut-15082026) | `diary/04-1308-1608.md` |
| §9.65 | [9.65 Keyframe khong phai bang chung co SPS — va vi sao chan doan im lang suot 3 vong (15/08/2026)](diary/04-1308-1608.md#965-keyframe-khong-phai-bang-chung-co-sps--va-vi-sao-chan-doan-im-lang-suot-3-vong-15082026) | `diary/04-1308-1608.md` |
| §9.66 | [9.66 Vite khong chuyen tiep console cua Web Worker — ba vong chan doan bi mu vi dieu nay](diary/04-1308-1608.md#966-vite-khong-chuyen-tiep-console-cua-web-worker--ba-vong-chan-doan-bi-mu-vi-dieu-nay) | `diary/04-1308-1608.md` |
| §9.67 | [9.67 Detector stall tu restart la vong phan hoi duong — cang nhieu may cang chet (15/08/2026)](diary/04-1308-1608.md#967-detector-stall-tu-restart-la-vong-phan-hoi-duong--cang-nhieu-may-cang-chet-15082026) | `diary/04-1308-1608.md` |
| §9.68 | [9.68 `BROADCAST_CAP` phai doc nhu mot toc do, khong phai mot kich thuoc (15/08/2026)](diary/04-1308-1608.md#968-broadcast_cap-phai-doc-nhu-mot-toc-do-khong-phai-mot-kich-thuoc-15082026) | `diary/04-1308-1608.md` |
| §9.69 | [9.69 App bao nguoi van hanh cai hai APK ma no khong he ship (16/08/2026)](diary/03-1508-2108.md#969-app-bao-nguoi-van-hanh-cai-hai-apk-ma-no-khong-he-ship-16082026) | `diary/03-1508-2108.md` |
| §9.70 | [9.70 Overlay quyet dinh ca cu keo tu DUNG HAI DIEM (16/08/2026)](diary/03-1508-2108.md#970-overlay-quyet-dinh-ca-cu-keo-tu-dung-hai-diem-16082026) | `diary/03-1508-2108.md` |
| §9.71 | [9.71 Bat `control=true` lam mat video ca 20 may — va no chan IM LANG (16/08/2026)](diary/03-1508-2108.md#971-bat-controltrue-lam-mat-video-ca-20-may--va-no-chan-im-lang-16082026) | `diary/03-1508-2108.md` |
| §9.72 | [9.72 Tran dong thoi cho recovery: da do, va phep do BAC BO ly do ban dau (16/08/2026)](diary/04-1308-1608.md#972-tran-dong-thoi-cho-recovery-da-do-va-phep-do-bac-bo-ly-do-ban-dau-16082026) | `diary/04-1308-1608.md` |
| §9.73 | [9.73 Mot kenh moi may — va cai bay chi mot test socket that moi thay (16/08/2026)](diary/04-1308-1608.md#973-mot-kenh-moi-may--va-cai-bay-chi-mot-test-socket-that-moi-thay-16082026) | `diary/04-1308-1608.md` |
| §9.74 | [9.74 `app_process` chet o 255 byte argv — nguyen nhan that su cua §9.71 (16/08/2026)](diary/04-1308-1608.md#974-app_process-chet-o-255-byte-argv--nguyen-nhan-that-su-cua-971-16082026) | `diary/04-1308-1608.md` |
| §9.75 | [9.75 Import/Export anh-video hai chieu, va cai bay `.thumbnails` (16/08/2026)](diary/04-1308-1608.md#975-importexport-anh-video-hai-chieu-va-cai-bay-thumbnails-16082026) | `diary/04-1308-1608.md` |
| §9.76 | [9.76 Phan 3 xong: socket control, `RESET_VIDEO`, va mot ket luan tu bac bo (16/08/2026)](diary/03-1508-2108.md#976-phan-3-xong-socket-control-reset_video-va-mot-ket-luan-tu-bac-bo-16082026) | `diary/03-1508-2108.md` |
| §9.77 | [9.77 Do "khong muot" thay vi doan: CPU khong phai thu phanh, va cho no that su nam (17/08/2026)](diary/03-1508-2108.md#977-do-khong-muot-thay-vi-doan-cpu-khong-phai-thu-phanh-va-cho-no-that-su-nam-17082026) | `diary/03-1508-2108.md` |
| §9.78 | [9.78 Keo truc tiep qua socket control — va cai bay "no chay roi" (17/08/2026)](diary/03-1508-2108.md#978-keo-truc-tiep-qua-socket-control--va-cai-bay-no-chay-roi-17082026) | `diary/03-1508-2108.md` |
| §9.79 | [9.79 Duong phuc hoi agent KHONG VOI TOI DUOC, va cooldown cho no (17/08/2026)](diary/03-1508-2108.md#979-duong-phuc-hoi-agent-khong-voi-toi-duoc-va-cooldown-cho-no-17082026) | `diary/03-1508-2108.md` |
| §9.80 | [9.80 Cham cung di socket control — de agent thoi la diem chet duy nhat (17/08/2026)](diary/03-1508-2108.md#980-cham-cung-di-socket-control--de-agent-thoi-la-diem-chet-duy-nhat-17082026) | `diary/03-1508-2108.md` |
| §9.81 | [9.81 95% thoi gian mo mot view nam trong MOT dong shell (17/08/2026)](diary/03-1508-2108.md#981-95-thoi-gian-mo-mot-view-nam-trong-mot-dong-shell-17082026) | `diary/03-1508-2108.md` |
| §9.82 | [9.82 Thay nong producer: giu hinh cu toi khi hinh moi co keyframe (17/08/2026)](diary/03-1508-2108.md#982-thay-nong-producer-giu-hinh-cu-toi-khi-hinh-moi-co-keyframe-17082026) | `diary/03-1508-2108.md` |
| §9.83 | [9.83 Dang bai day MOI bundle sang MOI may — va hai backend hieu `source_root` khac nhau (17/08/2026)](diary/03-1508-2108.md#983-dang-bai-day-moi-bundle-sang-moi-may--va-hai-backend-hieu-source_root-khac-nhau-17082026) | `diary/03-1508-2108.md` |
| §9.84 | [9.84 Go han dang nhap: mat khau plaintext trong cot ten `password_hash` (17/08/2026)](diary/03-1508-2108.md#984-go-han-dang-nhap-mat-khau-plaintext-trong-cot-ten-password_hash-17082026) | `diary/03-1508-2108.md` |
| §9.85 | [9.85 Flow chay that tren Android: cai `inspect_device_for_target` con thieu (17/08/2026)](diary/03-1508-2108.md#985-flow-chay-that-tren-android-cai-inspect_device_for_target-con-thieu-17082026) | `diary/03-1508-2108.md` |
| §9.86 | [9.86 27 loi cua dot soat doi khang: nhung gi dang nho lai (17/08/2026)](diary/03-1508-2108.md#986-27-loi-cua-dot-soat-doi-khang-nhung-gi-dang-nho-lai-17082026) | `diary/03-1508-2108.md` |
| §9.87 | [9.87 Chay that tren 20 may bat duoc hai loi khong test nao bat duoc (17/08/2026)](diary/03-1508-2108.md#987-chay-that-tren-20-may-bat-duoc-hai-loi-khong-test-nao-bat-duoc-17082026) | `diary/03-1508-2108.md` |
| §9.88 | [9.88 Menu chức năng từng máy: đo được gì trên máy thật, và bốn cái bẫy (21/08/2026)](diary/03-1508-2108.md#988-menu-chức-năng-từng-máy-đo-được-gì-trên-máy-thật-và-bốn-cái-bẫy-21082026) | `diary/03-1508-2108.md` |
| §9.89 | [9.89 Lúc phóng to cũng phải có đủ chức năng, và nhãn + icon app thật (21/08/2026)](diary/03-1508-2108.md#989-lúc-phóng-to-cũng-phải-có-đủ-chức-năng-và-nhãn--icon-app-thật-21082026) | `diary/03-1508-2108.md` |
| §9.90 | [9.90 "Ba dòng này không chạy" — cả ba đều chạy, và đó mới là vấn đề (21/08/2026)](diary/03-1508-2108.md#990-ba-dòng-này-không-chạy--cả-ba-đều-chạy-và-đó-mới-là-vấn-đề-21082026) | `diary/03-1508-2108.md` |
| §9.91 | [9.91 Hover mở submenu, một vùng cuộn, và `[object Object]` (21/08/2026)](diary/03-1508-2108.md#991-hover-mở-submenu-một-vùng-cuộn-và-object-object-21082026) | `diary/03-1508-2108.md` |
| §9.92 | [9.92 Một danh sách, và cái `max-height` cắt mất App List (21/08/2026)](diary/03-1508-2108.md#992-một-danh-sách-và-cái-max-height-cắt-mất-app-list-21082026) | `diary/03-1508-2108.md` |
| §9.93 | [9.93 Một nhãn bị hỏi "là sao?", và màu xanh trong một sản phẩm màu cam (21/08/2026)](diary/03-1508-2108.md#993-một-nhãn-bị-hỏi-là-sao-và-màu-xanh-trong-một-sản-phẩm-màu-cam-21082026) | `diary/03-1508-2108.md` |
| §9.94 | [9.94 Bốn thanh kéo chia nhau một trăm phần trăm (21/08/2026)](diary/05-1308-2408.md#994-bốn-thanh-kéo-chia-nhau-một-trăm-phần-trăm-21082026) | `diary/05-1308-2408.md` |
| §9.95 | [9.95 Thanh kéo đổi thang đo, và một công tắc tắt vẫn bị thu tiền (21/08/2026)](diary/05-1308-2408.md#995-thanh-kéo-đổi-thang-đo-và-một-công-tắc-tắt-vẫn-bị-thu-tiền-21082026) | `diary/05-1308-2408.md` |
| §9.96 | [9.96 `[object Object]` ở 47 chỗ, và ba lỗi mà chỉ e2e nhìn thấy (21/08/2026)](diary/05-1308-2408.md#996-object-object-ở-47-chỗ-và-ba-lỗi-mà-chỉ-e2e-nhìn-thấy-21082026) | `diary/05-1308-2408.md` |
| §9.97 | [9.97 Mô hình đe doạ cho hai cổng nghe, và chín lỗ đã bịt (22/08/2026)](diary/05-1308-2408.md#997-mô-hình-đe-doạ-cho-hai-cổng-nghe-và-chín-lỗ-đã-bịt-22082026) | `diary/05-1308-2408.md` |
| §9.98 | [9.98 Bốn lỗi mà chỉ việc dọn mới lôi ra, và năm mục tôi từ chối làm (23/08/2026)](diary/05-1308-2408.md#998-bốn-lỗi-mà-chỉ-việc-dọn-mới-lôi-ra-và-năm-mục-tôi-từ-chối-làm-23082026) | `diary/05-1308-2408.md` |
| §9.99 | [9.99 Sáu máy kẹt sau một trang không ai gỡ được, và cái thang phải chuyển nhà (23/08/2026)](diary/05-1308-2408.md#999-sáu-máy-kẹt-sau-một-trang-không-ai-gỡ-được-và-cái-thang-phải-chuyển-nhà-23082026) | `diary/05-1308-2408.md` |
| §9.100 | [9.100 Hai máy khoá màn hình, một câu báo lỗi nói sai, và thanh tiến trình đầu tiên (23/08/2026)](diary/05-1308-2408.md#9100-hai-máy-khoá-màn-hình-một-câu-báo-lỗi-nói-sai-và-thanh-tiến-trình-đầu-tiên-23082026) | `diary/05-1308-2408.md` |
| §9.101 | [9.101 Giá tiền tự bịa, một cổng vision hết hạn, và cái field `vision_body` không gửi (23/08/2026)](diary/05-1308-2408.md#9101-giá-tiền-tự-bịa-một-cổng-vision-hết-hạn-và-cái-field-vision_body-không-gửi-23082026) | `diary/05-1308-2408.md` |
| §9.102 | [9.102 Tại sao 3 máy không lướt ngang — và đo ra thì nhãn có, tôi đọc sai một lần (23/08/2026)](diary/05-1308-2408.md#9102-tại-sao-3-máy-không-lướt-ngang--và-đo-ra-thì-nhãn-có-tôi-đọc-sai-một-lần-23082026) | `diary/05-1308-2408.md` |
| §9.103 | [§9.103 — Bằng chứng cho bình luận: tấm ghép, khung trùng, và thứ tự](diary/05-1308-2408.md#9103--bằng-chứng-cho-bình-luận-tấm-ghép-khung-trùng-và-thứ-tự) | `diary/05-1308-2408.md` |
| §9.104 | [§9.104 — `card_is_still` cũng băm cả khung; và ba máy chung một AP không có mạng](diary/05-1308-2408.md#9104--card_is_still-cũng-băm-cả-khung-và-ba-máy-chung-một-ap-không-có-mạng) | `diary/05-1308-2408.md` |
| §9.105 * | [§9.105 — Mention thật cần phím thật; view tích luỹ; và một cổng đo bắn vào splash (24/08/2026)](diary/05-1308-2408.md#9105--mention-thật-cần-phím-thật-view-tích-luỹ-và-một-cổng-đo-bắn-vào-splash-24082026) | `diary/05-1308-2408.md` |
| §9.105 * | [§9.105 tiếp — bốn máy đó vẫn ở đúng bài, và ba lần tôi nói khác đều là lỗi dụng cụ](diary/05-1308-2408.md#9105-tiếp--bốn-máy-đó-vẫn-ở-đúng-bài-và-ba-lần-tôi-nói-khác-đều-là-lỗi-dụng-cụ) | `diary/05-1308-2408.md` |
| §9.106 | [9.106 Lịch tự chạy của nuôi TikTok (24/08/2026)](diary/05-1308-2408.md#9106-lịch-tự-chạy-của-nuôi-tiktok-24082026) | `diary/05-1308-2408.md` |
| §9.107 | [9.107 Khung giờ cho lịch nuôi, và nút chọn tất cả (24/08/2026)](diary/06-2408-2708.md#9107-khung-giờ-cho-lịch-nuôi-và-nút-chọn-tất-cả-24082026) | `diary/06-2408-2708.md` |
| §9.108 | [9.108 Chạy Tương tác ở quy mô 20 máy (25/08/2026)](diary/06-2408-2708.md#9108-chạy-tương-tác-ở-quy-mô-20-máy-25082026) | `diary/06-2408-2708.md` |
| §9.109 | [9.109 Bài nhiều ảnh: bình luận viết từ ảnh 1 (25/08/2026)](diary/06-2408-2708.md#9109-bài-nhiều-ảnh-bình-luận-viết-từ-ảnh-1-25082026) | `diary/06-2408-2708.md` |
| §9.110 | [9.110 Tiền thật của một bình luận, và 6/10 token là chữ nghĩ thầm (25/08/2026)](diary/06-2408-2708.md#9110-tiền-thật-của-một-bình-luận-và-610-token-là-chữ-nghĩ-thầm-25082026) | `diary/06-2408-2708.md` |
| §9.111 | [9.111 Gộp một lượt nháp cho cả link, và cái gộp bị phép đo loại bỏ (25/08/2026)](diary/06-2408-2708.md#9111-gộp-một-lượt-nháp-cho-cả-link-và-cái-gộp-bị-phép-đo-loại-bỏ-25082026) | `diary/06-2408-2708.md` |
| §9.112 | [9.112 Fan-out Riêng lẻ đã giết chống trùng, và bốn bình luận giống nhau đã lên thật (25/08/2026)](diary/06-2408-2708.md#9112-fan-out-riêng-lẻ-đã-giết-chống-trùng-và-bốn-bình-luận-giống-nhau-đã-lên-thật-25082026) | `diary/06-2408-2708.md` |
| §9.113 | [9.113 Gộp bị từ chối thì hỏi gộp lần nữa; và hai test đỏ vì CRLF (26/08/2026)](diary/06-2408-2708.md#9113-gộp-bị-từ-chối-thì-hỏi-gộp-lần-nữa-và-hai-test-đỏ-vì-crlf-26082026) | `diary/06-2408-2708.md` |
| §9.114 | [9.114 5/5 máy, và cái máy hỏng bốn lượt liền là stream kẹt chứ không phải code (26/08/2026)](diary/06-2408-2708.md#9114-55-máy-và-cái-máy-hỏng-bốn-lượt-liền-là-stream-kẹt-chứ-không-phải-code-26082026) | `diary/06-2408-2708.md` |
| §9.115 * | [9.115 Bằng chứng lấy từ web, không lấy từ máy — và ảnh cuối là ảnh quan trọng nhất (26/08/2026)](diary/06-2408-2708.md#9115-bằng-chứng-lấy-từ-web-không-lấy-từ-máy--và-ảnh-cuối-là-ảnh-quan-trọng-nhất-26082026) | `diary/06-2408-2708.md` |
| §9.115 * | [§9.115 tiếp — cột `context_json` giờ có người đọc (26/08/2026)](diary/06-2408-2708.md#9115-tiếp--cột-context_json-giờ-có-người-đọc-26082026) | `diary/06-2408-2708.md` |
| §9.115 * | [§9.115 tiếp — dọn bốn việc treo, và cái nào cũng có cổng (26/08/2026)](diary/06-2408-2708.md#9115-tiếp--dọn-bốn-việc-treo-và-cái-nào-cũng-có-cổng-26082026) | `diary/06-2408-2708.md` |
| §9.115 * | [§9.115 tiếp — lời thoại: có rồi, và cái prompt cũ đã ăn mất nó hai lần (26/08/2026)](diary/06-2408-2708.md#9115-tiếp--lời-thoại-có-rồi-và-cái-prompt-cũ-đã-ăn-mất-nó-hai-lần-26082026) | `diary/06-2408-2708.md` |
| §9.115 * | [§9.115 tiếp — tính năng này đã "xong" mà **không chạy** trên bản cài (26/08/2026)](diary/06-2408-2708.md#9115-tiếp--tính-năng-này-đã-xong-mà-không-chạy-trên-bản-cài-26082026) | `diary/06-2408-2708.md` |
| §9.115 * | [§9.115 tiếp — video giờ được *xem*, và tấm ghép nói ra nó xem được mấy giây (26/08/2026)](diary/06-2408-2708.md#9115-tiếp--video-giờ-được-xem-và-tấm-ghép-nói-ra-nó-xem-được-mấy-giây-26082026) | `diary/06-2408-2708.md` |
| §9.115 * | [§9.115 tiếp — `video_gate` đã chạy máy thật, và nó lôi ra hai lỗi (26/08/2026)](diary/06-2408-2708.md#9115-tiếp--video_gate-đã-chạy-máy-thật-và-nó-lôi-ra-hai-lỗi-26082026) | `diary/06-2408-2708.md` |
| §9.116 | [§9.116 "Nhận điện thoại rồi mà điều khiển không được" — và cái app đã không nói (26/08/2026)](diary/06-2408-2708.md#9116-nhận-điện-thoại-rồi-mà-điều-khiển-không-được--và-cái-app-đã-không-nói-26082026) | `diary/06-2408-2708.md` |
| §9.117 | [§9.117 "Mở thư mục máy còn mở không được" — hai lỗi, và cái log chôn cả hai (26/08/2026)](diary/06-2408-2708.md#9117-mở-thư-mục-máy-còn-mở-không-được--hai-lỗi-và-cái-log-chôn-cả-hai-26082026) | `diary/06-2408-2708.md` |
| §9.118 | [§9.118 Một lần sập giờ để lại một dòng — hai nửa, và cả hai đang trống (26/08/2026)](diary/06-2408-2708.md#9118-một-lần-sập-giờ-để-lại-một-dòng--hai-nửa-và-cả-hai-đang-trống-26082026) | `diary/06-2408-2708.md` |
| §9.119 | [§9.119 Pha 1: hết lỗi đã biết — và bốn chỗ tôi nói quá, ghi lại cho đúng (26–27/08/2026)](diary/06-2408-2708.md#9119-pha-1-hết-lỗi-đã-biết--và-bốn-chỗ-tôi-nói-quá-ghi-lại-cho-đúng-2627082026) | `diary/06-2408-2708.md` |
| §9.120 | [§9.120 Tài liệu: một file 10.385 dòng thành một kho, và bảy khẳng định trái với mã (27/08/2026)](diary/06-2408-2708.md#9120-tài-liệu-một-file-10385-dòng-thành-một-kho-và-bảy-khẳng-định-trái-với-mã-27082026) | `diary/06-2408-2708.md` |
| §9.121 | [§9.121 Codex review sáu lượt: 20 lỗi, và ba lần tôi tự bắt mình (27/08/2026)](diary/06-2408-2708.md#9121-codex-review-sáu-lượt-20-lỗi-và-ba-lần-tôi-tự-bắt-mình-27082026) | `diary/06-2408-2708.md` |
| §9.122 | [§9.122 Một chữ "đã root" trả lời hai câu hỏi, và 9/20 máy trả lời ngược nhau (28/08/2026)](diary/06-2408-2708.md#9122-một-chữ-đã-root-trả-lời-hai-câu-hỏi-và-920-máy-trả-lời-ngược-nhau-28082026) | `diary/06-2408-2708.md` |
| §9.123 | [§9.123 Codex bốn lượt cho vùng Flow: 19 lỗi, và ba lần test của tôi là test rỗng (28/08/2026)](diary/06-2408-2708.md#9123-codex-bốn-lượt-cho-vùng-flow-19-lỗi-và-ba-lần-test-của-tôi-là-test-rỗng-28082026) | `diary/06-2408-2708.md` |
| §9.124 | [§9.124 Một lượt review "thất bại" nằm chờ trên đĩa 26 KB, và ba đường mất việc chưa lưu (28/08/2026)](diary/06-2408-2708.md#9124-một-lượt-review-thất-bại-nằm-chờ-trên-đĩa-26-kb-và-ba-đường-mất-việc-chưa-lưu-28082026) | `diary/06-2408-2708.md` |
| §9.125 | [§9.125 Fleet cắm lại: ba cổng đạt, badge 46.2.42 đo được, và ba tool lạ trên một máy (28/08/2026)](diary/06-2408-2708.md#9125-fleet-cắm-lại-ba-cổng-đạt-badge-46242-đo-được-và-ba-tool-lạ-trên-một-máy-28082026) | `diary/06-2408-2708.md` |
| §9.126 | [§9.126 Composer đo lần đầu trên build 16/20 máy: ô thư viện nằm ngược phía, và cờ "đã chọn đủ ảnh" là cái ElementBox không nhìn thấy (29/08/2026)](diary/06-2408-2708.md#9126-composer-đo-lần-đầu-trên-build-1620-máy-ô-thư-viện-nằm-ngược-phía-và-cờ-đã-chọn-đủ-ảnh-là-cái-elementbox-không-nhìn-thấy-29082026) | `diary/06-2408-2708.md` |
| §9.127 | [§9.127 Codex bốn vùng cho đường đăng bài: 44 lỗi, và mười lăm test của tôi là test rỗng (29/08/2026)](diary/06-2408-2708.md#9127-codex-bốn-vùng-cho-đường-đăng-bài-44-lỗi-và-mười-lăm-test-của-tôi-là-test-rỗng-29082026) | `diary/06-2408-2708.md` |
| §9.128 | [§9.128 Đường đăng bài nối xong, và một lần tôi bác bỏ nhầm phát hiện của người review (30/08/2026)](diary/06-2408-2708.md#9128-đường-đăng-bài-nối-xong-và-một-lần-tôi-bác-bỏ-nhầm-phát-hiện-của-người-review-30082026) | `diary/06-2408-2708.md` |
| §9.129 | [§9.129 Tám chỗ còn nợ, và một cổng chưa ai chạy giấu một lỗi mock trong ảnh chụp (30/08/2026)](diary/06-2408-2708.md#9129-tám-chỗ-còn-nợ-và-một-cổng-chưa-ai-chạy-giấu-một-lỗi-mock-trong-ảnh-chụp-30082026) | `diary/06-2408-2708.md` |
| §9.130 | [§9.130 Lượt review thứ ba: bảy cái đúng, một cái sai về schema, và cái tôi vừa sửa lại hở (30/08/2026)](diary/06-2408-2708.md#9130-lượt-review-thứ-ba-bảy-cái-đúng-một-cái-sai-về-schema-và-cái-tôi-vừa-sửa-lại-hở-30082026) | `diary/06-2408-2708.md` |
| §9.131 | [§9.131 Mười người review song song — năm vùng, mỗi vùng hai người không đọc bài nhau (30/08/2026)](diary/06-2408-2708.md#9131-mười-người-review-song-song--năm-vùng-mỗi-vùng-hai-người-không-đọc-bài-nhau-30082026) | `diary/06-2408-2708.md` |
| §9.132 | [§9.132 Fleet cắm lại: 20/20 qua cổng phần cứng, và ba nhãn cuối của đường đăng bài đo xong (30/08/2026)](diary/06-2408-2708.md#9132-fleet-cắm-lại-2020-qua-cổng-phần-cứng-và-ba-nhãn-cuối-của-đường-đăng-bài-đo-xong-30082026) | `diary/06-2408-2708.md` |
| §9.133 | [§9.133 Đợt cải thiện toàn hệ thống: nối đường Sheet, dọn nợ, và hai chỗ máy thật bác lại kế hoạch (31/08/2026)](diary/06-2408-2708.md#9133-đợt-cải-thiện-toàn-hệ-thống-nối-đường-sheet-dọn-nợ-và-hai-chỗ-máy-thật-bác-lại-kế-hoạch-31082026) | `diary/06-2408-2708.md` |
| §9.134 | [§9.134 Vòng review mười người lần hai: cái tôi vừa viết hôm nay sai ở đâu (31/08/2026)](diary/06-2408-2708.md#9134-vòng-review-mười-người-lần-hai-cái-tôi-vừa-viết-hôm-nay-sai-ở-đâu-31082026) | `diary/06-2408-2708.md` |
| §9.135 | [§9.135 Công tắc hai chiều được đọc trước khi bấm, và musically thông toàn tuyến (31/08/2026)](diary/06-2408-2708.md#9135-công-tắc-hai-chiều-được-đọc-trước-khi-bấm-và-musically-thông-toàn-tuyến-31082026) | `diary/06-2408-2708.md` |
| §9.136 | [§9.136 M7: một bài thật, một cú tap hụt, và cái link đầu tiên đọc về bằng mã production (31/08/2026)](diary/06-2408-2708.md#9136-m7-một-bài-thật-một-cú-tap-hụt-và-cái-link-đầu-tiên-đọc-về-bằng-mã-production-31082026) | `diary/06-2408-2708.md` |
| §9.137 | [§9.137 Ổn định interaction/nurture: bằng chứng, effect và ngân sách được buộc cùng một card (01/09/2026)](diary/06-2408-2708.md#9137-ổn-định-interactionnurture-bằng-chứng-effect-và-ngân-sách-được-buộc-cùng-một-card-01092026) | `diary/06-2408-2708.md` |
| §9.138 | [§9.138 Windows sạch, effect-boundary và UI vận hành (02/09/2026)](diary/06-2408-2708.md#9138-windows-sạch-effect-boundary-và-ui-vận-hành-02092026) | `diary/06-2408-2708.md` |
| §9.139 | [§9.139 Nuôi TikTok: kill app ở cuối phiên và tách log theo số máy (02/09/2026)](diary/06-2408-2708.md#9139-nuôi-tiktok-kill-app-ở-cuối-phiên-và-tách-log-theo-số-máy-02092026) | `diary/06-2408-2708.md` |
| §9.140 | [§9.140 Parity clean-room: thư viện Android, AutoSwipe và chẩn đoán fleet (03/09/2026)](diary/06-2408-2708.md#9140-parity-clean-room-thư-viện-android-autoswipe-và-chẩn-đoán-fleet-03092026) | `diary/06-2408-2708.md` |
| §9.141 | [§9.141 Rời trang rồi quay lại: 20 decoder nhận packet nhưng canvas không vẽ (03/09/2026)](diary/06-2408-2708.md#9141-rời-trang-rồi-quay-lại-20-decoder-nhận-packet-nhưng-canvas-không-vẽ-03092026) | `diary/06-2408-2708.md` |
| §9.142 | [§9.142 Profile automation, Save và điều phối TikTok Android (04/09/2026)](diary/06-2408-2708.md#9142-profile-automation-save-và-điều-phối-tiktok-android-04092026) | `diary/06-2408-2708.md` |
| §9.143 | [§9.143 Mobile MCP: bắt tay thật, nhưng không được đi vòng control plane (04/09/2026)](diary/06-2408-2708.md#9143-mobile-mcp-bắt-tay-thật-nhưng-không-được-đi-vòng-control-plane-04092026) | `diary/06-2408-2708.md` |
| §9.144 | [§9.144 Guard bản nháp không latch và layout laptop có DPI (04/09/2026)](diary/06-2408-2708.md#9144-guard-bản-nháp-không-latch-và-layout-laptop-có-dpi-04092026) | `diary/06-2408-2708.md` |
| §9.145 | [§9.145 UI production, preflight video và ranh giới cleanup canary (05/09/2026)](diary/06-2408-2708.md#9145-ui-production-preflight-video-và-ranh-giới-cleanup-canary-05092026) | `diary/06-2408-2708.md` |
| §9.146 | [§9.146 Mười bài chỉ cần mười máy, và phạm vi trang không còn dính nhau (05/09/2026)](diary/06-2408-2708.md#9146-mười-bài-chỉ-cần-mười-máy-và-phạm-vi-trang-không-còn-dính-nhau-05092026) | `diary/06-2408-2708.md` |
| §9.147 | [§9.147 Bỏ toast nổi, giữ trạng thái trong tầm mắt và gom ba nút chạy về một chỗ (05/09/2026)](diary/06-2408-2708.md#9147-bỏ-toast-nổi-giữ-trạng-thái-trong-tầm-mắt-và-gom-ba-nút-chạy-về-một-chỗ-05092026) | `diary/06-2408-2708.md` |
| §9.148 | [§9.148 Một icon 16×32 px kéo lệch cả hàng Nuôi TikTok (05/09/2026)](diary/06-2408-2708.md#9148-một-icon-1632-px-kéo-lệch-cả-hàng-nuôi-tiktok-05092026) | `diary/06-2408-2708.md` |
| §9.149 | [§9.149 Form hồ sơ đồng nhất và trạng thái sẵn sàng Nuôi TikTok (05/09/2026)](diary/06-2408-2708.md#9149-form-hồ-sơ-đồng-nhất-và-trạng-thái-sẵn-sàng-nuôi-tiktok-05092026) | `diary/06-2408-2708.md` |
| §9.150 | [§9.150 Bản nháp, hồ sơ và tiến độ fleet không còn phụ thuộc tab đang mở (06/09/2026)](diary/06-2408-2708.md#9150-bản-nháp-hồ-sơ-và-tiến-độ-fleet-không-còn-phụ-thuộc-tab-đang-mở-06092026) | `diary/06-2408-2708.md` |
| §9.151 | [§9.151 Tài liệu theo tác vụ, chỉ mục có ngữ nghĩa và dữ liệu trùng được chứng minh (06/09/2026)](diary/06-2408-2708.md#9151-tài-liệu-theo-tác-vụ-chỉ-mục-có-ngữ-nghĩa-và-dữ-liệu-trùng-được-chứng-minh-06092026) | `diary/06-2408-2708.md` |
| §9.152 | [§9.152 UI cam trắng, phạm vi tác vụ và biên Local API (06/09/2026)](diary/06-2408-2708.md#9152-ui-cam-trắng-phạm-vi-tác-vụ-và-biên-local-api-06092026) | `diary/06-2408-2708.md` |

## Cổng

```powershell
python -m unittest scripts.test_build_agents_index scripts.test_check_docs -v
python scripts/build_agents_index.py --check
python scripts/check_docs.py
cargo test -p riviu-managers-phone every_agents_section_citation_resolves --lib --locked
cargo test -p riviu-managers-phone agents_md_stays_a_door --lib --locked
```

Bộ quét chỉ đọc file Git theo dõi; bản sao ignored trong `.agents/`, `.superpowers/`, `target/` không được làm chứng cho cây chính.
File agent mới phải được đưa vào Git trước cổng cuối. Chỉ mục kiểm nội dung chuẩn hoá LF, không lấy CRLF làm thay đổi tài liệu.
