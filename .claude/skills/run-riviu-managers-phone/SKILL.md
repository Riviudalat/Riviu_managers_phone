---
name: run-riviu-managers-phone
description: Build, launch, screenshot and drive the Riviu Manager Tauri desktop app on Windows, plus run the CI check suite. Use when asked to run, start, build, screenshot, click through, or verify the desktop app / iPhone farm UI, or to confirm a change in crates/ or apps/desktop actually works in the real app.
---

# Run Riviu Manager

> Renamed 13/08/2026: the product is **Riviu Manager** (org: Riviu Tech), window title
> `Riviu Manager`, identifier `com.riviu.manager`. The **iPhone agent** kept its old
> identity on purpose — `com.riviu.managersphone.agent.xctrunner` and
> `sidecars/wda/Riviumanagersphone.ipa` are hash-pinned artifacts, so do not "finish"
> the rename there.

Tauri 2 + Rust + React desktop app that drives a farm of USB-connected phones. As of
`b3223f0` that is **two platforms behind one control plane** — iPhones via
`crates/ios-driver` (pymobiledevice3 + the Riviu Agent) and Android via
`crates/android-driver` (adb), multiplexed by `crates/core/src/driver_multiplex.rs`.
Everything below was verified against the iPhone path. All paths are **relative to the
repo root**.

There is no CDP endpoint and no Playwright `_electron` handle: `tauri.conf.json`
sets `app.windows[0].create: false` and the WebView2 window is built in Rust
(`apps/desktop/src-tauri/src/lib.rs:36`). The handle is therefore Win32, wrapped in
**`.claude/skills/run-riviu-managers-phone/driver.ps1`** — it raises the window by
z-order, captures its screen rectangle, and injects real mouse/keyboard input.

Verified on Windows 11 Pro 26200, 20 cores, 100% display scaling, against the
iPhone 8 (`iPhone10,1`, iOS 16.7.15) documented in AGENTS.md.

## Prerequisites

Node 24 and VS Build Tools with MSVC were already present. These were not:

```powershell
# Windows SDK. MSVC 14.51 + link.exe existed but Windows Kits\10 had NO Lib/Include,
# so every MSVC link failed. Needs admin (UAC).
winget install --id Microsoft.WindowsSDK.10.0.26100 --source winget `
  --accept-package-agreements --accept-source-agreements --disable-interactivity

# Rust (user-level, no UAC). rustup-init is not on winget in a form that pins the host triple.
Invoke-WebRequest -Uri "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe" `
  -OutFile "$env:TEMP\rustup-init.exe"
& "$env:TEMP\rustup-init.exe" -y --default-toolchain stable-x86_64-pc-windows-msvc --profile minimal
& "$env:USERPROFILE\.cargo\bin\rustup.exe" component add clippy rustfmt

# Python 3.12 for the pymobiledevice3 sidecar (pinned pymobiledevice3==10.1.0).
winget install --id Python.Python.3.12 --source winget `
  --accept-package-agreements --accept-source-agreements --disable-interactivity

# Install the sidecar requirements INTO 3.12 specifically. find_python()
# (crates/ios-driver/src/pmd.rs) probes candidates in order - py -3.12, python3.12,
# py -3, python3, python - and accepts the first whose
# `importlib.util.find_spec('pymobiledevice3')` succeeds, so the interpreter that got
# these requirements is the one that gets used.
$py = "$env:LOCALAPPDATA\Programs\Python\Python312"
& "$py\python.exe" -m pip install -r sidecars\pymobiledevice3\requirements.txt

# Apple usbmux (USB driver + port 27015). Not redistributable in the app.
winget install --id 9NP83LWLPZ9K --source msstore `
  --accept-package-agreements --accept-source-agreements --disable-interactivity

cd apps\desktop; npm ci; cd ..\..
```

Confirm the interpreter the app will pick — `py -3.12` is candidate #1, and winget's
Python install registers with the `py` launcher:

```powershell
py -0
py -3.12 -c "import importlib.util,sys; sys.exit(0 if importlib.util.find_spec('pymobiledevice3') else 1)"
```

`py -0` must list `-V:3.12` and the probe must exit 0. A bare `python` here is 3.14
without the requirements and exits 1 — that is correct, the app rejects it and moves
on. `driver.ps1 status` prints the same resolution alongside the app state.

## Build

```powershell
cd apps\desktop\src-tauri
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo build --locked -p riviu-managers-phone
```

First build ~4m (~460 crates, includes bundled SQLite C). Later relinks ~19s.
`.cargo/config.toml` sets `-C target-feature=+crt-static` for the MSVC target so the
app needs no VC++ redistributable; because that is a rustflag, **checking out a commit
that adds or removes it invalidates the whole cache and forces a full rebuild**.
After changing branches also re-run `npm ci` — `b3223f0` moved the lockfile by 154
packages.
`driver.ps1 launch` builds too, so this step is optional — run it first when you
want compile errors on a clean exit code instead of buried in the dev log.

## Run — agent path

```powershell
$d = ".claude\skills\run-riviu-managers-phone\driver.ps1"
powershell -NoProfile -ExecutionPolicy Bypass -File $d launch      # add --mock for RIVIU_MOCK_DEVICES=1
powershell -NoProfile -ExecutionPolicy Bypass -File $d wait 300
powershell -NoProfile -ExecutionPolicy Bypass -File $d status
powershell -NoProfile -ExecutionPolicy Bypass -File $d shot 01-home
powershell -NoProfile -ExecutionPolicy Bypass -File $d click 116 147
powershell -NoProfile -ExecutionPolicy Bypass -File $d fill 594 154 iPhone
powershell -NoProfile -ExecutionPolicy Bypass -File $d log 40
powershell -NoProfile -ExecutionPolicy Bypass -File $d stop
```

| Command | What it does |
|---|---|
| `launch [--mock]` | `npm run tauri:dev` detached; returns when the window exists (~30s warm). Records the launcher pid for `stop`. |
| `wait [seconds]` | Blocks until the window is Responding on 4 consecutive polls. Default 300. |
| `status` | app pid/rect/foreground, vite :5173, usbmux :27015, python3/python/cargo/tidevice resolution, and every live `riviu_pmd.py` sidecar with its full argv. |
| `shot <name>` | PNG to `target/run-skill/<name>.png`. Warns if under 20 KB (unpainted webview). |
| `click <x> <y>` | Window-relative left click. Activates the window first (see Gotchas). |
| `fill <x> <y> <text>` | Click that point **and** type, in one process — the only reliable way to enter text. Hashes a 280x32 patch around the point before and after, retries once if nothing changed, and warns if still nothing. Accepts SendKeys syntax, so `fill 594 154 "^a{DEL}"` clears a field. |
| `type <text>` / `key <keys>` | SendKeys for follow-up keystrokes. Both **fail closed with exit 1** unless the app is already foreground, so a stray keystroke cannot land in another app. Expect them to refuse unless you just ran `fill`. |
| `devices` | Runs the **iOS** sidecar's `list` under a 60s cap — the fastest check that USB + pairing work. |
| `android` | Resolves adb by the app's own precedence, then `adb version`, `adb devices -l`, and per-device model / Android release / `wm size`. Warns explicitly on `unauthorized` (accept the prompt on the phone) and `offline`. |
| `usbmux` | Starts Apple's usbmux provider and waits for :27015. |
| `occlusion [--at x y]` | Diagnostic for the capture guard: is the app window clear right now, or which top-level window owns a given screen pixel. Reports the **resting** state and does not raise, so on a busy desktop it normally says OCCLUDED — that is fine, `shot`/`click` raise first. |
| `log [n]` | Tails `target/run-skill/tauri-dev.log`. |
| `stop` | WM_CLOSE (lets the app run its own exit ordering), then reaps the npm/tauri/vite chain scoped to this repo, then reports leftover sidecars. |

Screenshots and the dev log land in `target/run-skill/` (gitignored). **Open the PNG
and look at it** — a black frame at ~38 KB means the webview has not painted.

### Verified coordinates (window-relative, 1456x939 window)

The window position moves between launches, so never use screen coordinates —
the driver converts for you.

| Target | Coords | Verified |
|---|---|---|
| Sidebar "Quản lý cửa sổ" | `116 147` | yes |
| Sidebar "Cài đặt" (last item) | `150 641` | yes |
| Device filter input | `594 154` | yes (via `fill`) |

For anything else: `shot`, open the PNG, read the pixel offsets straight off it.

The navigation is Vietnamese as of `b3223f0` — `Kho nội dung`, `Trung tâm ứng dụng`,
`Tác vụ`, `Đăng bài`, `Dữ liệu`, `Tài khoản`, `Cài đặt`, and the sidebar summary is
`Tổng quan`. Row geometry did not move in that rename, but the sidebar can collapse
(chevron at the bottom), which shifts every x — `shot` first if a click misses.

### What a healthy run looks like

`status` shows three sidecars — `wda-proxy` (control `8916`, MJPEG `9094`,
`--backend riviu-agent`), `__xctest`, and `stream --fps 24 --mode mjpeg`. The device
tile footer reads `● Live  USB  iPhone10,1 · 16.…` and **Settings** reports the agent
`Sẵn sàng` with Auth/MJPEG/Session all `Yes`. `devices` returns:

```json
{"devices": [{"udid": "a99f4bd9...", "name": "iPhone 8 (Global)", "model": "iPhone10,1", "iosVersion": "16.7.15", "connection": "usb", "battery": null}]}
```

## Checks (what CI actually gates)

`.github/workflows/desktop-ci-cd.yml` runs exactly these on `windows-2025`:

CI runs whole-workspace commands. **Do not copy them onto this machine** — run the
per-crate list below instead, and see why underneath.

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo fmt --all -- --check

# Per crate, never --workspace. Counts as of 25/08/2026.
cargo test -p riviu-core                                     # 718 lib + 27 + 1, ~100 s
cargo test -p riviu-managers-phone                           # 179
cargo test -p riviu-android-driver                           # 185
cargo clippy -p riviu-core --all-targets -- -D warnings
cargo clippy -p riviu-managers-phone --all-targets -- -D warnings
cargo clippy -p riviu-android-driver --all-targets -- -D warnings

cd apps\desktop
npm run lint
npm test         # 682 tests
npm run build    # tsc -b && vite build
```

**Why per crate.** A whole-workspace `cargo test` or `cargo clippy` on this host is
killed by Smart App Control part way through and reports a link error that looks like a
code fault. The per-crate list covers the same ground and finishes. Never turn Smart App
Control off to make the workspace command work.

**`npm` may not run at all here.** `npm ci` was refused by the OS on 25/08/2026 —
`The operation was rejected by your operating system` — leaving `apps/desktop/node_modules`
with 11 packages and no `.bin`, so `npm test`, `npm run lint` and `npm run build` all fail
with `'vitest' is not recognized`. That is the machine, not the tree: the frontend gates run
in CI on every pull request. A Rust test does cover the Rust↔TypeScript boundary
(`the_frontend_types_describe_the_same_fields_the_backend_sends` reads `types.ts` as text),
so a type added on one side and forgotten on the other is caught by `cargo test -p riviu-core`.

`cargo test -q -p riviu-managers-phone` alone is the fast inner loop.
**Stop the app first** — a running `riviu-managers-phone.exe` is locked and cargo
cannot relink it.

Three `flow::evidence` tests fail **under load** and pass when run alone: their deadlines
are one second of real time and 700 sibling tests starve them. `cargo test -p riviu-core
--lib flow::evidence` on its own is the check; a failure there while the rest of the suite
is green is not a regression.

`npm run test:e2e` is **not** in CI but passes locally (6 specs, ~14s) after
`npx playwright install chromium`. It starts its own vite on `127.0.0.1:1421` with a
mocked Tauri IPC (`e2e/fixtures/tauriMock.ts`), so it is the only way to drive the
frontend with no phone attached. The `[vite] Unhandled error: ResizeObserver loop
completed with undelivered notifications` lines it prints are noise, not failures.

## Run — human path

```powershell
cd apps\desktop
npm run tauri:dev
```

A window opens; Ctrl-C in the terminal stops it. No PATH setup is needed: `python3`,
`cargo` and `tidevice` already resolve correctly in a fresh shell.

## Gotchas

- **MSVC present but linking fails.** VS Build Tools shipped `link.exe` and MSVC
  14.51 while `C:\Program Files (x86)\Windows Kits\10` had no `Lib`/`Include` — the
  SDK was registered (`KitsRoot10`, `ProductVersion 10.0.26100`) with only the App
  Certification Kit installed. Check for
  `Windows Kits\10\Lib\10.0.26100.0\um\x64\kernel32.Lib` before believing the SDK is there.
- **Interpreter choice is by capability, not by name.** `find_python()` probes
  `py -3.12`, `python3.12`, `py -3`, `python3`, `python` in order and takes the first
  where `find_spec('pymobiledevice3')` succeeds, naming every rejected candidate in
  its error. `scripts/run_python.mjs` does the same for the build script. So a 3.14
  on `python` is harmless — but if you install the requirements into the *wrong*
  interpreter you get "no Python with pymobiledevice3 installed" rather than a silent
  misfire. Install them into 3.12.
- **First cold connect freezes the UI for ~2 minutes.** `_list_devices()` in
  `sidecars/pymobiledevice3/riviu_pmd.py:337` awaits `create_using_usbmux()` with no
  timeout, and `lib.rs:57` runs `block_on(AppState::bootstrap(..))` inside `.setup()`.
  The title shows "(Not Responding)" over a black webview. **Wait it out** — it
  recovers and the device comes up Ready. Only the first connect after pairing does this.
- **`SetForegroundWindow` is refused** for a non-foreground caller, so it silently
  does nothing and `CopyFromScreen` captures whatever window is actually on top —
  you get a screenshot of your own terminal. The driver raises with
  `SetWindowPos(HWND_TOPMOST)` and restores `HWND_NOTOPMOST` after.
- **Do not use `Process.MainWindowHandle` for this app.** It is documented as "the
  first top-level window", and a Tauri/WebView2 process owns invisible helper
  windows: mid-session it started returning a **16x16 window at 0,0 with an empty
  title**. Everything downstream then operated on the top-left corner of the *screen*
  — `status` printed a 16x16 rect and the occlusion guard "found" a browser there and
  refused to work, all without a single error. The driver now enumerates the process's
  top-level windows and picks the visible one over 40000 px² preferring the title
  `Riviu Manager`. If `status` ever shows a tiny rect or an empty title, that is
  this bug returning.
- **`HWND_TOPMOST` is not sufficient either, and failure is silent.** Another topmost
  window can still sit above ours; a `shot` here captured a full-screen Chrome window
  and looked like a perfectly valid PNG. Every raise is now followed by
  `WindowFromPoint` + `GetAncestor(GA_ROOT)` at five points inside the rect; if any
  belongs to another window the driver clicks the title bar, re-raises, and **throws
  rather than capture the wrong thing**. Check the resting state any time with
  `driver.ps1 occlusion`. A plausible file size proves nothing — this is why you open
  the PNG.
- **The first click on an inactive window is swallowed by activation** and never
  reaches the element. The driver activates with a click on the inert title bar
  first, deliberately *not* on the target — activating by clicking the target
  double-fires whatever is under it.
- **`click` then `type` as two driver calls loses the text.** Each invocation is a
  new process; between them another app can take focus and eat the keystrokes — this
  happened here, the text landed in an unrelated window. Focus drift on a real
  desktop is constant, not rare: 9 seconds after a clean `launch`, the foreground
  window was a chat client, not the app. Use `fill x y text`, which does the click
  and the typing inside one process. `type`/`key` deliberately exit 1 rather than
  send keystrokes to whatever happens to be focused.
- **The focusing click itself loses races, silently.** A `fill` at the correct
  coordinates reported success while the field stayed empty and the focus ring
  remained on the previously-clicked nav item; an identical retry worked. Nothing in
  the mouse/SendKeys path reports this, which is why `fill` now hashes the field
  region before and after and retries. Treat any UI assertion that is not backed by
  a pixel change or a fresh `shot` you looked at as unproven. Related: another
  process can hold foreground even while the app is `HWND_TOPMOST`, so captures can
  contain slivers of other windows along the edges.
- **usbmux is not a Windows service.** The Store package `AppleInc.AppleDevices`
  provides it via `AppleMobileDeviceProcess.exe`, which only starts once the app or
  its `AMPDevicesAgent` has been launched. Executing that exe directly fails with
  "Access is denied" (WindowsApps ACLs); launch by AUMID:
  `explorer.exe shell:AppsFolder\AppleInc.AppleDevices_nzyj5cx40ttqa!AMPDevicesAgent`.
  An unpaired iPhone appears only as a WPD device and
  `C:\ProgramData\Apple\Lockdown` holds no `<UDID>.plist`.
- **Keep `--all-targets` on clippy.** Several helpers are only reached from
  `#[cfg(test)]` code, so dropping the flag turns them into `dead_code` errors under
  `-D warnings`. The repo pins no toolchain (no `rust-toolchain.toml`) while releases
  are built with Rust 1.95.0, so a newer stable can surface lints CI never saw — this
  happened at `fa8ecca`, where clippy 1.97 flagged an `unnecessary_cast` in
  `crates/core/src/nurture/actions.rs`. Fixed as of `b3223f0`; if a fresh stable
  breaks clippy again, that is the pattern.
- **e2e is not a CI gate, so it drifts.** At `fa8ecca` it failed 5/6 here (two
  win32 snapshot diffs plus a Playwright strict-mode violation where a row locator
  resolved to 2 elements); `b3223f0` refreshed the baselines and the specs, and it is
  6/6 now. Nothing in `.github/workflows/desktop-ci-cd.yml` runs it, so expect it to
  rot again. Failure details land in
  `apps/desktop/node_modules/.tmp/playwright-results/*/error-context.md`.
- **`stop` used to leak a `cargo run` wrapper.** `tauri dev` launches the app through
  `cargo run --no-default-features --color always --`, whose command line names
  neither the repo nor tauri, so no fingerprint scan can attribute it. Two of them
  survived ~78 minutes with a dead parent before this was caught. `stop` now reaps
  `cargo.exe` inside the launcher's process tree and warns about strays it cannot
  prove ownership of. If cargo commands ever hang waiting on
  `target/debug/.cargo-lock`, look for exactly this.
- **`tauri dev` watches `src-tauri/` and restarts the app on any touch there** —
  including a `git checkout -- apps/desktop/src-tauri/Cargo.toml` that changes no
  content, only line endings. The window vanishes and a new pid appears a few
  seconds later, so a `status` in that gap says "not running" with vite still up.
  Re-run `wait` instead of concluding it crashed; the log will show
  `Finished dev profile in 0.5s` followed by a fresh `Running`.
- **The window position changes every launch** (observed 104,104 / 156,141 / 26,26).
  Never cache screen coordinates — the driver converts window-relative for you.
- **Never `pkill`-style broad matches.** AGENTS.md §2.8 requires PID **and**
  command-line fingerprint. A bare `*vite*` match reaped six processes here; an
  unfiltered walk of the launcher's process tree also swept up `conhost.exe` and an
  unrelated `powershell.exe`. `stop` now requires name ∈ {node,cmd} **and** the
  command line to contain this repo's path.
- **A diagnostic that greps for `riviu_pmd.py` matches its own command line** and
  reports a phantom leaked sidecar. Filter on `Name -like 'python*'` too.
- **In PowerShell 5.1, do not `2>&1` a native exe.** Cargo writes progress to stderr;
  redirecting wraps each line in a `NativeCommandError` and flips `$?` to false even
  on exit 0.
- **Never capture binary stdout with PowerShell `>`.** It applies text encoding and
  silently corrupts the bytes — the repo warns about the same trap for `screencap`
  (`adb.rs`), and it bit this skill's author: `adb exec-out screencap` measured
  15,552,021 bytes through `>` versus the true 10,368,016. Redirect through
  `cmd /c "... > file"`, or write on the device and `adb pull`. A stable byte count
  across runs is **not** evidence the bytes are intact — check the magic
  (`89 50 4E 47` PNG, `FF D8 FF` JPEG).
- **Android joins only when `adb` runs.** `detect_driver` shells out to `adb version`;
  if that fails the backend sits out with `android_unavailable_reason` set, **separate**
  from the iOS `driver_degraded_reason`. `RIVIU_ADB_PATH` points at the adb executable
  itself (not its directory) and outranks `ANDROID_SDK_ROOT`/`ANDROID_HOME`, then PATH;
  `driver.ps1` puts platform-tools on PATH for you and `driver.ps1 android` reports the
  state. Verified here on a Redmi Note 12 (`23021RAAEG`, Android 15, 1080x2400): it
  appears in the fleet as its own tile. minicap is bundled in the installer
  (`sidecars/android/noarch/minicap.apk`); a clean Windows install streams
  Android tiles as `● Live` without `RIVIU_MINICAP_APK`. Click a tile to open
  the centered control overlay — taps ride the live stream and must not park it
  or foreground TikTok (AGENTS.md §9.48).
- **The mixed fleet holds.** With an iPhone 8 and the Redmi attached at once, one
  `DeviceControlPlane` shows both tiles side by side. After minicap was bundled
  (AGENTS.md §9.27) both tiles are `● Live` and the summary reads `Thiết bị 2/2`.
  The older `1/2` / Android `● Error` reading was the missing-APK-env bug, not a
  multiplexer bug. Sampled every 15s for ~45s, the dashboard region hashed
  byte-identical every time, so the failure AGENTS.md §9 warns about (two planes
  polling independently and taking turns deleting each other's devices, because
  `DeviceRegistry::upsert_many` replaces the whole vector) does **not** happen.
  If you ever see the device count oscillate, that is the bug to suspect.
- **What Android still cannot do here, and why.** Do not read the above as a working
  automation path. (1) `crates/android-driver/examples/probe.rs` hardcodes
  `com.zhiliaoapp.musically`, but a SEA phone carries `com.ss.android.ugc.trill`, so
  the probe cannot drive TikTok on such a device as shipped. (2) `agent.rs` talks HTTP
  to a resident `appium-uiautomator2-server` over `adb forward`; if
  `io.appium.uiautomator2.server{,.test}` is not installed, every `find_and_tap` /
  `read_text` / text-input path is unavailable. (3) A foreign UiAutomator
  instrumentation on the phone (this one had `com.genfarmer.uiautomator.test`) is the
  Android analogue of the 3uTools/XCTest clash in §2.9 — only one can hold the
  accessibility connection. (4) `screen::CALIBRATED_LAYOUTS` holds exactly
  `iphone8-portrait-v1`, so nurture refuses any other screen class by design. The
  desktop's only TikTok constant is the **iOS** bundle `com.ss.iphone.ugc.Ame`.
  See AGENTS.md §9/§10 and `docs/ANDROID_PROBE_REPORT_2026-08-09.md`.
- **`wm size` can print one line or two.** AGENTS.md says read `Override`, measured on
  a fleet that always had one; this Redmi prints only `Physical size: 1080x2400`.
  `parse_wm_size`/`parse_wm_density` handle both — their own tests cover the
  single-line case, so this is not a bug to "fix".
- **Do not run the app and the `live_nurture_test` harness at once** — they fight
  over USB (AGENTS.md). Quit 3uTools too: it runs its own XCTest runner
  (`notes.3u`) and iOS permits only one XCTest session (AGENTS.md §2.9).
- **`apps/desktop/src-tauri/Cargo.toml` shows as modified with an empty diff.** The
  repo has `core.autocrlf=true` and **no `.gitattributes`**, so after a cargo/tauri run
  that file reports `M` while `git diff --numstat` returns nothing — pure EOL churn.
  `git checkout --` clears it and it comes back on the next build. Do not "fix" it by
  committing the file; AGENTS.md §3.9 is explicit that line-ending churn must not be
  baked into commits (`prepare.py` pins `git -c core.autocrlf=false` for the same
  reason). Also note `git checkout` on that path is a watched file, so it makes
  `tauri dev` rebuild and restart the app.
- **`.gitignore` had `.claude/`**, which would have hidden this skill. It is now
  `.claude/*` + `!.claude/skills/`, so local settings stay ignored. Don't revert that
  line or the skill disappears from git.
- **Device-mutating buttons are not exercised here.** `Agent`/`Repair` on the device
  tile install or re-sign the agent IPA on a real iPhone, and the free Apple
  Developer cert lasts 7 days (AGENTS.md §4.0). Ask before pressing them.

## Troubleshooting

| Symptom | Fix |
|---|---|
| Screenshot shows your terminal, not the app | You raised with `SetForegroundWindow`. Use `driver.ps1 shot`, which uses `HWND_TOPMOST`. |
| ~38 KB black PNG, title "(Not Responding)" | Cold bootstrap is blocking. `driver.ps1 wait 300`, then shoot again. |
| Click does nothing, page unchanged | The click activated the window instead. `driver.ps1 click` handles this; if you rolled your own, activate first. |
| Typed text vanished / went elsewhere | Use `fill x y text`, not `click` + `type` in separate processes. |
| `devices` returns `{"devices": []}` or hangs 60s | usbmux down or iPhone unpaired. Run `driver.ps1 usbmux`, unlock the phone, tap **Trust This Computer**, re-run. |
| `link.exe` / `kernel32.lib` errors | Windows SDK `Lib`/`Include` missing — install `Microsoft.WindowsSDK.10.0.26100`. |
| cargo cannot write `riviu-managers-phone.exe` | The app is running and the exe is locked. `driver.ps1 stop` first. |
| `tauri dev` exits immediately | `driver.ps1 log 60` — the launcher's exit code and cargo errors are in `target/run-skill/tauri-dev.log`. |
| Sidecars still alive after `stop` | `stop` reports them by pid with argv. Verify the fingerprint, then `Stop-Process -Id <pid>`. |
