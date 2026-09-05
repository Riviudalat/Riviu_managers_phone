# Riviu Managers Phone — Design Spec

Date: 2026-07-25

## Goal

Desktop device-farm app (macOS + Windows) to manage and automate multiple iPhones, with Android deferred behind a `DeviceDriver` trait.

## Stack

- Tauri 2 + React/TypeScript UI
- Rust core (registry, SQLite jobs, events)
- Sidecar `pymobiledevice3` for USB/Wi‑Fi iOS ops
- WebDriverAgent for UI automation + MJPEG stream
- Free Apple ID signing helper (7-day profiles)

## UX

GenFarmer-inspired Control Center: live device grid, sidebar quality/size controls, focus stream with remote touch, group control, Jobs/Scripts/Settings tabs. Stream FPS fixed at **24**.

## Modes

- **Mock** (default when pymobiledevice3 unavailable or `RIVIU_MOCK_DEVICES=1`): synthetic devices + generated JPEG frames @ 24 FPS
- **Real**: pymobiledevice3 list/install/screenshot + WDA session/MJPEG

## Out of scope (MVP)

Android driver, visual script builder, fully unattended re-sign, non-MJPEG codecs
