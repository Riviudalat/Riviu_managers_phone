# Bundle / packaging notes

- `apps/desktop/src-tauri/tauri.conf.json` includes `sidecars/**/*` as bundle resources.
- At runtime, `RIVIU_SIDECAR_ROOT` overrides the resolved sidecar directory.
- Dev resolves sidecars from the monorepo `sidecars/` folder via `CARGO_MANIFEST_DIR`.
- First run without `pymobiledevice3` installed automatically uses the **mock** driver (3 fake iPhones + 24 FPS JPEG frames).
- Production free-signing: replace `sidecars/signer/riviu_signer.py` stub with anisette + zsign pipeline.
- Windows & macOS targets are enabled (`bundle.targets = all`); build with `npm run tauri:build` from `apps/desktop`.
