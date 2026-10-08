# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

@AGENTS.md

Keep the import above. Once a CLAUDE.md exists, Claude Code no longer reads AGENTS.md on its own. AGENTS.md is the contract shared by every agent, and this file covers only what it leaves out.

## What this repo is

Riviu Manager is a Tauri 2 desktop app built from a Rust workspace, React 19/TypeScript/Vite and SQLite. It drives fleets of Android and iOS phones through TikTok nurture (Nuôi), interaction (Tương tác) and publishing (Đăng bài), plus device Flows/Macros, orchestration (Điều phối), schedules and an evidence ledger. Windows x64 is the primary target. macOS builds exist, but Linux hosts are unsupported. Phone effects are not idempotent, so a sent command never proves that it worked. Code comments are mostly in English. Product docs and all text an operator sees, including logs, are in Vietnamese.

## Commands

These commands add to the base gates listed in AGENTS.md. Package names differ from directory names: `apps/desktop/src-tauri` is `riviu-managers-phone`, and `crates/core` is `riviu-core`. The other crates are `riviu-android-driver`, `riviu-ios-driver`, `riviu-script-engine`, `riviu-signing` and `riviu-deployment-checker`.

```powershell
# Rust, from the repo root. Run tests single-threaded, as CI does.
cargo test --locked -p riviu-managers-phone -- --test-threads=1
cargo test --locked -p riviu-core <name_filter> -- --test-threads=1                 # one test
cargo test --locked -p riviu-core --test flow_release_one -- --test-threads=1       # one file in crates/core/tests/
cargo clippy --locked -p riviu-managers-phone --all-targets -- -D warnings
# CI's full Rust gate (slow):
cargo clippy --workspace --all-targets --features riviu-managers-phone/diagnostics,riviu-managers-phone/deployment-check --locked -- -D warnings
cargo test --workspace --features riviu-managers-phone/diagnostics,riviu-managers-phone/deployment-check --locked -- --test-threads=1

# Frontend, from apps/desktop
npx vitest run src/deviceNaming.test.ts -t "<test name>"   # one Vitest file or case
npx playwright test e2e/pages.spec.ts -g "<title>"          # one browser scenario (mocked Tauri bridge)

# IPC codegen, docs and tooling, from the repo root. Python is 3.12; bare `python` may be another version.
cargo run --locked -p riviu-core --example export_ipc -- apps/desktop/src/generated-ipc.ts
py -3.12 scripts/check_generated_ipc.py
py -3.12 scripts/check_docs.py --include-untracked   # Markdown links/anchors; covers new files before staging
py -3.12 scripts/collect_desktop_ci_artifacts.py verify-version
uv run --project sidecars/gui-service pytest sidecars/gui-service/tests
node --test scripts/test_publish_sheet.mjs scripts/publish_acceptance.test.mjs
```

- The `quality` job in `.github/workflows/desktop-ci-cd.yml` lists every gate. CI runs only when someone dispatches it manually, so run the relevant gates locally.
- The workspace manifest denies all clippy lints, so clippy fails locally the same way it fails in CI. Plain rustc warnings fail only in CI, through `-D warnings`.
- `rust-toolchain.toml` (1.95.0) must match `RUST_TOOLCHAIN` in the workflow. A test in `apps/desktop/src-tauri/src/lib.rs` asserts this.
- All worktrees share one Cargo `target/` in the main checkout. Run Cargo sequentially, and never `cargo clean` or override `--target-dir`. See `scripts/dev_compile_cache.ps1`, which wraps sccache with a lock. `.cargo/config.toml` turns off incremental compilation on purpose (it once took ~200 GB), so don't turn it back on.
- A version bump changes five files: `apps/desktop/package.json`, `apps/desktop/src-tauri/{tauri.conf.json,tauri.full.conf.json,Cargo.toml}` and `crates/deployment-checker/Cargo.toml`. It also updates both lockfiles. `verify-version` checks that the five versions match.

### Running the app

- `npm run dev` (in `apps/desktop`) starts Vite only, with no Tauri backend. Playwright drives it through `e2e/fixtures/tauriMock.ts`.
- To check UI in the real Tauri shell, use isolated UI smoke. It is debug-only, with a scratch DB/log/WebView and credentials kept in RAM. It has no USB, sidecars or workers, and it allows only an IPC allowlist:
  `powershell -NoProfile -File .claude/skills/run-riviu-managers-phone/driver.ps1 launch --smoke` (also `status` and `stop`).
- `npm run tauri:dev` runs the real controller. It uses the operational DB and credentials and live USB phones, and saved schedules can fire. Use it only with explicit permission, and only when no other controller holds the phones. Setting `RIVIU_MOCK_DEVICES=1` by itself does not isolate data. For full agent mode, run `$env:RIVIU_AGENT_MODE='full'; npm run tauri:dev -- --config src-tauri/tauri.full.conf.json`.
- Binaries in `apps/desktop/src-tauri/src/bin/` need `--features diagnostics`. Building them is safe, but running them can Like/Comment/Follow/Post for real.

## Architecture

```text
React UI (api.ts) / Local API / Riviu MCP
  -> Tauri commands -> admission (ensure_accepting_work) + DeviceControlPlane leases
  -> engines in riviu-core: nurture / interaction / publish / flow / orchestration
  -> riviu-android-driver | riviu-ios-driver
  -> observation + verifier -> SQLite intent/receipt/outbox -> read models -> UI
```

- `apps/desktop/src-tauri` is the composition root:
  - `lib.rs::run()` picks a `StartupPolicy` in `ui_smoke.rs`: production, UI smoke or no-public rehearsal.
  - `AppState::bootstrap` in `state.rs` then sets up the runtime: data dir and `riviu.db`, OS credential store, sidecar root, iOS runtime (`agent_runtime.rs`), and the Android backend, which joins only if adb is usable. It then spawns the background workers.
  - Commands live in `*_commands.rs` and `commands/`.
- `crates/core` holds the contracts and engines:
  - `device_control/` (`DeviceControlPlane`: leases, sessions, streams, roster) and `device_work.rs`
  - `driver.rs` (the `DeviceDriver` and `UiSession` traits)
  - `db/` (rusqlite, `MIGRATIONS` in `db/migrations.rs`, and a `StorageExecutor` with bounded reader and writer lanes)
  - the feature engines, perception (`ui_automation/`, `app_automation/`) and the `tiktok_*` action modules
- The other crates:
  - `crates/android-driver`: ADB, UiAutomator2 and the Riviu Helper APK, scrcpy view and minicap evidence. The adb resolver is `src/adb.rs`.
  - `crates/ios-driver`: pymobiledevice3 sidecar, WDA/Riviu Agent, usbmux and MJPEG, plus the Windows Job Object process-tree guard.
  - `crates/script-engine`: Script/Flow validation and compilation.
  - `crates/deployment-checker`: the installer checker, which reuses the desktop package checks without Tauri.
- `sidecars/`:
  - `gui-service`: Python 3.12 FastAPI with uv. It only returns OCR and template-match candidates and never touches ADB or the app DB.
  - `pymobiledevice3`: the iOS runtime.
  - `wda`: the iPhone agent IPA and the manifests that own its artifact identity.
  - `riviu-android-agent`: the Helper APK (Gradle).
  - `android`: bundled adb and tools, pinned to exact bytes.
  - `yt-dlp`: called only by `crates/core/src/tiktok_web.rs`.

### Command invariants (source-scanning tests in `apps/desktop/src-tauri/src/lib.rs`)

Each `#[tauri::command]` must:

- be registered in `generate_handler!`. A new command file also goes into `COMMAND_SOURCES`.
- return `Result<_, CommandError>` (from `command_error.rs`), or be listed in `INFALLIBLE_COMMANDS` with a reason.
- bind `let _admission = state.ensure_accepting_work()?;` in its own body, or be listed in `ADMISSION_EXEMPT` with a reason.
- be called by its literal `"command_name"` from one of the IPC modules the test scans: `api.ts`, `appWorkflow.ts`, `operatorRecords.ts` or `inspectorApi.ts` in `apps/desktop/src/`. Otherwise it must be listed in `UNREACHABLE_EXEMPT`.

Three more rules:

- Commands in `commands/android_ops.rs` must call `hold_this_phone(...)` or be listed in `LEASE_EXEMPT`.
- UI smoke rejects any command that is not on the reviewed `smoke_read_command` allowlist in `ui_smoke.rs`.
- Every HTTP client in the workspace must be built with a deadline.

### Cross-cutting rules

- **One IPC boundary.** `api.ts` and its three sibling wrapper modules own every `invoke` call. Add typed wrappers there, never in components.
  - Types come from the hand-written `src/types.ts` and from `src/generated-ipc.ts`. ts-rs generates the latter from the types listed in `crates/core/examples/export_ipc.rs`. Regenerate it rather than editing it by hand; `check_generated_ipc.py` fails on drift.
  - TanStack Query (`readQuery.ts`) caches persisted read models only. Device probes and effects stay uncached in `api.ts`, and mutations call `invalidateReadScope`.
- **One owner per phone.** The UI, Local API, Riviu MCP adapter and Inspector v2 all go through the same admission and control-plane leases. The Local API reuses the commands' `commands::with_manual_session` path and has no route of its own.
  - The Local API listens on loopback `127.0.0.1:22222`. It is off by default and requires a bearer token.
  - The Riviu MCP adapter is `scripts/riviu_agent_mcp.mjs`.
  - To get around "busy", never add a second driver, controller or scheduler, kill adb-server or force-drop leases.
- **Intent before effect.** SQLite records the intent and receipt (revision/CAS) before any device or network effect. Never hold a transaction across HTTP or device I/O. Secrets go in the OS credential store, not SQLite.
- **ACK is not proof.** Publish states are separate layers:
  - Submitted
  - Verified (canonical link plus proof of account, caption and time)
  - Sheet delivered/readback
  - Uncertain

  When a Post, Send or Sheet write has an uncertain outcome, observe it. Never retry it. Missing observation data means unknown, not absent, and a stale or ambiguous match never authorizes a tap.
- **Platform is not network.** `DevicePlatform` (Android/iOS) is separate from `SocialNetwork`. Only TikTok is implemented, and Instagram/Threads refuse before any effect. iOS Flow UI does not have Android parity yet.
- **A panic stops every phone.** Release builds use `panic = "abort"`, so a panic in any spawned task (scrcpy reader, job queue, Flow runtime) ends the work on all phones. Don't add `unwrap`/`expect` on runtime paths.
- **Measured constants and finite budgets.** A geometry constant carries a comment naming the device it was measured on. Recovery budgets are finite. Configured ceilings such as `--videos` or concurrency are caps, not targets.

## Docs and repo hygiene

- Owner docs:
  - `README.md`: product behavior
  - `docs/developer-guide.md`
  - `docs/development/contracts.md`: execution and data invariants
  - `docs/development/testing.md`: gates and evidence tiers
  - `docs/development/build-release.md`
  - `docs/agents/`: standing technical constraints. Start at `agent-runbook.md`.
  - `docs/ui-reference-matrix.md`: the UI contract

  `docs/archive/` is dated history, never current state. Update the doc that owns a behavior. Don't add changelogs or incident logs.
- `.gitattributes` keeps shipped binaries and `sidecars/android/**` byte-for-byte, with hashes in `sidecars/android/android-tools-manifest.json` and `NOTICE`. Don't renormalize line endings. The dev machine uses `core.autocrlf=true`.
- `.claude/skills/` is the canonical home of the project skills (the `riviu-*` skills, `run-riviu-managers-phone`, `rust-best-practices` and `systematic-debugging`). `.agents/skills/` is a git-ignored copy for other agent hosts, so edit `.claude/skills/` only.
