# TikTok Interaction Gate 0

This directory receives the reviewed Gate 0 transport, URL, clipboard, geometry,
and lifecycle evidence. The probe is `tools/interaction-gate0/probe.py`.

Gate 0 qualifies one exact device/app/Agent/adapter tuple. It does not attest the
later Rust action executor, enable Interaction commands, or edit
`sidecars/wda/interaction-capabilities.json`. A fixture result is always
`FIXTURE_ONLY` and is never a production qualification.

## Fixture verification

Run from the repository root:

```powershell
python -m unittest discover -s tools/interaction-gate0 -p test_probe.py -v
```

The local tests use protected fake control and MJPEG servers plus generated JPEG
frames. They cover:

- missing, wrong, and correct authentication (`401/401/200`) on every exercised
  control route and on MJPEG;
- exact direct-video, short-link, and photo `POST /url` request bodies;
- protected session creation before a generation-owned, continuous MJPEG reader;
- action-completion sequence boundaries, so queued pre-action frames cannot become evidence;
- the 65,536-byte decoded clipboard ceiling and both clipboard access modes;
- stable foreground bundle/PID proof and strict 375x667 portrait geometry derived
  from MobileGestalt plus sessionless `/wda/deviceOrientation`;
- frame-derived Share detection and pinned macOS Vision Copy Link OCR;
- reader/server cleanup with local and device-port closure;
- token, UDID, and raw target-URL redaction in raw bytes and decoded JSON leaves;
- IPA `Info.plist` executable identity cross-checking;
- uninstall/fresh-install identity binding for the exact hashed IPA;
- HTTPS TikTok-only short-link redirects and exact Mac dependency pins; and
- crash-recoverable JSON/Markdown publication with byte-verified rollback.

## Live invocation

Run only on the Mac attached to the unlocked test iPhone, with desktop, harness,
3uTools, and other XCTest owners stopped:

Before starting, copy the exact text `RIVIU_GATE0_CLIPBOARD_FIXTURE_V1` on the
fixture iPhone. The probe accepts only that controlled plaintext clipboard state,
restores the same bytes after every case, and rejects a personal clipboard value.
It also terminates and uninstalls the existing Agent, then fresh-installs the exact
hashed `RiviuAgent.ipa`; use only the designated Gate 0 device.

```bash
python3 -m pip install -r sidecars/wda/riviu-agent/requirements-mac.txt

RIVIU_RTMMO_TOKEN="$(security find-generic-password \
  -s riviu-managers-phone -a agent-auth-token -w)" \
python3 tools/interaction-gate0/probe.py \
  --udid "$RIVIU_GATE0_UDID" \
  --ipa sidecars/wda/RiviuAgent.ipa \
  --agent-manifest sidecars/wda/agent-manifest.json \
  --token-env RIVIU_RTMMO_TOKEN \
  --tiktok-bundle com.ss.iphone.ugc.Ame \
  --direct-url "$RIVIU_GATE0_DIRECT_URL" \
  --photo-url "$RIVIU_GATE0_PHOTO_URL" \
  --short-url "$RIVIU_GATE0_SHORT_URL" \
  --report-dir docs/re/interaction-gate0
```

There are no CLI controls for lowering samples, weakening identity, supplying
device/app versions, overriding geometry, or choosing a clipboard result. The
tool hashes the IPA and manifest first, then derives installed Agent, TikTok,
transport, iOS, and device identity through the Device Bridge. Windows publishes
a typed `PENDING_MAC_DEVICE` result without starting live services.

The Mac path enforces `pymobiledevice3==10.1.0` and `Pillow==11.3.0`, stops the
exact prior Agent PID, proves both ports closed, fresh-installs the hashed IPA,
then cold-starts it with its token in the process environment. It opens control
and MJPEG through separate usbmux relays and fixes the
RT-MMO route matrix to protected `/session`, `/url`, `/wda/setPasteboard`,
`/wda/getPasteboard`, `/wda/activeAppInfo`, `/wda/deviceOrientation`, and
sessionless `/wda/swipe`. It never calls TikTok hierarchy, element, session-window,
or WDA screenshot routes. Each target gets a fresh session before MJPEG. The
reader keeps one authenticated, unbounded connection for the active generation,
requires multiple decoded frames, rejects EOF, finite `Content-Length`, frozen
content, geometry drift, and gaps over two seconds. Every action samples the reader
sequence only after the correct authenticated request completes, and evidence uses
frames strictly newer than that boundary.

Short-link resolution validates the initial URL and every redirect as HTTPS on an
exact TikTok host, with a stateful five-hop limit. Share is located from the
right-rail white-glyph chain. The newer Share-sheet frame
is processed by `vision_ocr.swift`, pinned to Vision request revision 3, accurate
recognition, English/Vietnamese languages, language correction, and confidence
0.55. Exactly one normalized Copy Link label is required. Both Share and Copy Link
are one-pixel sessionless native swipes; missing/wrong auth requests are proven not
to act before the one correct request. The probe resolves and compares copied post
kind/content ID, restores the controlled plaintext clipboard on success and failure,
and dismisses an open Share sheet with feed-rail frame confirmation. It rechecks the
same session/PID and continuous stream using frames captured strictly after the
health checks. An ambiguous Share request is treated as possibly executed; cleanup
samples a newer frame before deciding whether the sheet needs dismissal. It then
terminates the exact Agent and proves both device ports closed. Any uncertainty
leaves the gate `FAIL` and stops the matrix.

## Evidence policy

Publication writes `gate-0.json` and `gate-0.md` into a transaction directory,
checks both for secret representations, journals old/new SHA-256 values, and
replaces both destinations. It verifies both destination bytes before sealing the
commit. The next invocation recovers a process death between replacements before
performing device work. Published evidence contains only exact tuple values,
hashes, timings, stable request labels/statuses, outcomes, and selected frame
hashes. It must not contain the token, raw UDID, prior clipboard bytes, or raw
target URLs.

Adding a production qualification remains a separate operator-reviewed step. It
must bind the final report SHA-256 and preserve the production IPA and manifest.
The current Windows checkpoint is `PENDING_MAC_DEVICE`; the production registry is
still empty, so no Interaction command is enabled by this work.
