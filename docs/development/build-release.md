# Build và phát hành desktop

[Hướng dẫn phát triển](../developer-guide.md). Nguồn lệnh đầy đủ là
[Desktop CI/CD](../../.github/workflows/desktop-ci-cd.yml); tài liệu này giải thích
thứ tự, đầu vào và điều kiện bàn giao, không tự cấp phép cài/push/release.

## Điều kiện trước build

- Kiểm working tree, source commit, version Tauri/npm/Cargo và toolchain/lock.
- Tạo môi trường Python build riêng đúng phiên bản; không kéo package ngẫu nhiên
  từ Python của máy vào runtime. GUI-service có environment riêng với uv.lock.
- Cấu hình **ứng dụng Google OAuth Desktop** phải có trong release. Dùng biến
  `RIVIU_GOOGLE_OAUTH_CONFIG_JSON`, hoặc `RIVIU_GOOGLE_OAUTH_CONFIG_FILE`, hoặc file
  local gitignored `apps/desktop/src-tauri/google-oauth.local.json`.
- JSON tối thiểu: `{"clientId":"YOUR_DESKTOP_CLIENT_ID.apps.googleusercontent.com"}`.
  `clientSecret` tùy chọn. `pickerApiKey` và `projectNumber` legacy tùy chọn nhưng phải
  hợp lệ khi có. Không chứa account/accessToken/refreshToken. Không in config vào log.
- JSON environment ưu tiên kể cả khi rỗng; release thiếu/sai config phải fail. Debug
  có thể không config để phát triển phần khác; `RIVIU_REQUIRE_GOOGLE_CONFIG=1` kiểm
  yêu cầu release trong dev. Runtime local config đã lưu được ưu tiên để giữ client
  identity của phiên cũ. Không dùng credential của developer để chứng nhận PC mới.

## Chuỗi build Windows

Giữ cùng source/target/config trong cả chuỗi; thao tác với cache Cargo phải tuần tự.
Các script có `--help`; đối số cụ thể theo invocation trong workflow hiện hành.

1. Cài frontend đúng npm lock và Python build requirements trong môi trường cô lập.
2. Stage `yt-dlp` theo nền tảng, tạo manifest nguồn/version/hash. Đây là ngoại lệ
   tải bản mới theo chính sách extractor; cùng commit source không tự bảo đảm cùng binary.
3. `scripts/stage_android_package_tools.py`: stage bundletool/JRE và overlay
   `target/tauri-android-package-tools.conf.json`. Kiểm digest/provenance.
4. `scripts/build_desktop_sidecar.py`: runtime iOS + signer + manifest; tạo
   `target/tauri-sidecar.conf.json`, chạy frozen self-tests.
5. `scripts/build_gui_service.py`, rồi `scripts/test_gui_service_package.py`:
   GUI runtime, OCR models và overlay `target/tauri-gui-service.conf.json`.
6. Build `riviu-deployment-checker` với target/version/config Google tương ứng;
   stage bằng `scripts/stage_deployment_checker.py`. Checker không phải EXE cũ
   tùy ý còn trong cache; hash/version phải khớp bước build cuối.
7. Build app với `RIVIU_DEFAULT_AGENT_MODE=full`, các overlay Full + iOS + Android
   tools + GUI và `--locked`; Windows dùng `tauri build --no-bundle` một lần.
8. Kiểm `--verify-frontend <report-absolute-path>` trên release binary: embedded
   directory có index và đủ asset tham chiếu. Không dùng `C:/...` như frontendDist
   URL; dùng đường tương đối được Tauri diễn giải đúng.
9. `scripts/bundle_windows_installers.py`: NSIS/MSI từ đúng executable đã build,
   overlay WiX per-user; script giữ updater bundle identity. Không build app khác
   giữa hai loại installer. Overlay fast-bundle chỉ đổi nén cho giao nội bộ.
10. Verify/extract package thực, checker, resource/runtime/hash/architecture,
    frontend và startup trên PATH sạch. Ghi manifest và checksum cho artifact giao.

Không bỏ bước để “build được trước”, không tái sinh manifest pin từ input bất kỳ
để làm hash gate xanh. Đừng xóa toàn target; đó còn là nơi giữ evidence và rollback.

`npm run build` ghi `dist/frontend-provenance.json` sau khi TypeScript và Vite cùng
thành công. Khi nhúng frontend, Cargo kiểm lại hash đầu vào và các asset trong dist;
thiếu hoặc lệch biên nhận thì phải build frontend lại. Không chép một dist cũ từ
checkout khác để vượt bước này. Kiểm tra chỉ đọc biên nhận, không chạy Vite lần hai;
devUrl và cargo check không nhúng frontend không cần dist. Biên nhận này chứng minh
nguồn và asset khớp nhau, không thay kiểm tra giao diện/native hay thiết bị thật.

## Tên hiển thị và danh tính bộ cài

Operator thấy tên **Riviu Manager** ở tiêu đề cửa sổ, mục Apps & Features, lối tắt
Start menu/desktop, thư mục Start menu của MSI và tên file bộ cài phát hành. Bản Full
vẫn giữ `productName` "Riviu Manager Full" và `identifier` `com.riviu.manager.full`;
đừng đổi hai giá trị này để "đồng bộ tên":

- Template NSIS của Tauri (tauri-cli 2.11.4) lấy `productName` làm khóa gỡ cài
  `…\Uninstall\<productName>`, thư mục cài mặc định `%LOCALAPPDATA%\<productName>`,
  khóa `Software\riviu\<productName>` dùng để tìm bản cài trước và tên lối tắt.
  UpgradeCode mặc định của MSI sinh từ `<productName>.exe.app.x64`; template WiX
  per-user cũng dùng nó cho `INSTALLDIR` và các khóa HKCU. Đổi `productName` sẽ cài
  bản mới song song với bản cũ (hai mục trong Apps), và trùng khóa/thư mục với
  biến thể cơ sở `com.riviu.manager` vốn đã tên "Riviu Manager".
- `identifier` quyết định thư mục WebView `%LOCALAPPDATA%\<identifier>`. DB,
  credential và log nằm ở `%APPDATA%\riviu-managers-phone` (`state.rs`), không theo
  hai giá trị trên.

Phần hiển thị được đổi bằng các đường Tauri hỗ trợ:

- `app.windows[0].title` cho cửa sổ.
- `scripts/windows-installer-hooks.nsh` cho NSIS: ghi `DisplayName`, đổi tên lối tắt
  `<productName>.lnk` thành `Riviu Manager.lnk` sau cài/cập nhật và khi đóng wizard,
  xóa chúng khi gỡ cài (không xóa khi cập nhật).
- `wix/main-per-user.wxs` cho MSI: đặt tên hiển thị/lối tắt và xóa các lối tắt tên
  cũ còn lại sau major upgrade.
- `published_asset_name` trong collector đổi tên file phát hành.

Thư mục cài vẫn là `%LOCALAPPDATA%\Riviu Manager Full`. Nếu cài cả hai biến thể trên
một máy, chúng trùng tên hiển thị nhưng không trùng dữ liệu. Biến thể cơ sở không
được CI đóng gói. Trên macOS, bundle vẫn là `Riviu Manager Full.app`; chỉ
tiêu đề cửa sổ và tên file phát hành được đổi.

## macOS và phạm vi hỗ trợ

CI có arm64/x64, runtime Python native theo kiến trúc và mapping resource bảo toàn
symlink. Build/codesign/DMG mount kiểm đúng artifact, không dùng sibling .app thay
bộ cài. Hiện ký ad-hoc; chưa Developer ID/notarization. GUI-service được stage/bundle
trong nhánh Windows của CI, **chưa có parity package trên macOS**; không quảng bá
OCR/vision service từ một Python wheel có thể cài.

Build/ký IPA iPhone là quy trình riêng, chỉ trên Mac được phép và theo
[WDA safety](../agents/02-wda-doc-truoc-khi-sua.md). Không sửa IPA/hash/identity khi
chỉ cần build desktop. Artifact manifest có features không chứng nhận device mới.

## CI, updater và release

Workflow hiện chỉ `workflow_dispatch`; push/PR/tag không tự chạy. Chạy thủ công theo
nhánh được duyệt để nhận quality gates và artifacts; không dùng lần CI cũ làm chứng
nhận diff mới. Version phải khớp Tauri/npm/Cargo; release immutable, không clobber.

Chữ ký updater xác minh payload update, không thay Authenticode Windows hoặc
Developer ID/notarization macOS. Windows hiện chưa ký Authenticode; profile nội bộ
của checker cho phép warning này, không được báo production gate đạt.

Bộ cài MSI và NSIS phải nhận đúng loại update; kiểm `latest.json` theo collector.
Update chỉ khi app xác minh fleet/hàng đợi rảnh, drain/nhả trước khi chạy installer.
Không gửi bản cài hay tạo GitHub release nếu chưa có yêu cầu phát hành.

## Hồ sơ bàn giao

Giữ `target/<version>/` cho artifact/report; cache compiler dùng chung target chuẩn.
Ghi source commit, diff nếu dirty, toolchain/lock, overlays, app/checker/resource hash,
installer hash và phạm vi smoke. Không chứa Google account/refresh token/DB vận hành.

Tách kết luận: compile, artifact integrity, installer startup, clean-host device và
public-action acceptance. Mở WebView với zero phones không chứng minh fleet; unit
xanh không thay OAuth thật. Báo cả gate blocked/failed và các điều kiện còn thiếu.
