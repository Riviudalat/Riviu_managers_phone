# Mẫu spec worker

Mỗi run có một file common (`target/<run>/COMMON.md`) và một spec ngắn cho từng worker.
Spec nằm dưới `target/` (git-ignored): nó trỏ tới đường dẫn máy, nghiên cứu và plan riêng.
Phần dưới là nội dung tái dùng; chép vào common của run rồi chỉnh chỗ `<...>`.

## Common của run

```markdown
# Common rules for run "<tên run>"

## Sources
- Plan: <đường dẫn plan>. Read your phase.
- Repo rules: CLAUDE.md and AGENTS.md:
  - operator text is Vietnamese;
  - no `unwrap`/`expect` on runtime paths;
  - measured constants name their device;
  - budgets are finite and failures close;
  - a stale or ambiguous match never authorizes a tap;
  - never retry an uncertain Post/Send;
  - record the intent before the effect.

## Tests and gates
- <"Write NO new tests" nếu owner đã quyết, hoặc "use test-audit before writing tests">.
- Offline checks: `rustfmt --edition 2021 <your files>` then revert any hunk you did not
  write (the tree is not rustfmt-clean; `cargo fmt --all` rewrites ~100 files and CRLF→LF).
  Edit Vietnamese text with Python or `-Encoding UTF8`, never PowerShell `Get-Content -Raw`
  round-trips. `cargo clippy --locked -p <touched crate>
  --all-targets -- -D warnings` (report pre-existing debt elsewhere, do not fix unrelated
  files); the EXISTING tests of the modules you touch, filtered by module path; for UI:
  `npx tsc -b` and `npm run lint` in `apps/desktop`.
- Update an existing test only when your behavior change legitimately breaks it. Never weaken
  an assertion.

## Devices and machine
- Do NOT touch adb or phones; do not start the Riviu controller, GenFarmer or Appium.
- Never kill or restart the adb server yourself; the controller's HX-ADB recovery owns that.
- Cargo: one command at a time; never `cargo clean`, never override `--target-dir`; no
  parallel cargo inside your worker (the host is low on memory).
- Python is `py -3.12`.

## Research inputs
Read-only and git-ignored: <thư mục nghiên cứu>. Never copy decompiled TikTok code into the
repo; only ids, short UI strings and class names.

## Git and secrets
- Never `git add -A` or `git add .`: stage explicit paths (the repo root may hold large
  untracked artifacts).
- Scoped commits (`fix(scope): ...`) ending with the run's Co-Authored-By trailer.
- Do NOT push or merge.
- Do not read settings files, `.mcp.json` or `~/.claude.json`.
- No secrets in code, fixtures, logs or reports.

## Final report
worker_done with: branch and commits; what changed; exact gate results; what needs a
real-device check (which screen, which phone action); anything unverified;
**"Bài học cho docs"**: 0–3 lines, each a standing rule or measured fact (where it was
measured) and the owner doc it belongs in (`docs/agents/…`, `docs/development/…`,
`docs/agents/tiktok-builds/…`), or "none".
```

## Phần thêm cho worker UI

```markdown
- Frontend only: no Rust, no Cargo, no `npm run tauri:dev`, no phones, no controller.
- Read the UI contract `docs/ui-reference-matrix.md` first; update it in the same commit when
  your phase changes a contract (record named exceptions there).
- IPC stays in `api.ts` and its sibling modules. Uncertain outcomes stay uncertain. Toasts
  follow "ACK is not proof" ("Đã gửi lệnh…", not "Thành công" for an unverified effect).
- Keep Vietnamese accessible names unless the phase deliberately changes a semantic contract;
  then update the e2e selectors.
- Dependencies: npm only, exact versions, released at least 2 weeks ago.
- Gates from `apps/desktop`: `npx tsc -b --pretty false`; the locked oxlint
  (`npx oxlint@<locked> --deny-warnings`, list the accepted pre-existing warnings);
  `npx vitest run` on touched tests, then `npm test`; `npx playwright test` on the specs
  covering your components, **on a free port** (playwright.config.ts reuses an existing
  server on its fixed port, so a parallel worktree's dev server would be tested instead).
- Visual baselines: re-bless only the diffs your change explains; list stale ones.
- Acceptance sizes: 820×560, 1366×768, 1920×1080, saved under `target/<run>/ui-<phase>-shots/`.
  Mock-browser evidence only; say so.
- Other UI workers touch `App.tsx`, `index.css`, `styles/*`, `tauriMock.ts`: keep hunks
  minimal and local, do not reformat code you do not own.
```

## Spec của một worker

```markdown
Read `<path>/COMMON.md` first. <Thêm: "and spec-ui-common.md" cho UI.>
Scope: <crate/thư mục/file được sửa>. <Không được sửa: ...>

# Task <id>: <một câu kết quả>

## Context
<vì sao, bằng chứng đo được: máy, thời điểm, log/screenshot path>

## Do
1. <bước có kết quả kiểm được>

## Gates
<lệnh cụ thể, focused trước>

## Report
worker_done with <những gì coordinator cần để merge và nghiệm thu>.
```

Một spec tốt: một kết quả, phạm vi file rõ, bằng chứng đầu vào có đường dẫn, cổng kiểm cụ
thể và chỉ rõ phần nào coordinator sẽ tự kiểm trên máy thật.
