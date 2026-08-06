# Nurture human-v2 verification

Date: 2026-08-06
Device: iPhone 8, iOS 16.7.15, UDID `a99f4bd9f877b2a0e3682ee24fd1c68f75ba6982`

## Changed artifact

- Build: `target/debug/bundle/macos/Riviumanagersphone Full.app`
- Installed: `/Applications/Riviumanagersphone Full.app`
- Executable SHA-256: `e4da1fb730ad7fcb4cf82b750c85ed05f5b3bcf743f6ab4a427c4d81ec9e53e2`
- Embedded `live_nurture_test` SHA-256: `681ffe53517fb1244791778c177091ff8baf0d33389c9167bec309e29f6246df`
- `codesign --verify --deep --strict`: PASS
- Patch: `nurture-human-v2.patch`
- Rollback script: `rollback.sh`
- Settings rollback script: `rollback-db.sh`
- Baseline rollback copy: `/Applications/Riviumanagersphone Full.app.rollback-20260806-human-v2`
- Baseline executable SHA-256: `335c35fcb79af920e0714b2f96d20ffeb250100ef361628f8ff798252d1ef68a`

## Automated verification

Commands and results:

```text
cargo fmt --all
cargo test -q -p riviu-core --lib                         PASS: 299 passed, 1 ignored
cargo test -q -p riviu-core --test real_frames              PASS: 15 passed
cargo test -q -p riviu-managers-phone                       PASS: 49 passed
npm --prefix apps/desktop run build                         PASS
npm --prefix apps/desktop test -- --run                   PASS: 15 files, 73 tests
codesign --verify --deep --strict <Full.app>                PASS
```

The frame fixture suite covers `LivePreview` classification and fresh action-rail
location. The UI source no longer contains `Nhịp an toàn`, `RiskGuard`, or
`risk_*` settings. DeepSeek text-only uses the shared OCR-caption path; the
desktop macOS adapter is wired through `FrameTextSource`.

The follow-up default audit also covers the fixed 7..13-video rest threshold,
bounded legacy video/round/schedule values, and a first-run default with comment
probability 0 so a missing AI key does not block browsing. Fresh defaults are
like 35%, follow 3%, frenzy 6%, watch 3..18s, and (when explicitly enabled)
240-minute scheduling with 150-minute blocks.

The existing stored profile was migrated once from the legacy values to
`numVideos=120`, `likeProb=35`, `commentProb=0`, `followProb=3`, `frenzyProb=6`,
`watchMin=3`, `watchMax=18`, and schedule `240/150`; obsolete `riskGuard*` fields
were removed and marker `nurture.settings.migration.v2=2026-08-06-human-v2` was
written. The source DB was backed up at
`/Users/admin/Library/Application Support/riviu-managers-phone/riviu.db.rollback-human-v2-20260806`
(SHA-256 `a14beca737a8924da399dbcb748dd07a0618e3550ace48540bc93458e0fd7870`).
`rollback-db.sh` was run on a copy and restored the old `50/40/25/5` values with
the legacy risk fields intact.

The touch/speed review adds a per-UDID/session planner on the integer logical
screen grid. It keeps every generated nurture tap coordinate unique, avoids
nearby repeats in the recent trail, and preserves a safe hitbox for each rail or
composer control. Feed swipes now use quick `190..280ms`, normal `300..520ms`,
slow `520..820ms`, and explicit frenzy `150..240ms` bands; photo swipes use
`280..420ms` or `420..760ms`.

The planner unit fixture generated 400 points in one rail hitbox and verified
unique integer coordinates; the bounds fixture also kept every point inside
`0.5..374.5 x 0.5..666.5`. Command result: `cargo test -q -p riviu-core --lib`
=> `299 passed; 0 failed; 1 ignored` (exit 0). OCR caption extraction now drops
photo metadata such as `lượt thích`/`• Ảnh`, retains lower visible caption lines,
and retries a text-only draft when the verifier flags formal style.

## Installed app smoke

The Full app was launched after the final install and after rollback restoration.
The authenticated candidate health response was:

```text
HTTP 200
health_state ready
protocol 2
features stream/tap/swipe/clipboard/text/pushMedia
```

Authenticated `/screenshot` returned a 1,126,612-byte PNG (`750 x 1334`). On the
device MJPEG socket, unauthenticated `GET /` returned HTTP 401 and the same
request with `X-Riviu-Token` returned HTTP 200. The app's control relay, stream
sidecar, and process-tree cleanup were observed under the exact app PID.

The final local control proxy returned protected `/wda/locked` HTTP 200 and
unauthenticated protected routes returned HTTP 401. The migrated live DB returned
`PRAGMA integrity_check = ok` with marker `2026-08-06-human-v2`.

## Live nurture smoke

Pass command:

```text
RIVIU_AGENT_TOKEN=<ephemeral> \
RIVIU_AGENT_MANIFEST=$PWD/sidecars/wda/candidate-manifest.json \
RIVIU_DEFAULT_AGENT_MODE=full \
./target/debug/live_nurture_test --udid a99f4bd9f877b2a0e3682ee24fd1c68f75ba6982 \
  --minutes 1 --videos 12 --like-prob 35 --comment-prob 0 --follow-prob 3 \
  --watch-min 2 --watch-max 5 \
  --jsonl docs/verification/nurture-human-v2-20260806/live-smoke-human-v2.jsonl
```

Observed result: exit 0; session create/prime pass; stream first frame pass;
4 videos; 1 like confirmed by the red icon; 0 popup closed; 0 heavy recoveries;
and the run ended in TikTok after 66s. WDA had 4 native swipes (p50 711ms).

## Live comment and URL gate

The harness now accepts `--open-url` and uses the candidate-only standard WDA
`/url` route when the live interaction capability report is empty. It then closes
that context before Nurture creates its fresh text session. The target photo URL
was opened and the caption-only DeepSeek path was exercised with
`--comment-prob 100 --steady chatty`:

```text
docs/verification/nurture-human-v2-20260806/live-comment-target-open-url-v6.jsonl
exit 0
3 videos, 2 comments, 2 send-button-off frame confirmations
interaction.openUrl.standard n=1, keys n=2, request errors=0
popup closed=2, heavy recoveries=0, final state=TikTok
```

The first ad/LIVE attempts were skipped when evidence was insufficient; the
targeted photo run passed only after OCR retained the actual caption lines. The
comment text remains fail-closed on contradiction, unsupported claims, UI text,
or overly formal output.

The follow-up action-biased run is retained as
`live-smoke-actions.jsonl`. It reached the trusted session and stream but the
device was on a non-FYP screen, so it ended with 0 videos and exit 2. This is a
real setup/content result, not a rewritten pass.

## Rollback verification

`rollback.sh` was executed against the installed app while no app process was
running. It produced the baseline hash above and `codesign --verify` PASS. The
new Full app was then recopied, producing executable hash
`e4da1fb730ad7fcb4cf82b750c85ed05f5b3bcf743f6ab4a427c4d81ec9e53e2` and another
codesign PASS. The modified app was launched once more and returned authenticated
health 200 with protocol 2, features `stream/tap/swipe/clipboard/text/pushMedia`,
and the expected active agent session.
