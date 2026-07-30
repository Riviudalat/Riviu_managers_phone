# Gate A - RT-MMO Forensic Inventory

- [x] **IPA inventory evidence chain**: recomputed 20 entries and 4 Mach-O images from artifact bytes
- [x] **Artifact hash**: 3182711 bytes
- [x] **Mach-O inventory**: 4 images; 3 runtime; 1 dSYM
- [x] **Signing metadata**: 4 provisioning entitlements; 1 signed runtime images
- [x] **DWARF evidence**: 42 compile units; 3 subprograms; 83 line rows
- [x] **Baseline evidence chain**: npm integrity verified; archive SHA-256 0c52fc0dcc6f837287be02a593d96d8ef28563c90b4d41f629830e84878f6bbb; source 7b52b871e3aca1eaab16163343af1271c25b7a31feca09138f96d025d70d030d; inventory 01fa311e51ff9a0567dd51ba3cf98b6bda031b40bbf752a44e9428bc49357673
- [x] **WDA baseline**: framework 15.1.4; baseline 15.1.4
- [x] **Static delta evidence**: 533 exported symbols; oracle/baseline-only: 44/297 classes, 714/1185 methods, 80/101 routes; complete source/image provenance
- [x] **Route path inventory**: 8 contract routes; 8 paths confirmed; 77 oracle-only, 100 baseline-only, 30 shared undocumented candidates; method/auth/session/body remain contract assertions
- [x] **Report redaction**: inventory and baseline reports scanned

## Contract Route Path Evidence

| Method | Path | Auth | Session | Body | Status | Path evidence | Contract source |
|---|---|---|---|---|---|---|---|
| GET | `/status` | exempt | none | none | path-confirmed | oracle Mach-O + WDA baseline | crates/ios-driver/src/wda.rs |
| GET | `/wda/locked` | protected | none | none | path-confirmed | oracle Mach-O + WDA baseline | sidecars/pymobiledevice3/riviu_pmd.py |
| POST | `/session` | protected | none | required: `capabilities`, `capabilities.firstMatch` | path-confirmed | oracle Mach-O + WDA baseline | crates/ios-driver/src/wda.rs |
| DELETE | `/session/{sessionId}` | protected | required | none | path-confirmed | oracle Mach-O | sidecars/pymobiledevice3/riviu_pmd.py |
| POST | `/wda/swipe` | protected | none | required: `delay`, `fromX`, `fromY`, `toX`, `toY` | path-confirmed | oracle Mach-O | crates/ios-driver/src/wda.rs |
| POST | `/wda/tap` | protected | none | required: `x`, `y` | path-confirmed | oracle Mach-O | crates/ios-driver/src/wda.rs |
| POST | `/session/{sessionId}/wda/keys` | protected | required | required: `value` | path-confirmed | WDA baseline | crates/ios-driver/src/wda.rs |
| GET | `/screenshot` | protected | none | none | path-confirmed | oracle Mach-O + WDA baseline | crates/ios-driver/src/wda.rs |

## Additional Route Path Evidence

| Path | Status | Evidence |
|---|---|---|
| `/actions` | oracle-only | oracle Mach-O route candidate |
| `/alert/accept` | path-confirmed | oracle Mach-O + WDA baseline |
| `/alert/dismiss` | path-confirmed | oracle Mach-O + WDA baseline |
| `/alert/text` | path-confirmed | oracle Mach-O + WDA baseline |
| `/appium/settings` | oracle-only | oracle Mach-O route candidate |
| `/calibrate` | oracle-only | oracle Mach-O route candidate |
| `/deactivateApp` | baseline-only | WDA baseline route candidate |
| `/element` | oracle-only | oracle Mach-O route candidate |
| `/element/:uuid/attribute/:name` | oracle-only | oracle Mach-O route candidate |
| `/element/:uuid/clear` | oracle-only | oracle Mach-O route candidate |
| `/element/:uuid/click` | oracle-only | oracle Mach-O route candidate |
| `/element/:uuid/displayed` | oracle-only | oracle Mach-O route candidate |
| `/element/:uuid/element` | oracle-only | oracle Mach-O route candidate |
| `/element/:uuid/elements` | oracle-only | oracle Mach-O route candidate |
| `/element/:uuid/enabled` | oracle-only | oracle Mach-O route candidate |
| `/element/:uuid/name` | oracle-only | oracle Mach-O route candidate |
| `/element/:uuid/rect` | oracle-only | oracle Mach-O route candidate |
| `/element/:uuid/screenshot` | oracle-only | oracle Mach-O route candidate |
| `/element/:uuid/selected` | oracle-only | oracle Mach-O route candidate |
| `/element/:uuid/text` | oracle-only | oracle Mach-O route candidate |
| `/element/:uuid/value` | oracle-only | oracle Mach-O route candidate |
| `/element/active` | oracle-only | oracle Mach-O route candidate |
| `/elements` | oracle-only | oracle Mach-O route candidate |
| `/health` | oracle-only | oracle Mach-O route candidate |
| `/orientation` | path-confirmed | oracle Mach-O + WDA baseline |
| `/rotation` | path-confirmed | oracle Mach-O + WDA baseline |
| `/screenshot/:uuid` | oracle-only | oracle Mach-O route candidate |
| `/session/{sessionId}/actions` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/alert/accept` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/alert/dismiss` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/alert/text` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/appium/settings` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/deactivateApp` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/attribute/:name` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/attribute/focused` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/clear` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/click` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/displayed` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/element` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/elements` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/enabled` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/name` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/rect` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/screenshot` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/selected` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/text` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/:uuid/value` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/element/active` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/elements` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/orientation` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/rotation` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/screenshot` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/screenshot/:uuid` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/source` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/timeouts` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/url` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/accessibleSource` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/activeAppInfo` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/alert/buttons` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/apps/activate` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/apps/launch` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/apps/list` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/apps/state` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/apps/terminate` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/batteryInfo` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/deactivateApp` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/device/info` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/device/location` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/deviceOrientation` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/doubleTap` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/dragfromtoforduration` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/accessibilityContainer` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/accessible` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/doubleTap` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/dragfromtoforduration` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/focuse` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/forceTouch` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/getVisibleCells` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/keyboardInput` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/pinch` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/pressAndDragWithVelocity` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/rotate` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/scroll` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/scrollTo` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/swipe` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/tap` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/tapWithNumberOfTaps` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/touchAndHold` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/element/:uuid/twoFingerTap` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/expectNotification` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/forceTouch` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/getPasteboard` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/keyboard/dismiss` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/lock` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/locked` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/performAccessibilityAudit` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/performIoHidEvent` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/pickerwheel/:uuid/select` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/pinch` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/pressAndDragWithVelocity` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/pressButton` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/resetAppAuth` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/rotate` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/screen` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/scroll` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/setPasteboard` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/simulatedLocation` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/siri/activate` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/swipe` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/tap` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/tapWithNumberOfTaps` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/touchAndHold` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/touch_id` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/twoFingerTap` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/unlock` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/video` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/video/start` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/video/stop` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/voiceOver/currentSpeech` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/voiceOver/disable` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/voiceOver/enable` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/voiceOver/enabled` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/wda/voiceOver/move` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/window/rect` | baseline-only | WDA baseline route candidate |
| `/session/{sessionId}/window/size` | baseline-only | WDA baseline route candidate |
| `/source` | path-confirmed | oracle Mach-O + WDA baseline |
| `/timeouts` | oracle-only | oracle Mach-O route candidate |
| `/url` | oracle-only | oracle Mach-O route candidate |
| `/wda/accessibleSource` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/activeAppInfo` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/alert/buttons` | oracle-only | oracle Mach-O route candidate |
| `/wda/apps/activate` | oracle-only | oracle Mach-O route candidate |
| `/wda/apps/launch` | oracle-only | oracle Mach-O route candidate |
| `/wda/apps/launchUnattached` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/apps/list` | oracle-only | oracle Mach-O route candidate |
| `/wda/apps/state` | oracle-only | oracle Mach-O route candidate |
| `/wda/apps/terminate` | oracle-only | oracle Mach-O route candidate |
| `/wda/batteryInfo` | oracle-only | oracle Mach-O route candidate |
| `/wda/deactivateApp` | oracle-only | oracle Mach-O route candidate |
| `/wda/device/appearance` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/device/info` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/device/location` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/deviceOrientation` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/doubleTap` | oracle-only | oracle Mach-O route candidate |
| `/wda/dragfromtoforduration` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/accessibilityContainer` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/accessible` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/doubleTap` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/dragfromtoforduration` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/forceTouch` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/getVisibleCells` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/keyboardInput` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/pinch` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/pressAndDragWithVelocity` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/rotate` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/scroll` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/scrollTo` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/swipe` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/tap` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/tapWithNumberOfTaps` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/touchAndHold` | oracle-only | oracle Mach-O route candidate |
| `/wda/element/:uuid/twoFingerTap` | oracle-only | oracle Mach-O route candidate |
| `/wda/expectNotification` | oracle-only | oracle Mach-O route candidate |
| `/wda/forceTouch` | oracle-only | oracle Mach-O route candidate |
| `/wda/getPasteboard` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/healthcheck` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/homescreen` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/keyboard/dismiss` | oracle-only | oracle Mach-O route candidate |
| `/wda/keys` | oracle-only | oracle Mach-O route candidate |
| `/wda/lock` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/performAccessibilityAudit` | oracle-only | oracle Mach-O route candidate |
| `/wda/performIoHidEvent` | oracle-only | oracle Mach-O route candidate |
| `/wda/pickerwheel/:uuid/select` | oracle-only | oracle Mach-O route candidate |
| `/wda/pinch` | oracle-only | oracle Mach-O route candidate |
| `/wda/pressAndDragWithVelocity` | oracle-only | oracle Mach-O route candidate |
| `/wda/pressButton` | oracle-only | oracle Mach-O route candidate |
| `/wda/pushFile` | oracle-only | oracle Mach-O route candidate |
| `/wda/pushImage` | oracle-only | oracle Mach-O route candidate |
| `/wda/pushVideo` | oracle-only | oracle Mach-O route candidate |
| `/wda/resetAppAuth` | oracle-only | oracle Mach-O route candidate |
| `/wda/rotate` | oracle-only | oracle Mach-O route candidate |
| `/wda/rt/refresh_stream` | oracle-only | oracle Mach-O route candidate |
| `/wda/screen` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/scroll` | oracle-only | oracle Mach-O route candidate |
| `/wda/setPasteboard` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/shutdown` | oracle-only | oracle Mach-O route candidate |
| `/wda/simulatedLocation` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/siri/activate` | oracle-only | oracle Mach-O route candidate |
| `/wda/tapWithNumberOfTaps` | oracle-only | oracle Mach-O route candidate |
| `/wda/touchAndHold` | oracle-only | oracle Mach-O route candidate |
| `/wda/touchDown` | oracle-only | oracle Mach-O route candidate |
| `/wda/touchMove` | oracle-only | oracle Mach-O route candidate |
| `/wda/touchUp` | oracle-only | oracle Mach-O route candidate |
| `/wda/touch_id` | oracle-only | oracle Mach-O route candidate |
| `/wda/twoFingerTap` | oracle-only | oracle Mach-O route candidate |
| `/wda/unlock` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/video` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/video/start` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/video/stop` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/voiceOver/currentSpeech` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/voiceOver/disable` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/voiceOver/enable` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/voiceOver/enabled` | path-confirmed | oracle Mach-O + WDA baseline |
| `/wda/voiceOver/move` | path-confirmed | oracle Mach-O + WDA baseline |
| `/window/rect` | oracle-only | oracle Mach-O route candidate |
| `/window/size` | path-confirmed | oracle Mach-O + WDA baseline |

## Evidence Boundary

The runtime images are stripped and the bundled dSYM exposes only surviving runner symbols. Gate A records exported symbols, DWARF ranges/line tables, filtered Objective-C metadata, route paths, typed contract assertions, and provenance. Path-confirmed does not prove the declared HTTP method, auth, session, or body semantics, and Gate A does not claim a recovered feature call graph. Project 2 must add a contract or probe before implementing any feature-specific delta.

Decision: PASS
