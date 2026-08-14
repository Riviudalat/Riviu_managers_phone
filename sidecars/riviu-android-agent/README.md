# Riviu Android helper

A small APK the desktop can install on a farm phone. It is **not** an iPhone
Agent equivalent and it is **not** a replacement for
`io.appium.uiautomator2.server`.

| This APK does | This APK does not |
|---|---|
| Read/write clipboard while it is briefly the current IME | Stay the default keyboard |
| Insert/delete one MediaStore row by `_id` | Drive taps, the tree, or typing |
| Answer `GET /status` on loopback `:17980` | Bind `0.0.0.0` or replace scrcpy/minicap |

Typing stays `ACTION_SET_TEXT`. View stays scrcpy 3.3.4. Evidence stays minicap.
See `AGENTS.md` §9.51 / §9.52.

## Why there is no APK in `sidecars/android/noarch/`

The manifest in `sidecars/android/` pins **bytes + SHA-256**. Inventing those
numbers for a file that was never assembled — or pinning a debug APK from a
local assemble — is a lie the CI gate exists to catch.

Build on a machine with JDK 17 and Android SDK 34, then pin the output.

## Build

```powershell
# Requires JAVA_HOME (JDK 17+) and ANDROID_HOME or ANDROID_SDK_ROOT (platforms;android-34 + build-tools).
# Gradle 8.7+ on PATH, or open this folder in Android Studio and Build > Build Bundle(s) / APK(s).
.\build.ps1
```

The script refuses when the SDK or JDK is missing. It does not download either.
Output: `app/build/outputs/apk/debug/app-debug.apk`.

To ship it:

1. Copy the APK to `sidecars/android/noarch/riviu-agent.apk`.
2. Regenerate `sidecars/android/android-tools-manifest.json` **deliberately**
   (bytes + sha256, `role: riviuAgentApk`). Do not regenerate on every build.
3. Record the same digest in root `NOTICE`.
4. `python scripts/collect_desktop_ci_artifacts.py verify-android-tools`.

Override at runtime without bundling: `RIVIU_ANDROID_AGENT_APK=<path>`.
Precedence is `config → env → bundled`, same trap as minicap (§9.27).

## Install on a phone

```powershell
adb -s <serial> install -r -g app\build\outputs\apk\debug\app-debug.apk
adb -s <serial> shell ime enable com.riviu.agent/.RiviuIme
adb -s <serial> shell am start-foreground-service -n com.riviu.agent/.AgentService
```

Do **not** `ime set` this IME and leave it. The driver enables it for one
clipboard call and restores `settings get secure default_input_method`.

`AgentService` is `exported=true` so `adb shell am start-foreground-service`
can start it. The HTTP server still binds `127.0.0.1` only. `exported=false`
fails with `Requires permission not exported from uid …`.

Clipboard runs on the main looper. Calling `ClipboardManager` from the HTTP
accept thread throws `Can't create handler inside thread that has not called
Looper.prepare()`.

MIUI/HyperOS may still return `INSTALL_FAILED_USER_RESTRICTED` until
Developer options → *Install via USB* is on. That is policy, not an APK bug.
Do not retry `adb install` / `pm install` / install-session.

## Protocol

Loopback only. Host reaches it with `adb forward tcp:0 tcp:17980`.

| Method | Path | Body | Success |
|---|---|---|---|
| GET | `/status` | — | `ok`, `agentVersion=0.1.0`, `protocolVersion=1`, `features` |
| POST | `/v1/clipboard/set` | `{"text":"…"}` | `{"ok":true}` |
| POST | `/v1/clipboard/get` | `{}` | `{"ok":true,"text":"…"}` |
| POST | `/v1/media/import` | `{"relativePath":"01.png","displayName":"01.png"}` | `id`, `pendingModel` |
| POST | `/v1/media/delete` | `{"id":"123"}` | `{"ok":true,"id":"123"}` |

Staged images live in the app's external files dir, `inbox/<relativePath>`.
Push there with `adb push` (from PowerShell or Rust — Git Bash mangles
`/sdcard`, §9.12). `relativePath` is one segment: letters, digits, `.`, `_`, `-`.
Delete is by MediaStore `_id` only.
