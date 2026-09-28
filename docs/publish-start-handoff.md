# Publish start and selected-device handoff

## Entry and recovery

- Preflight observes readiness and owners; it does not stop operations or close TikTok. `needsRelease` lists the selected devices and owner scope that confirmation would release.
- Preflight and Start share one intended request ID. Acceptance-mode denial and an owner that cannot be split both make `canExecute` false. Any non-releasable owner wins over another releasable row on the same device.
- An expired report is discarded so the operator can check again. A failed receipt without a campaign is a durable rejection; retirement removes the local pending marker and broadcasts to both the page and progress monitor. A storage failure does not broadcast retirement. A null receipt never means rejected.
- For a historical pending ID with no receipt, the progress monitor exposes **Hủy yêu cầu chưa được tiếp nhận**. This explicitly and permanently cancels that ID, not the campaign. An IMMEDIATE transaction returns an existing start receipt unchanged, or adopts an existing create receipt as uncertain with its campaign. Only when neither exists does it write a failed `cancelledBeforeAcceptance` tombstone. Its sentinel fingerprint intentionally does not bind an unknown body.
- Start acceptance and legacy creation check the same durable receipt within their write transactions. A delayed same-ID Start/create either wins before cancellation, or loses to the tombstone. The operator must preflight a new ID after a confirmed cancellation.

## Confirmed replacement

- Immediate publication checks queued assignments as well as live owners and Post/link guards. Scheduled creation does not interrupt current work.
- Publication is excluded per selected assignment. The shared exclusion path revokes its composer and guarded link verifier. Nurture stops per selected device. An indivisible operation with devices outside the selection is refused before revocation.
- A handoff-pending marker is written atomically with the exclusion, preventing exclusion settlement from releasing that assignment's account reservation before physical closure.
- Handoff waits for running dispatch jobs, assignment work claims, the device work owner and selected nurture status to drain. Physical closures use the existing shared limit of two and the existing deadline; limits are not increased.
- The exclusive closer rechecks the selected assignment authorization, closes its apps, drops its exclusive context, then persists an assignment-specific release proof. The proof binds the latest exclusion request, device, state, revision, Post intent and evidence; a changed assignment or newer explicit verification resume invalidates it. Only that assignment's account reservation is deleted. The guard then treats its unresolved link as review debt, not a live device owner.
- No campaign-wide publication stop is issued for replacement. Unselected sibling assignments are not excluded or closed. Existing manual-handoff callers retain their prior behavior.
- `require_current_preflight_digest` is unchanged. Fresh readiness is checked after release; cache invalidation does not relax approval of source content or target mapping.

## Verification boundary

Compiler/typecheck/lint results do not establish runtime behavior. Real Tauri UI, selected-device replacement, delayed-request cancellation races, sibling preservation and active-composer exit require coordinator-owned live evidence. No unit/mock suites were added or run for this change. The worker does not access the operational DB or devices.
