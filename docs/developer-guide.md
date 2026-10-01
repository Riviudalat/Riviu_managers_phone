# Hướng dẫn phát triển

## Context và ranh giới

Riviu là ứng dụng desktop điều khiển thiết bị có effect không thể coi là idempotent.
Rust/Tauri/React/SQLite là stack hiện hành. Không tạo controller hoặc scheduler mới
để né ownership; không đổi stack để phù hợp ví dụ của một skill.

| Vùng | Owner | Quy tắc |
|---|---|---|
| UI/IPC | `apps/desktop/src/api.ts`, React pages/components | Typed request, stale-response guard; không gọi driver riêng |
| Bootstrap/lifecycle | `apps/desktop/src-tauri/src/state.rs`, commands | Admission/shutdown, injection runtime, owner theo UDID |
| Device control | `crates/core/src/device_control/`, `device_work.rs` | Lease/context có identity, cancellation drain, quarantine |
| Android/iOS | `crates/android-driver/`, `crates/ios-driver/` | Transport, input và observation; không tự cấp quyền nghiệp vụ |
| Persistence | `crates/core/src/db/` | Transaction/CAS, immutable identity/revision, intent/receipt/outbox |
| Automation | core nurture/interaction/publish/flow/orchestration | Mục tiêu và verifier riêng; ACK không thay proof |
| Perception | core `ui_automation`/`app_automation`, `sidecars/gui-service` | Resolver trả ứng viên, engine thực thi; unknown không là absent |
| Packaging | CI, `scripts/`, `deployment-checker` | Pin/hash, frontend embedding, runtime sạch và installer đúng loại |

`DevicePlatform` khác `SocialNetwork`; TikTok là network được implement, Instagram/
Threads từ chối trước effect. Flow thiết bị là graph primitive; Điều phối gọi engine
nghiệp vụ. Không biến node UI thành proof Post/Send.

## Chuẩn bị

[Toolchain, dependency và lệnh kiểm](development/testing.md). Bảo toàn diff của
người dùng, dùng nhánh công việc và đọc instructions trong `AGENTS.md`.
Mọi sửa WDA/iOS phải đọc toàn bộ [WDA safety](agents/02-wda-doc-truoc-khi-sua.md).

## Quy trình sửa và xác nhận

1. Xác định contract/source owner và reproduction, phân biệt lỗi code/môi trường.
2. Viết regression đi qua đường thực; expected độc lập với implementation.
3. Sửa hẹp, giữ deadline/cancel/ownership/uncertainty. Không replay effect mất ACK.
4. Chạy focused gate trước, gate rộng theo blast radius; baseline đỏ báo riêng.
5. Nghiệm thu UI bằng Tauri smoke cô lập; phone/public effect chỉ khi có phép riêng.
6. Reviewer kiểm diff, scenario sai và evidence; main xác minh findings trước sửa.
7. Cập nhật đúng tài liệu sở hữu hành vi, không chép nhật ký thay đổi vào mọi guide.

## Cổng theo thay đổi

[Testing matrix và bằng chứng](development/testing.md#cổng-theo-thay-đổi).
Unit/fixture/browser mock/Tauri/device/installer là các tầng riêng. Build thành công
không chứng minh vận hành. Không tắt verifier hoặc sửa assertion để làm cổng xanh.

## Smoke giao diện Tauri cô lập

Dùng `RIVIU_UI_SMOKE=1`, `RIVIU_MOCK_DEVICES=1` và `RIVIU_UI_SMOKE_DIR` tuyệt đối
chưa tồn tại; backend tự claim scratch local. Mode chỉ có ở debug, giữ DB/log/
WebView profile riêng, credential RAM, không USB/sidecar/API/worker nghiệp vụ và
allowlist IPC. Chỉ đặt mock hoặc đổi data-dir không thay được isolation này.

Skill launcher mặc định chọn smoke. Debug background/CDP loopback được phép chỉ
trên đúng process/scratch đã chứng minh. CDP không tự có nghĩa read-only; không gọi
IPC ngoài allowlist. Không chạy app thật để thay thế smoke khi gate bị chặn.

## Bộ cài Windows

[Build/release và provenance](development/build-release.md). Tái sử dụng một app
compile cho NSIS/MSI qua script bundler, giữ metadata updater đúng loại. Không
`cargo clean`; dùng cache chuẩn và build tuần tự theo [cache script](../scripts/dev_compile_cache.ps1).

## Bản đồ trách nhiệm

[Contract chi tiết](development/contracts.md) giữ account/target proof, session epoch,
recovery, ledger, Sheet, Flow composition và shutdown. Source/version hiện hành thắng
snapshot lịch sử; khi tài liệu mâu thuẫn không làm theo assertion lịch sử để bỏ guard.

### Hợp đồng Publish

- `publish_start` ghi receipt trước chuẩn bị chậm; status đối soát cùng request ID.
- `Submitted` khác canonical verified; thiếu link không cấp quyền Post lại.
- Sheet, cleanup và verifier có scope/worker riêng, không mở composer để retry outbox.
- Handoff theo selected-device/assignment, không ảnh hưởng sibling ngoài scope.
- Proof Sheet có thể tái dùng tối đa năm phút theo binding local; delivery vẫn kiểm
  remote protocol/receipt. Cache không chứng minh quyền remote chưa đổi.

[Start/handoff](publish-start-handoff.md) · [Contract thực thi](development/contracts.md).

## Xác minh Publish và kết quả qua restart

[Luồng bằng chứng và phục hồi](development/contracts.md#xác-minh-publish-và-kết-quả-qua-restart).
Không dùng tuổi campaign/HTTP ACK/ảnh cũ thay account, caption, thời điểm và link.

## Nghiệm thu Publish qua ứng dụng đang chạy

[Runbook harness inspect/preflight/submit/observe](development/publish-acceptance.md#nghiệm-thu-publish-qua-ứng-dụng-đang-chạy).
Inspect không mở thiết bị; preflight có thiết bị/network; submit có public effect.
Chỉ dùng IPC của app đang sở hữu máy, không mở driver thứ hai hoặc restart upload
để tạo CDP. Mất ACK giữ nguyên intent/requestId/report-dir.

## Hội thoại theo phiên

[Flow, connectors, scripted conversation và comment verification](development/contracts.md#hội-thoại-theo-phiên).
Flow Sheet vẫn cần Apps Script legacy, chưa dùng OAuth direct; không sửa DB để bỏ điều kiện.

## Tích hợp TypeSafe

[Contract TypeSafe](development/contracts.md#tích-hợp-typesafe). Model judgment không
thay quyền thao tác. Đọc skill và live docs khi đổi API/model; fixture tiếng Việt và
ngưỡng phải được đánh giá theo hậu quả, không coi confidence là chắc chắn đúng.

## Tài liệu và công cụ agent

[Toolkit setup](agent-toolkit-setup.md) · [Runbook agent](agents/agent-runbook.md) ·
[UI contract](ui-reference-matrix.md) · [Historical snapshots](archive/technical-snapshots-2026-10-01/README.md).
Tài liệu hiện hành giữ một owner cho mỗi quyết định; snapshot có ngày chỉ giải thích
lý do/số đo. Giữ immutable evidence, không sửa digest/PASS cũ để khớp build mới.
