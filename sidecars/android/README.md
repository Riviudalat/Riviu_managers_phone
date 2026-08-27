# Bundled Android tools

Binaries that ship inside the installer so a clean machine can drive Android
phones without an Android SDK. Nothing here is written by this project.

Attribution and licence exposure: [`../../NOTICE`](../../NOTICE). Read it before
touching `win-x86_64/` — the adb entry is recorded there as a **knowingly
accepted, unreviewed** risk, not as a cleared one.

## Layout, and why it is this shape

```
android-tools-manifest.json      bytes + SHA-256 for every file below
noarch/minicap.apk               JPEG evidence stream, pushed and run in place
noarch/scrcpy-server             H.264 view stream (scrcpy 3.3.4), not evidence
noarch/appium-uiautomator2-server.apk       the instrumentation control talks HTTP to
noarch/appium-uiautomator2-server-test.apk  its androidTest half; `am instrument` names this one
noarch/riviu-agent.apk           clipboard, media import, wallpaper, mock GPS, app labels
win-x86_64/adb.exe               platform-tools 37.0.1
win-x86_64/AdbWinApi.dll         must sit beside adb.exe
win-x86_64/AdbWinUsbApi.dll      must sit beside adb.exe
win-x86_64/NOTICE-platform-tools.txt   Google's own third-party notices
```

The two DLLs are **not** optional and **not** relocatable: Windows resolves a
binary's dependent DLLs relative to the executable's own directory, so an
`adb.exe` on its own fails at load time, not at build time. That failure cannot
be reproduced on a developer machine that has platform-tools on `PATH`, which is
why it is written here rather than left to be discovered.

`noarch` holds the architecture-independent Android payloads. minicap is never
installed — it is pushed and executed via `CLASSPATH=<apk> app_process`, so one
file covers every ABI. `scrcpy-server` is the official Genymobile 3.3.4 JAR,
pushed to `/data/local/tmp/riviu-scrcpy-server` and launched the same way.
It is the **view** path (H.264 tiles/overlay). minicap stays the JPEG
**evidence** path. The desktop scrcpy client (FFmpeg/SDL) is not bundled.

## The manifest pins both size and digest, on purpose

`ensure_apk` decides whether to re-push minicap by comparing the **byte count**
on the device. A corrupted APK of the same size would therefore be trusted
forever. So the manifest carries `bytes` *and* `sha256`, and the loader
(`apps/desktop/src-tauri/src/android_tools.rs`) checks both.

A mismatch does **not** panic and does **not** fail startup. The affected tool
resolves to `None` and a problem is recorded naming the file and both digests —
a corrupt bundled adb must not stop an operator whose own adb is fine.

## Changing these bytes

1. Replace the file.
2. Regenerate the manifest — do it deliberately, never as an automatic build
   step. A manifest regenerated on every build agrees with whatever is on disk
   and therefore pins nothing.
3. Update the digest and byte count in `../../NOTICE`, and the revision if it
   moved.
4. `python scripts/collect_desktop_ci_artifacts.py` verifies the tree against
   the manifest; CI runs the same check in the `quality` job so a bad digest
   fails in minutes instead of after three 120-minute builds.

`sidecars/android/**` is marked `-text` in `.gitattributes`. That is load-bearing
for `NOTICE-platform-tools.txt`, which is pure LF: with `core.autocrlf=true` a
checkout would rewrite it to CRLF and both pinned numbers would be wrong on any
machine other than the one that wrote them.

## These are not the only Android prerequisites

**This file is the single owner of "what is bundled".** The root `README.md` and
`AGENTS.md` used to restate it and two of the three copies went stale; they now
point here instead.

One thing still cannot come from here: the per-model USB driver and the on-device
*Allow USB debugging* prompt. That is it.

`io.appium.uiautomator2.server` + `.test` **are** bundled — `noarch/appium-uiautomator2-server.apk`
(17,948,327 B) and `noarch/appium-uiautomator2-server-test.apk` (197,183 B), pinned
in the manifest with `role: agentServerApk` / `agentTestApk`, and installed by
`install_agent_apks` (`pm install -r -g -t`). Both halves are required: the runner
lives in the `-test` one and `am instrument` names it, so a phone with only the
server refuses every tap exactly as a phone with neither does.

The Riviu helper APK (`com.riviu.agent`) **is** bundled too:
`noarch/riviu-agent.apk`, `role: riviuAgentApk`. Its source is under
`sidecars/riviu-android-agent/`, and the built artifact is gitignored — only the
pinned copy here ships. Clipboard on Android 10+ needs it; uiautomator2 must not
advertise an empty `get_clipboard`.

The manifest pins **nine** files. The Layout section above lists them; if that list
and the manifest ever disagree, the manifest is the one the loader reads.
