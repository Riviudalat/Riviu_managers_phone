# TikTok Interaction Campaign Implementation Roadmap

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the approved TikTok Interaction campaign feature without breaking the live-confirmed iPhone/WDA lifecycle, shared USB capacity, existing Nurture behavior, or production Agent rollback artifacts.

**Architecture:** Implement six gated plans in dependency order. SQLite is the durable source of work; a shared per-UDID coordinator owns all screen-changing workflows; one stream budget counts every MJPEG producer; device capabilities fail closed against an exact live-qualified tuple; action side effects are persisted before execution and confirmed from frames.

**Tech Stack:** Rust 2021, Tokio, rusqlite, Tauri 2, React 19, TypeScript, Vitest, Python 3.9+, pymobiledevice3 10.1.0, Pillow 11.3.0, XCTest/WDA, SQLite.

---

## Approved Source

- Design: `docs/superpowers/specs/2026-07-29-tiktok-interaction-campaign-design.md`
- Approved design commit: `10433fb`
- Runtime constraints: `AGENTS.md`
- Production oracle/rollback artifacts that must remain byte-identical:
  - `sidecars/wda/RiviuAgent.ipa`
  - `sidecars/wda/agent-manifest.json`

Do not begin a later plan while an earlier plan's named gate is incomplete. A unit-test pass is not a substitute for a Mac/device live gate where the plan explicitly requires one.

## Execution Order

| Order | Plan | Gate produced | Blocks |
|---|---|---|---|
| 1 | `2026-07-29-tiktok-interaction-gate-0-device-control.md` | G0: shared ownership, bounded streams, typed capability/URL/clipboard contract | Every Interaction command |
| 2 | `2026-07-29-tiktok-interaction-campaign-core.md` | G1: durable domain, planner, dispatcher, scheduler, recovery | Campaign execution |
| 3 | `2026-07-29-tiktok-interaction-verified-actions.md` | G2: open/identity/watch/Like/Follow/Comment with frame evidence | First usable release |
| 4 | `2026-07-29-tiktok-interaction-desktop-workflow.md` | G3: Setup/Monitor/proxy UX and Tauri API | Operator release candidate |
| 5 | `2026-07-29-tiktok-interaction-new-action-gates.md` | G4: Save/Repost/Direct Message independently qualified | Expanded action set |
| 6 | `2026-07-29-tiktok-interaction-fleet-acceptance.md` | G5: Mac/device/fleet acceptance, packaging, rollback proof | Production enablement |

## Global Invariants

- Never enable Interaction from `AgentStatus.features` alone.
- Never map `TargetIdentityCopyLink` or Watch to production `Ready` from the G0 reference probe alone; the exact tuple also requires the G2 production-runtime live qualification.
- Never call generic `preflight_agent()` or `repair_agent_locked()` from the Interaction lifecycle.
- Never open MJPEG before the profile-approved session exists.
- Never run more active producers than the stream budget, including desktop tiles.
- Never let Nurture, Script, Repair, manual control, Group Sync, Open on Device, or Interaction own the same UDID concurrently.
- Never hold a SQLite transaction across `.await`, a device lease, stream transfer, USB operation, or network redirect.
- Never repeat Copy Link within an identity attempt after its `identity_copy_intent=issued`. An operator retry may append a new Pending/None identity attempt only after a prior Confirmed identity or a terminal pre-Copy attempt whose intent remained None; it never resets or reuses an old row.
- Retry may reopen only selected terminal `Partial|Failed|Interrupted` work through the reviewed append-only transaction. It must leave successful assignments/actors untouched, recompute affected actor/campaign projections, and never reopen `Succeeded`, `Uncertain`, `Cancelled`, or skipped work.
- Never automatically retry Comment, Repost, or Direct Message after `effect_intent=issued` when completion is ambiguous.
- Never resample probability, Watch duration, action pacing, or target pacing after the immutable plan commit. Direct Message chooses its recipient exactly once during the first durable preparation from the persisted assignment seed and normalized policy; after that payload is committed, retries, re-entry, and restarts never replace it.
- Never treat HTTP 200, a changed frame, or a visible TikTok rail as target identity proof.
- Never expose coordinate actions outside an exact live-qualified geometry/orientation profile.
- Never promote a G2/G4 capability before its full regression passes. Keep one original-registry rollback transaction across every promoted entry and post-promotion/package/staging check, then seal it only after the reviewed commit succeeds.
- Never log tokens, raw prior clipboard bytes, proxy passwords, or unredacted UDIDs in published evidence.
- Never modify or overwrite the production IPA/manifest during candidate or A/B work.

## Shared Checkout Rules

The repository can contain unrelated uncommitted Project 2/runtime work. For every task:

1. Inspect `git status --short` before editing.
2. Stage only files named by the current task.
3. Do not stage, revert, format, or rewrite unrelated modified files.
4. Keep `docs/claude/**` out of commits.
5. Re-read `AGENTS.md` before WDA, stream, supervisor, signing, or device lifecycle changes.
6. Update `AGENTS.md` in the same change when an architecture invariant, device constraint, gate result, or do-not-repeat item changes.

## Cross-Plan Verification

Run after every Rust slice:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Run after every desktop slice:

```powershell
npm --prefix apps/desktop test -- --run
npm --prefix apps/desktop run build
```

Run the Python suites touched by a slice with `python -m unittest ... -v`. Use the exact pinned Mac Python environment for live probes.

## Release Definition

The first usable release is G0 through G3 with only capabilities that have passed their own live gate. Save, Repost, and Direct Message may remain disabled without blocking that release. Production enablement additionally requires G5, including:

- direct video, photo, and short-link target identity;
- Unicode Comment sent evidence;
- Like and Follow desired-state evidence;
- budget-1 and budget-2 producer accounting;
- two-device All and RoundRobin campaigns;
- restart/crash recovery classification;
- artifact/token/clipboard/proxy redaction;
- unchanged production Agent rollback checksums;
- packaged desktop smoke test and a written rollback drill.

## Handoff Record

At the end of each plan, add a dated checkpoint to `AGENTS.md` containing:

- completed gate and commit;
- exact verification commands and pass counts;
- live qualification tuple, when applicable;
- remaining disabled capabilities and why;
- next plan filename;
- rollback command/artifact.
