# Riviu Agent Evidence

This directory contains candidate-only Gate B/C evidence. Generated reports must
contain measured values, never credentials, device identifiers, user-home paths,
or inferred PASS states.

The Windows checkpoint validates deterministic five-patch, mode-aware full-tree
preparation, protocol contracts, packaging helpers, Objective-C unit-test
orchestration, source/xcconfig signed-plist attestation, Xcode 26 runtime-closure
finalization, clipboard runtime-schema enforcement, token scans, transactional
evidence publication, and authenticated local HTTP/MJPEG fixtures with real JPEGs.
Xcode build/signing and all device behavior remain `PENDING_MAC_DEVICE` until
`probe_gate_bc.py` runs against the signed candidate on a Mac/iPhone.

The live probe accepts only a candidate manifest produced by `build_candidate.py`.
It verifies the manifest/source/xcconfig/IPA digest chain, rehashes and token-scans
the separately locked xcconfig, verifies runtime token absence, performs an
exact-bundle fresh install, validates installed identity, and only then starts
five launch cycles. Each cycle proves the prior PID is gone, both ports are closed,
and DVT returns a new PID that still matches after health, session, and the first
JPEG. The next cycle or final cleanup must terminate that same PID.
Fixture reports are labeled `FIXTURE_ONLY` and cannot become live evidence.

Production RT-MMO artifacts are not candidate evidence and are not replaced by
files in this directory.
