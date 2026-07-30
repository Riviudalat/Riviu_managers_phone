# Riviu unified on-device agent

The product runtime installs `RiviuAgent.ipa` (`com.mrph.svc`) and validates it
against `agent-manifest.json` before every install or repair. It provides the
RT-MMO control channel on port 8906 and MJPEG on port 9093.

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
