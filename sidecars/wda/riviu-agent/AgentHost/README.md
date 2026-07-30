# Standalone XCTest Host Boundary

Project 2 packages the generated `WebDriverAgentRunner-Runner.app` produced by
Xcode `build-for-testing`. A normal UIKit HTTP host is not treated as an
automation agent: the candidate must obtain a real XCTest automation session.

B0 passes only after five clean device cycles complete this order:

```text
terminate exact candidate -> candidate process absent + both device ports closed
-> DVT plain launch + stable new PID
-> protected health -> foreground Settings -> fresh POST /session
-> start MJPEG relay -> first complete JPEG
```

Opening an HTTP port, returning `/status`, or requiring the token is not enough.
If plain launch cannot create the automation session, stop at B0 and preserve the
generated runner/framework topology for comparison with the oracle. Do not add a
UIKit-only server or claim standalone readiness from liveness alone.

The signed runner identity is bound by patch
`patches/0004-signed-artifact-attestation.patch`. It declares source SHA-256,
locked xcconfig SHA-256, Objective-C test result, and exact Xcode version/build as
expandable strings in
the embedded signed
`PlugIns/WebDriverAgentRunner.xctest/Info.plist`, with protocol version as a
hardcoded plist integer `2`. Keep `INFOPLIST_EXPAND_BUILD_SETTINGS=YES`; custom
`INFOPLIST_KEY_*` names do not create arbitrary keys for this upstream target.

On Xcode 26 and newer, the host is finalized only after all four Testing runtime
dependencies are present, nested code is re-signed, and the outer app passes a
fresh deep/strict codesign verification.

Current result: `PENDING_MAC_DEVICE`.
