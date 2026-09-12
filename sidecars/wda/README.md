# Riviu unified on-device agent

Riviu Agent is the on-device iPhone component that supplies screen frames and
accepts touch, swipe and text commands. The desktop selects an artifact in
`apps/desktop/src-tauri/src/agent_runtime.rs`; every install/repair validates its
manifest and IPA hash. Android uses its own ADB/UiAutomator2/helper path.

| Mode | Artifact | Control / MJPEG | Declared capabilities |
|---|---|---|---|
| candidate / riviu-agent / full | `RiviuAgent-candidate.ipa`, `candidate-manifest.json` | 8916 / 9094 | stream, tap, swipe, clipboard, text, pushMedia |
| text / candidate-text | `RiviuAgent-text.ipa`, `text-manifest.json` | 8916 / 9094 | stream, tap, swipe, clipboard, text |
| rt-mmo | `RiviuAgent.ipa`, `agent-manifest.json` | 8906 / 9093 | stream, tap, swipe, text, clipboard, pushMedia |

The desktop passes `prefer_candidate=true`. Without overrides, the build-time
`RIVIU_DEFAULT_AGENT_MODE` wins, otherwise candidate is selected. Runtime override
order is `RIVIU_AGENT_MODE`, then `RIVIU_WDA_BACKEND`; `RIVIU_AGENT_MANIFEST` can
select an explicit manifest. Full uses an ephemeral session token; candidate/text
use the candidate credential path. Manifest capability declarations and historical
gate results do not certify a newly connected device or a currently valid signature.

## RT-MMO reference artifact

Installed identity is bound to bundle/version/build plus payload app
`777wealth.app` and the signer identity recorded in the manifest. Bundle and
version alone are insufficient because the revoked Wuhan build reused the same
`com.mrph.svc` / `1.0` / `1` values.

The bundled artifact is the `777wealth.app` release updated on 2026-07-24 and
signed with the Beijing enterprise profile `chuvendor`. Its SHA-256 is
`8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea`; the profile
expires on 2027-07-24. Install, launch, protected auth and MJPEG were verified on
an iPhone 8 running iOS 16.7.15 on 2026-07-28.

Do not restore the older Wuhan `csc-native-ios.app` artifact with SHA-256
`628b4b3b36dbe2fa1e4c753d1d7b004443d00c829bf8581a28101ab499b7cb5a`. Its
signing identity is revoked and installation returns `0xe8008018`, even though
the embedded profile lists 2026-08-07 as its expiration date.

The current agent accepts its fixed RT-MMO token, not an arbitrary `FARM_KEY`.
Provide `RIVIU_RTMMO_TOKEN` for the first desktop launch so it is migrated into
the native credential store. Later desktop and harness launches read the
credential store. Supplying a nonblank environment token again explicitly
replaces a stale stored value; a random token is never generated.

`build_and_install.py` and the stock `com.riviu.managersphone.agent` WDA remain
legacy diagnostics only. They do not replace the unified runtime or provide the
trusted TikTok text-input path.
