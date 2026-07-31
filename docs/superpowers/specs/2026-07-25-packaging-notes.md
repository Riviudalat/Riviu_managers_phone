# Bundle / packaging notes

- Base resources are mapped explicitly in `apps/desktop/src-tauri/tauri.conf.json`.
  Release builds merge `target/tauri-sidecar.conf.json`, which maps the attested
  native runtime to `sidecars/pymobiledevice3/runtime/`.
- Packaged code prefers `runtime/riviu-pmd(.exe)`. System Python is only a source
  development fallback. `RIVIU_SIDECAR_ROOT` still overrides the sidecar root.
- Build the native runtime with
  `python scripts/build_desktop_sidecar.py`; install its exact dependencies from
  `sidecars/pymobiledevice3/requirements-build.txt` first. The complete resolved
  closure is constrained by `requirements-lock.txt` and recorded in the manifest.
  Unrelated local distributions are ignored, but every active locked distribution
  must be installed at its exact version; CI builds from a clean interpreter.
- The PyInstaller runtime pins pymobiledevice3/tidevice, embeds the signer runner,
  emits `runtime-manifest.json`, and smoke-tests process-control, tidevice and the
  signer and pinned signing resources. IPython/Jedi are intentionally excluded
  because the product never exposes pymobiledevice3's interactive shell.
- Packaged legacy re-signing uses the checked-in WDA 16.0.0 tree and
  `legacy-wda-source-lock.json`; it copies verified source into the user cache before
  building. Its digest canonicalizes CRLF to LF and binds the explicit POSIX
  executable-mode list. Never restore an unpinned network clone, hash a Windows
  checkout without normalization, or write into app resources.
- Windows uses the WebView2 download bootstrapper and current-user NSIS install.
  Apple Devices/Apple Mobile Device Support remains a vendor USB-driver prerequisite.
- CI builds Windows x64, macOS arm64 and macOS x64 separately with exact Python,
  Node and Rust toolchains and commit-pinned official actions. Every `main` push
  uploads 30-day artifacts; `v*` tags create an immutable GitHub Release. Existing
  releases are never overwritten; publish a new version/tag instead.
- CI administratively extracts MSI, silently installs/uninstalls NSIS, and mounts
  the uploaded DMG read-only. It re-runs packaged runtime/signing-resource smoke
  tests and compares IPA/resource trees including symlinks and modes. macOS also
  verifies architecture and deep code signature. Windows parses the installed
  desktop PE from both MSI and NSIS, requires the exact Cargo binary name and x64,
  then records each bundle's size/hash separately because Tauri patches bundle-type
  metadata into the executable.
- Release tags must exactly match all three application versions (`v<version>`).
- CI macOS artifacts use ad-hoc signing until Developer ID/notarization secrets are
  configured. Xcode is required only for rebuilding/re-signing the iPhone agent.
- Production free-signing still needs the Xcode/Apple-account flow; do not claim an
  anisette/zsign pipeline until that separate implementation and live gate exist.
