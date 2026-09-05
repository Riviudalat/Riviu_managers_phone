# Riviu Manager

Ứng dụng desktop Tauri 2 + Rust + React để quản lý và điều khiển dàn điện thoại
qua USB — **iPhone** (pymobiledevice3 + Riviu Agent) và **Android** (adb),
chung một control plane. Bản phân phối hỗ trợ Windows x64, macOS Apple Silicon
và macOS Intel.

Phần mềm của người khác đi kèm trong bộ cài được liệt kê ở [`NOTICE`](NOTICE),
gồm cả một mục ghi rõ chỗ giấy phép **chưa được thẩm định**.

Tài liệu hiện tại ở [docs/README.md](docs/README.md): hướng dẫn vận hành 12 trang,
runbook phát triển/agent, hợp đồng UI và kho lịch sử. [AGENTS.md](AGENTS.md) vẫn là
cửa vào của agent; [chỉ mục §](docs/agents/README.md) dẫn tới số đo và nhật ký mới nhất.
Báo cáo có ngày ở [archive](docs/archive/README.md) không phải trạng thái hiện tại.

## Cài bản dựng

Mỗi lần push lên `main`, workflow **Desktop CI/CD** tạo ba artifact trong trang
GitHub Actions:

- `desktop-windows-x64`: bộ cài `.msi` và `.exe` (NSIS).
- `desktop-macos-arm64`: `.dmg` cho Mac Apple Silicon.
- `desktop-macos-x64`: `.dmg` cho Mac Intel.

Artifact được giữ 30 ngày. Tag dạng `v*` (ví dụ `v0.1.0`) tạo GitHub Release và
đính kèm toàn bộ bộ cài, manifest cùng SHA-256. Tag phải khớp chính xác version
trong Tauri, npm và Cargo; lệch version thì pipeline dừng trước khi phát hành.
Release đã tồn tại không bị ghi đè; muốn phát hành lại phải tăng version và tạo tag
mới.

Bộ cài đã mang sẵn Python runtime, `pymobiledevice3==10.1.0` và
`tidevice==0.12.11`; máy người dùng không cần cài Python hay pip. Trên Windows,
installer tự tải WebView2 bootstrapper khi hệ điều hành chưa có, và bản dựng
link tĩnh CRT nên không cần cài Visual C++ Redistributable.

Đường Android cũng đã mang sẵn: `adb.exe` + hai DLL của nó (platform-tools
37.0.1) và `minicap.apk` cho stream. Không cần cài Android SDK. Máy nào **đã**
có adb thì bản đó được ưu tiên trước bản đóng gói — cố ý, xem "Thứ tự tìm adb"
dưới đây.

### Cập nhật

App **không tự kiểm** bản mới lúc mở — máy farm thường offline và không ai yêu cầu nó gọi
về. Vào **Settings → Bản cập nhật** rồi bấm **Kiểm bản mới**.

Nút **Tải và cài đặt** chỉ bật khi fleet đang rảnh. Đang có phiên Nuôi TT hoặc việc trong
hàng đợi thì nút tắt và panel nói rõ đang chạy cái gì — bộ cài thay chính tiến trình đang
giữ session và lease của các máy. Hàng đợi **không đọc được** cũng tính là đang chạy.

Khi bấm cài: tải xong trước, rồi app dừng mọi phiên và nhả hết máy, rồi mới chạy bộ cài.
Trên Windows app tự đóng và mở lại; trên macOS phải mở lại tay.

**Cài bằng `.msi` hay `.exe` đều cập nhật được, và cập nhật đúng loại của mình.**
`latest.json` mang một khoá riêng cho từng loại bộ cài, nên bản cài MSI nhận MSI và bản cài
NSIS nhận NSIS. Chỗ này từng là một cái bẫy: updater tra `windows-x86_64-msi` rồi mới lùi về
`windows-x86_64`, nên nếu thiếu khoá MSI thì bản cài MSI sẽ **âm thầm cài bản NSIS đè lên** —
một app mà hai mục gỡ cài đặt.

Các prerequisite thuộc hệ điều hành/nhà cung cấp:

- Windows cần Apple Devices hoặc Apple Mobile Device Support để có USB driver và
  dịch vụ usbmux của Apple. Thành phần này không được phân phối lại trong app.
- macOS chạy app bình thường không cần Python. Xcode chỉ bắt buộc khi rebuild hoặc
  ký lại agent iPhone.
- Artifact macOS CI hiện ký ad-hoc. Khi chưa cấu hình Developer ID + notarization,
  macOS có thể yêu cầu xác nhận trong **Privacy & Security** ở lần mở đầu.
- Bộ cài Windows **chưa được ký số**. Lần đầu chạy `.exe` NSIS, SmartScreen có
  thể hiện "Windows protected your PC" và phải bấm **More info → Run anyway**.
  Đây là việc còn bỏ ngỏ, không phải lỗi.

### Máy Android còn cần làm tay

Chỉ còn **một** thứ bộ cài không mang được, và nó là thứ không có cách nào đóng gói:

- **USB driver theo từng model, và cái hộp thoại trên máy.** `adb devices` có thể
  rỗng dù adb hoàn toàn bình thường. Bật **USB debugging** trong Developer options,
  cắm dây, rồi chấp nhận hộp thoại *Allow USB debugging* hiện trên điện thoại. Chưa
  chấp nhận thì adb báo `unauthorized`; dây/hub sạc-only thì báo `offline`.

Mục này từng có một gạch đầu dòng thứ hai nói **hai APK `io.appium.uiautomator2.server{,.test}`
"chưa đóng gói"** và bảo người vận hành tự cài. Điều đó **đã sai từ 16/08/2026**: cả hai
APK nằm trong `sidecars/android/noarch/`, được ghim trong
`android-tools-manifest.json` với `role: agentServerApk` / `agentTestApk`, và
`install_agent_apks` **tự cài chúng** lên từng máy. Ai làm theo hướng dẫn cũ sẽ cài
tay đúng thứ bộ cài đã ship. Trạng thái đóng gói giờ có **một chủ sở hữu duy nhất** —
`sidecars/android/README.md` — và cả file này lẫn AGENTS.md trỏ tới đó thay vì tự
nhắc lại.

### Thứ tự tìm adb

`configured → RIVIU_ADB_PATH → ANDROID_SDK_ROOT → ANDROID_HOME → PATH → bản
đóng gói`. Bản đóng gói xếp **cuối cùng** có chủ ý: máy đã cài Android Studio
hoặc scrcpy thì adb của nó đang giữ adb server ở cổng 5037, và một client khác
revision sẽ buộc `adb server version doesn't match this client; killing...`,
phá session của công cụ khác. adb của người vận hành thắng **nếu nó chạy được**;
bản đóng gói là lưới an toàn cho máy sạch. `RIVIU_MINICAP_APK` cũng vẫn override
được `minicap.apk` đóng gói theo cùng nguyên tắc đó.

## Chạy từ source

```powershell
# Windows PowerShell
py -3.12 -m pip install -r sidecars/pymobiledevice3/requirements.txt
cd apps/desktop
npm ci
npm run tauri:dev
```

Trên macOS, thay `py -3.12` bằng `python3.12`; các lệnh còn lại giữ nguyên.

1. Cắm iPhone và chọn **Trust This Computer**.
2. iOS 17+ cần tunnel `pymobiledevice3` phù hợp với phiên thiết bị.
3. Trên thanh công cụ, bấm nút có tooltip **“Quét lại thiết bị”**.
4. Dùng nút **“Cài / re-sign agent”** khi cần chuẩn bị agent.

Hai bước cuối trước đây ghi là *“Trong Control Center, chọn Refresh devices”* và
*“Cài / Re-sign Riviumanagersphone”*. **Không nhãn nào trong đó tồn tại**: không có
trang nào tên Control Center (trang là “Quản lý cửa sổ”), không có nút nào tên
“Refresh devices”, và tên sản phẩm cũ đã bỏ khỏi UI từ 13/08/2026. Một hướng dẫn
nêu tên một nút không có ở đó là một hướng dẫn người mới không đi qua được.

Mock farm chỉ dùng khi phát triển: `RIVIU_MOCK_DEVICES=1`.

### yt-dlp: bộ cài có, chạy từ source thì chưa

Đường lấy caption / slide / lời thoại của một link TikTok gọi `yt-dlp`. **Bộ cài
mang sẵn nó** (CI tải bản release mới nhất theo nền tảng lúc dựng và fail build nếu
không tải được), nhưng repo **không commit** binary đó — nên một checkout chạy từ
source không có nó, và mọi lượt tra trả `NoBinary` một cách im lặng.

Muốn đường đó chạy khi dev, tải một bản vào `sidecars/yt-dlp/`:

```powershell
# Windows
curl -L -o sidecars/yt-dlp/yt-dlp.exe https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe
```

Cố ý **không ghim theo hash**: TikTok phá extractor theo lịch của nó và cách sửa duy
nhất là một bản yt-dlp mới hơn, nên một bản ghim là một thất bại được ghim. Đây là
ngoại lệ duy nhất trong bộ cài — mọi thứ khác ghim theo byte. Chi tiết:
`sidecars/yt-dlp/README.md`.

## Build bộ cài local

```powershell
# Windows PowerShell
$env:RIVIU_DEFAULT_AGENT_MODE = "full"
py -3.12 -m pip install -r sidecars/pymobiledevice3/requirements-build.txt
cd apps/desktop
npm ci
npm run sidecar:build
npm run tauri -- build `
  --config src-tauri/tauri.full.conf.json `
  --config ../../target/tauri-sidecar.conf.json
```

Trên macOS/Linux, dùng `export RIVIU_DEFAULT_AGENT_MODE=full`, thay `py -3.12`
bằng `python3.12`, rồi chạy cùng các lệnh build với dấu `\` thay cho dấu `` ` ``.

`build_desktop_sidecar.py` tạo runtime native đúng kiến trúc, chạy smoke test và
ghi `runtime-manifest.json`. Package Python không liên quan trong môi trường local
được bỏ qua, nhưng mọi dependency đang hoạt động của runtime phải có mặt và đúng
exact version trong lock. Bản release chính thức dùng Python 3.12.10, Node 24.15.0
và Rust 1.95.0. Không commit thư mục `target/`; CI dựng lại sạch trên từng hệ điều
hành. Push lên `main` không đưa bundle trong `target/` vào Git; workflow sẽ dựng
`desktop-windows-x64` và upload bộ cài `.msi`/NSIS trong Actions trong 30 ngày. Tag
`v<version>` sẽ đính kèm các bộ cài đã verify vào GitHub Release.
IPA Agent là artifact ký riêng theo provisioning/UDID: muốn workflow đóng gói
đúng bản Full thì phải commit `sidecars/wda/RiviuAgent-text.ipa` cùng
`text-manifest.json`; IPA hiện tại dùng profile Xcode-managed 7 ngày, còn
installer Windows không có hạn 7 ngày. Đổi sang iPhone mới hoặc hết hạn profile
vẫn cần build/ký IPA trên Mac trước.

Luồng re-sign legacy trên Mac dùng source WDA 16.0.0 và asset đã khóa hash trong
bundle, sau đó copy sang cache người dùng để build. Nó không tải source upstream
không pin và không ghi vào app đã ký.

## Chạy cổng

Mọi mục §9 trong [nhật ký](docs/agents/README.md#nhật-ký-9x) kết thúc bằng một dòng
“Cổng” phát biểu bằng đúng những lệnh dưới đây, và file này chưa từng liệt kê chúng —
nên một người mới không tái hiện được một cổng nào từ README.

```powershell
# Rust cục bộ: format, lint và test từng crate. Không dùng whole-workspace trên máy
# bị Smart App Control chặn binary vừa link; CI vẫn chạy whole-workspace có --locked.
cargo fmt --all -- --check
cargo test --locked -p riviu-core -- --test-threads=1
cargo clippy --locked -p riviu-core --all-targets -- -D warnings
cargo test --locked -p riviu-managers-phone -- --test-threads=1
cargo clippy --locked -p riviu-managers-phone --all-targets -- -D warnings
cargo test --locked -p riviu-android-driver -- --test-threads=1
cargo clippy --locked -p riviu-android-driver --all-targets -- -D warnings

# Frontend: kiểu, lint, test, build. `tsc -b` bắt những thứ vitest không thấy, vì
# vitest xoá kiểu — nên nó phải chạy sau MỖI file mới, không phải sau mỗi đợt.
cd apps/desktop
npm ci
npx tsc -b
npx oxlint --deny-warnings
npx vitest run
npx vite build
npx playwright test        # cần `npx playwright install chromium` một lần
npm audit --audit-level=high

# Python 3.12: sidecar, media-stage, script đóng gói và probe Gate 0
cd ../..
python3 -m py_compile scripts/build_desktop_sidecar.py `
  scripts/collect_desktop_ci_artifacts.py scripts/test_collect_desktop_ci_artifacts.py `
  scripts/stage_deployment_checker.py scripts/test_stage_deployment_checker.py `
  sidecars/pymobiledevice3/pyinstaller_runtime_hook.py sidecars/signer/riviu_signer.py `
  sidecars/wda/build_and_install.py sidecars/wda/test_build_and_install.py `
  tools/interaction-gate0/probe.py tools/interaction-gate0/test_probe.py `
  scripts/build_agents_index.py
python3 -m unittest -v scripts.test_build_per_user_wix_resources `
  scripts.test_collect_desktop_ci_artifacts scripts.test_stage_deployment_checker `
  scripts.test_stage_android_package_tools scripts.test_xiaowei_provenance `
  scripts.test_build_agents_index scripts.test_check_docs sidecars.pymobiledevice3.test_app_control `
  sidecars.pymobiledevice3.test_rtmmo_lifecycle `
  sidecars.pymobiledevice3.test_media_stage sidecars.signer.test_riviu_signer `
  sidecars.wda.test_build_and_install
python3 -m unittest discover -s tools/interaction-gate0 -p "test_probe.py" -v
python3 scripts/collect_desktop_ci_artifacts.py verify-version
python3 scripts/collect_desktop_ci_artifacts.py verify-android-tools
python3 scripts/build_agents_index.py --check
python3 scripts/check_docs.py
python3 -m pip check

# Phụ thuộc: advisory + giấy phép
cargo deny check
git diff --check
```

Trên máy này `python` là 3.14 còn CI dùng **3.12.10**; nếu một test Python báo thiếu
module (`tidevice`), chạy bằng `python3` — đó là bản 3.12 khớp CI.

### Nghiệm thu trên máy thật

Một số thứ không test nào bắt được — ROM in `ls` kiểu khác, một lease bị lấy trên serial thật.
Chạy **headless**, gọi thẳng hàm production, không lái UI bằng chuột (một lần lái chuột đã
đăng nhầm một bình luận thật):

```powershell
# Trình quản lý tệp: thư mục bị từ chối nói là bị từ chối, tên có dấu nháy đọc nguyên văn,
# size đúng từng byte, đẩy/kéo/xoá đều đọc-lại xác nhận. Ghi/xoá một file /sdcard/Download.
cargo run -p riviu-android-driver --example device_files_gate -- <serial>

# Lease: máy đang bị giữ thì lệnh khác **bị từ chối**, và lồi **nêu tên việc đang giữ máy**.
# Chỉ đọc — không đổi gì trên máy, chạy được giữa ca.
cargo run -p riviu-android-driver --example lease_conflict_gate -- <serial>

# Lối đặc quyền: mỗi máy báo đúng lối nó có (su, hay adb shell đã là root), và
# KHÔI PHỤC GỐC chỉ mở khi có `su`. Chỉ đọc — chạy một lệnh vô hại, không bao giờ
# gọi factory_reset.
cargo run -p riviu-android-driver --example root_route_gate -- [serial]
```

Cả hai in `adb` nào đã giải được và từ đâu trước khi làm gì khác: `0 device(s)` vì
"không thấy máy" và vì "không có adb nào" đã từng bị đọc lẫn nhau một lần.

CI chạy đúng bộ trên, cộng ba job dựng bộ cài cho ba nền tảng. **Push một nhánh
thường không kích hoạt CI** (workflow chỉ nghe `main`, tag `v*`, `pull_request` và
`workflow_dispatch`); chạy đủ cổng trên một nhánh mà không mở PR:

```powershell
gh workflow run "Desktop CI/CD" --ref <tên-nhánh>
```

## Workspace

```text
apps/desktop/            Tauri + React UI (README riêng trong đó)
crates/core/             registry, SQLite, nurture/interaction/Flow
crates/android-driver/   adb, scrcpy, minicap, agent HTTP — đường lái Android
crates/ios-driver/       pymobiledevice3 + WDA (+ mock)
crates/signing/          credential store và luồng ký agent
crates/script-engine/    JSON/Flow runtime
sidecars/android/        adb + APK, ghim theo byte (chủ sở hữu trạng thái đóng gói)
sidecars/riviu-android-agent/  nguồn helper APK
sidecars/yt-dlp/         chỉ trong bộ cài, KHÔNG commit — xem mục trên
sidecars/wda/            agent IPA + manifest
sidecars/pymobiledevice3/
sidecars/signer/
scripts/                 build/attestation/CI artifact tooling
tools/                   probe và tiện ích khảo sát, ngoài đường chạy của app
docs/agents/             nội dung AGENTS.md, chia theo chủ đề + nhật ký §9
docs/re/                 khảo sát genfarmer / xiaowei / rtmmo
docs/verification/       log nghiệm thu trên phần cứng thật
```

`crates/android-driver/` **thiếu hẳn trong bảng này** cho tới 27/08/2026, dù nó là
crate lái cả 20/20 máy trong trại — bảng chỉ kể iOS. Ba thư mục `sidecars/android*`,
`tools/` và `docs/` cũng vắng.

Production oracle vẫn là `sidecars/wda/RiviuAgent.ipa`; bản Full dùng candidate
kết hợp `sidecars/wda/RiviuAgent-candidate.ipa` với `text` và `pushMedia` đã build
theo transaction. **Đọc [`docs/agents/02-wda-doc-truoc-khi-sua.md`](docs/agents/02-wda-doc-truoc-khi-sua.md)
trước khi sửa runtime, WDA, IPA hoặc các gate thiết bị thật** — đó là mục duy nhất mà bỏ
qua có thể làm hỏng thiết bị thật.

## Đăng carousel ảnh

Mở trang **Đăng carousel**, chọn thư mục nội dung một cấp (mỗi thư mục con là
một bài), quét rồi chọn subset bundle và phone. Ảnh `01`, `02`... được giữ đúng
thứ tự; `caption*.txt` được hash nguyên bản; `partners*.xlsx` và file ẩn bị bỏ
qua. Mapping phải một-một theo thứ tự hiển thị. Có thể tạo ngay hoặc lịch một
lần. Media được copy vào staging managed và transfer qua HouseArrest/AFC với
readback size + SHA-256, sau đó Agent native import vào Photos. Nút `Post` mở
composer ảnh TikTok, chọn đúng thứ tự ảnh, nhập caption, bấm Đăng và chờ frame
thoát khỏi composer trước khi ghi `Succeeded`; chỉ khi đó asset import mới được
cleanup. Lỗi sau khi đã bấm Đăng được ghi `Uncertain` để tránh đăng trùng.
