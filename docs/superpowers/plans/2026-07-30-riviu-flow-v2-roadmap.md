# Riviu Flow V2 Roadmap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the approved Riviu Flow V2 release-1 visual workflow without weakening device ownership, session-before-MJPEG, evidence, persistence, or rollback guarantees.

**Architecture:** Execute four gated plans in dependency order. Rust owns the graph model, compiler, immutable revisions, run state, evidence, and device lifecycle; Tauri exposes typed commands; React Flow owns only draft layout and operator interaction.

**Tech Stack:** Rust 2021, Tokio, rusqlite, serde, SHA-256, Tauri 2, React 19, TypeScript 6, `@xyflow/react`, Vitest, Testing Library, Playwright, Python 3.9+, pymobiledevice3 10.1.0.

---

## Approved Source

- Design: `docs/superpowers/specs/2026-07-30-riviu-flow-v2-design.md`
- Approved design commit: `a8c3497`
- Runtime constraints and handoff: `AGENTS.md`

## Execution Order

| Order | Plan | Gate |
|---|---|---|
| 1 | `2026-07-30-riviu-flow-v2-foundation.md` | F0: typed model, catalog, compiler, legacy diagnostics, versioned migrations, immutable revisions |
| 2 | `2026-07-30-riviu-flow-v2-runtime.md` | F1: durable attempts, evidence, artifacts, device runtime, cancellation, recovery, shutdown |
| 3 | `2026-07-30-riviu-flow-v2-desktop.md` | F2: typed Tauri API and usable React Flow editor/monitor |
| 4 | `2026-07-30-riviu-flow-v2-acceptance.md` | F3: mock regression, Mac/device qualification, package and rollback proof |

Do not begin a plan until the preceding gate is committed and its named verification commands pass. F3 Mac/device evidence cannot be replaced by Windows fixture output.

## Pre-F0 Baseline

Immediately before the first F0 source edit, require a clean plan commit and record
that exact commit as the rollback baseline:

```powershell
if (git status --porcelain) {
  throw "Commit or isolate the plan before starting F0"
}
$env:RIVIU_PRE_F0_COMMIT = (git rev-parse HEAD).Trim()
git show --no-patch --format="%H %s" $env:RIVIU_PRE_F0_COMMIT
```

Write the printed full hash into the Flow V2 checkpoint in AGENTS.md. Every F0-F3
handoff repeats that same hash; it never advances to a later implementation commit.

## Design Coverage

| Design section | Implemented by |
|---|---|
| 1-4 Context, decisions, scope, non-goals | Roadmap invariants and all gate boundaries |
| 5 Authoring model | F0 Tasks 1-2; F2 Tasks 3-5 |
| 6 Action registry | F0 Task 1; F2 Tasks 1 and 4 |
| 7 Compiler/immutable plan | F0 Tasks 2-3 |
| 8 Persistence/artifacts | F0 Tasks 4-5; F1 Tasks 2-3 |
| 9 Execution/capability/shutdown | F1 Tasks 6-7; F2 Task 1 |
| 10 Side effects/retry/cancel | F1 Tasks 3, 5-7 |
| 11 TikTok/cross-device boundary | Roadmap exclusion until Interaction G0-G3 |
| 12 Desktop UX | F2 Tasks 2-7 |
| 13 Tauri API/events | F1 Task 7; F2 Tasks 1-2 |
| 14 Compatibility/rollback | F0 Task 3; F2 Task 7; F3 Task 4 |
| 15 Terminate/syslog correction | F1 Task 4; syslog remains outside release 1 |
| 16 Testing | Every task's red/green step; F3 Tasks 1-3 |
| 17 Acceptance | F3 Tasks 1-4 |

## Global Invariants

- Never execute the mutable canvas document; execute only its immutable compiled revision and hash.
- Never call a raw WDA route, arbitrary HTTP endpoint, or shell command from a Flow node.
- Never raise stock `snapshotMaxDepth` above 1.
- Never open MJPEG before the approved session exists.
- Never run a UI-session plan without Launch App as its first executable node, and
  never dispatch that first Launch more than once.
- Never use WDA `GET /screenshot`; Flow Screenshot uses the exact owned stream generation.
- Never let a verifier wait across a stream-generation advance; advance, deadline,
  and cancellation are explicit outcomes.
- Never mark a side effect `Succeeded` from a transport acknowledgement.
- Never retry an `Uncertain` Tap, Swipe, or Type Text attempt.
- Never hold device A while waiting to acquire device B.
- Never downgrade or reacquire a release-1 device context between dependent nodes.
- Never hold a SQLite transaction across `.await`, USB, stream, device, or network work.
- Never put a user-provided artifact label into a filesystem path.
- Never let a Flow worker outlive `DeviceControlPlane::shutdown_cleanup()`.
- Never advertise Terminate App until DVT termination and process-absence verification pass.
- Never reconcile Terminate by killing again; use the read-only process query under
  the same per-UDID ownership chain.
- Never add TikTok nodes until `InteractionCampaignEngine` exists and G0-G3 pass.
- Never modify the production IPA, Agent manifest, interaction capability registry, legacy scripts, or legacy job rows in F0-F2.

## Shared Checkout Rules

1. Read `AGENTS.md` and inspect `git status --short` before every task.
2. Stage only paths named by that task.
3. Use `apply_patch` for manual edits.
4. Keep production Agent artifacts byte-identical.
5. Update `AGENTS.md` in the same commit when a gate, invariant, architecture boundary, or do-not-repeat item changes.
6. Use TDD: run each named focused test red, implement the smallest slice, then run it green.

## Verification After Every Rust Task

```powershell
cargo fmt --all -- --check
cargo test -p riviu-core
cargo test -p riviu-script-engine
```

Run before each Rust gate commit:

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Verification After Every Desktop Task

```powershell
npm --prefix apps/desktop test
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
```

## Gate Handoff

At the end of each plan, append a dated checkpoint to `AGENTS.md` containing the gate, commit, commands and counts, remaining disabled nodes, next plan, and rollback commit. Keep each plan's checkboxes current as steps complete.
