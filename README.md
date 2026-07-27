# Riviumanagersphone

Desktop iPhone device-farm manager (Tauri 2 + Rust + React). Control Center inspired by GenFarmer with **24 FPS** live streams, remote touch, and free-Apple-ID install of the on-device **Riviumanagersphone** agent (orange R icon).

## Quick start (real devices)

```bash
source "$HOME/.cargo/env"
python3 -m pip install -r sidecars/pymobiledevice3/requirements.txt
cd apps/desktop
npm install
npm run tauri:dev
```

1. Plug iPhone / Trust This Computer (Wi‑Fi pair optional).
2. iOS 17+: keep a `pymobiledevice3` tunnel running.
3. In Control Center → **Refresh devices**.
4. **Cài / Re-sign Riviumanagersphone** — installs the branded agent on the phone.

Mock farm (dev only): `export RIVIU_MOCK_DEVICES=1`.

## Branding

- Desktop app name / dock icon: **Riviumanagersphone** (from `logo.jpg`)
- On-iPhone agent: display name **Riviumanagersphone**, bundle `com.riviu.managersphone.agent`
- Prepare agent IPA: `python3 sidecars/wda/prepare_branded_agent.py`

## Workspace

```
apps/desktop/          Tauri + React UI
crates/core/           registry, SQLite, job queue
crates/ios-driver/     pymobiledevice3 + WDA (+ optional mock)
crates/signing/        free Apple ID signing
crates/script-engine/  JSON scripts
sidecars/wda/          branded Riviumanagersphone.ipa + icons
sidecars/pymobiledevice3/
sidecars/signer/
```
