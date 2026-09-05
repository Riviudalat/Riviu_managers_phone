# Planning records — historical, not current status

Sixteen plans written 28/07/2026 – 31/07/2026. **Every file here is a record of what was
intended on its date**, kept because the reasoning is worth reading. None of them is a
status report, and none is updated when the code moves.

Treat a claim in this directory as *"true on the date in the filename"* and nothing more.

**Current status of anything these plans name: `AGENTS.md` section 9, newest entry first.**
That is the only place updated per batch.

## Why this file exists

Until 27/08/2026 fifteen of the sixteen carried **no status line at all**, and the
sixteenth (`2026-07-31-interaction-flow-delivery.md`) said `**Status:** Active delivery
plan` for four weeks after it had shipped and gone live on a 20-phone fleet. So the
directory offered a reader exactly two options: no signal, or a wrong one.

The fix is not sixteen status lines — a per-file status is another thing that goes stale
silently, and re-deriving the true delivery state of sixteen plans is precisely the kind
of claim that should not be made without measuring each one. The fix is stating the
invariant once, here, where it cannot rot: **these are dated records; AGENTS.md holds the
present tense.**

## Reading them

Grouped by what they were about:

- `2026-07-28-unified-agent-runtime.md`, `2026-07-29-riviu-agent-standalone-control-parity.md`
  — agent runtime shape
- `2026-07-29-rtmmo-agent-forensic-inventory.md` — reverse-engineering notes (iOS route);
  the survey material itself lives in `docs/re/`
- `2026-07-29-tiktok-interaction-*.md` (7 files) — the Interaction campaign, from Gate 0
  device control through fleet acceptance; `-roadmap.md` is the entry point
- `2026-07-30-riviu-flow-v2-*.md` (5 files) — Flow V2 foundation, runtime, desktop,
  acceptance; `-roadmap.md` is the entry point
- `2026-07-31-interaction-flow-delivery.md` — the combined delivery plan for the two above

Specs these plans were written against: `docs/archive/specs/`.
