# Riviu Flow V2 Release 1 Checkpoint

Date: 31/07/2026

Release 1 is complete through the Windows desktop, deterministic fixture,
browser, and rollback gates. The Mac/iPhone live portion of F3 remains
`PENDING_MAC_DEVICE`; this document does not promote a mock profile to a live
qualified profile.

## Gates

| Gate | Status | Evidence |
|---|---|---|
| F0 foundation | PASS | Checkpoint and rollback commit are recorded in `AGENTS.md`. |
| F1 runtime | PASS | Recovery, exact termination, evidence, cancellation, and shutdown checkpoints are recorded in `AGENTS.md`. |
| F2 desktop | PASS | Rust workspace, Clippy, Vitest, lint, frontend build, and diff checks pass on Windows. |
| F3 Rust fixture | PASS (`FIXTURE_ONLY`) | Two devices, 16 attempts, two verified JPEG artifacts, zero uncertain attempts, and clean control/stream shutdown. |
| F3 browser | PASS (`FIXTURE_ONLY`) | Six Playwright workflows pass at 1440x900 and 900x700. |
| Rollback | PASS | Release migration plus the detached pre-F0 core, frontend, desktop build, and desktop boot proof pass against one copied database. |
| F3 live | `PENDING_MAC_DEVICE` | Requires the Phase 4A protected-auth/geometry tuple and Mac/iPhone execution. |

## Windows Verification

- `cargo test --workspace`: PASS; core `263` passed and `1` explicit fixture
  ignored, iOS driver `131` passed, desktop `40` passed, and all integration/doc
  test targets passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS.
- `cargo fmt --all -- --check`: PASS.
- Vitest: `71/71` passed across `14` files.
- Oxlint: exit `0`; seven Fast Refresh warnings remain non-blocking.
- Frontend production build: PASS (`1979` modules transformed).
- Playwright: `6/6` passed, including 1440x900 and 900x700 screenshots.
- Python app-control contract tests: `10/10` passed.
- `git diff --check`: PASS.

## Fixture Evidence

- Flow: `docs/fixtures/flow-release-one.json`
- Plan SHA-256: `88333ddcbb7ae804825e1902ad5c0a3d04431def5a947f65aabf8dae724173c4`
- Devices: `MOCK-IPHONE-01`, `MOCK-IPHONE-02`
- Aggregate: `succeeded`
- Attempts: `16`; uncertain attempts: `0`
- Artifacts: `2` JPEG files with identical validated SHA-256
  `ec2911edb8c793f94c69e470f12d14cc25d9907554000482e2852c7a03149771`
- Legacy driver screenshots: `0`; UI screenshots: `0`
- Stream budget: configured `2`, maximum reserved `2`, reserved/running after cleanup `0/0`
- Context cleanup: active `0`, quarantined `0`

The headless harness real-device mode remains typed-disabled as
`LiveFlowDriverPendingConfiguration`; an HTTP status response or a fixture run is
not live acceptance evidence.

## Enabled Nodes

Start, End, Launch App, Terminate App, Wait, Tap, Swipe, Type Text, Screenshot,
Home, and Assert Visible are in the Release 1 catalog. A node runs only when its
definition has no disabled reason and runtime preflight qualifies every required
device capability.

Terminate App uses the F1 exact-PID DVT contract. Success requires persisted
`ProcessAbsent { bundleId }` evidence with `ok=true`, `running=false`, the exact
requested bundle, and a matched pre-effect PID. A transport acknowledgement alone
is not success.

Real coordinate picking and Pmd UI execution remain fail-closed until Phase 4A
returns target-bound protected auth, exact qualified geometry, profile identity,
and readiness before and after stream parking.

## Rollback

- Pre-F0 commit: `805056790d890046384ad7a578cc34a99088e799`
- Fixture: `docs/fixtures/rollback-legacy-probe.rs`
- Detached-worktree data-dir seam:
  `docs/fixtures/rollback-pre-f0-mock-data-dir.patch`
- Seam patch SHA-256:
  `ad5d540a6f1f404ed2e061ab50b37eb361accec73c23c39fdfd3850492054fcb`
- Final copied database SHA-256:
  `5120458fe333e3a5350f60688cdad9e08f177801e15709dba2d48de94a7f9c9c`
- Final copied database size: `245760` bytes
- SQLite ledger: exactly versions `1` and `2`; `integrity_check=ok`
- Pre-F0 legacy repository/parser probe: PASS
- Pre-F0 frontend build: PASS
- Pre-F0 desktop build and five-second boot against the copied DB: PASS, exit `0`

`RIVIU_MOCK_DATA_DIR` is accepted only with `RIVIU_MOCK_DEVICES=1` and an
absolute path. The checked-in patch adds that same test-only seam to the detached
pre-F0 source; it does not alter the rollback commit or production binary. The
operator's real data directory is never redirected, copied over, or opened by the
proof.

## Production Agent

- `RiviuAgent.ipa` SHA-256:
  `8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea`
- Canonical-LF `agent-manifest.json` SHA-256:
  `e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a`

Both production artifacts are unchanged.

## Deferred Scope

The live F3 tuple, TikTok action nodes, conditions, loops, cross-device bindings,
account switching, A-comment/B-reply, MDM/supervision, backup/restore, system
proxy application, push media, and syslog remain deferred. Interaction Campaign
G0.12 and its production engine/UI remain separate and blocked on their Mac live
gate.
