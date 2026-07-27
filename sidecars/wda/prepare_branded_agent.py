#!/usr/bin/env python3
"""Prepare branded Riviumanagersphone iOS agent (WDA shell).

Creates a Payload/ template + Info.plist overrides so free-signing installs
show display name Riviumanagersphone and the orange R icon on SpringBoard.

Usage:
  python3 prepare_branded_agent.py [--wda-app path/to/WebDriverAgentRunner-Runner.app]
"""

from __future__ import annotations

import argparse
import json
import plistlib
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[1]
LOGO = REPO / "logo.jpg"
ICONSET = ROOT / "AppIcon.appiconset"
OUT_DIR = ROOT / "branded"
OUT_APP = OUT_DIR / "Riviumanagersphone.app"
OUT_IPA = ROOT / "Riviumanagersphone.ipa"
BUNDLE_ID = "com.riviu.managersphone.agent"
DISPLAY_NAME = "Riviumanagersphone"


def write_info_plist(app_dir: Path) -> None:
    info_path = app_dir / "Info.plist"
    if info_path.exists():
        with info_path.open("rb") as f:
            info = plistlib.load(f)
    else:
        info = {}

    info["CFBundleDisplayName"] = DISPLAY_NAME
    info["CFBundleName"] = DISPLAY_NAME
    info["CFBundleIdentifier"] = BUNDLE_ID
    info["CFBundleExecutable"] = info.get("CFBundleExecutable") or "WebDriverAgentRunner-Runner"
    info["CFBundlePackageType"] = "APPL"
    info["CFBundleShortVersionString"] = info.get("CFBundleShortVersionString") or "1.0"
    info["CFBundleVersion"] = info.get("CFBundleVersion") or "1"
    info["MinimumOSVersion"] = info.get("MinimumOSVersion") or "15.0"
    info["UIDeviceFamily"] = [1, 2]
    info["CFBundleIcons"] = {
        "CFBundlePrimaryIcon": {
            "CFBundleIconFiles": ["AppIcon"],
            "CFBundleIconName": "AppIcon",
        }
    }
    # Keep WDA URL scheme / capabilities if present
    info.setdefault("NSAppTransportSecurity", {"NSAllowsArbitraryLoads": True})

    with info_path.open("wb") as f:
        plistlib.dump(info, f, sort_keys=False)


def copy_icons(app_dir: Path) -> None:
    dest = app_dir
    if ICONSET.exists():
        for png in ICONSET.glob("*.png"):
            # Flatten common names onto AppIcon*.png inside .app
            target = dest / png.name.replace("AppIcon-", "AppIcon")
            shutil.copy2(png, target)
        # Primary 60pt @2x / @3x used by many sideloaders
        for src_name, dst_name in [
            ("AppIcon-60x60@2x.png", "AppIcon60x60@2x.png"),
            ("AppIcon-60x60@3x.png", "AppIcon60x60@3x.png"),
            ("AppIcon-76x76@2x.png", "AppIcon76x76@2x.png"),
            ("AppIcon-512@2x.png", "iTunesArtwork@2x.png"),
        ]:
            src = ICONSET / src_name
            if src.exists():
                shutil.copy2(src, dest / dst_name)
    elif LOGO.exists():
        shutil.copy2(LOGO, dest / "AppIcon.jpg")


def write_brand_manifest() -> None:
    manifest = {
        "displayName": DISPLAY_NAME,
        "bundleId": BUNDLE_ID,
        "ipa": str(OUT_IPA.name),
        "protocol": {
            "wdaPort": 8100,
            "mjpegPort": 9100,
            "note": "Branding only — WebDriverAgent HTTP/MJPEG protocol unchanged",
        },
    }
    (ROOT / "brand-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def package_ipa(app_dir: Path) -> Path:
    payload = OUT_DIR / "Payload"
    if payload.exists():
        shutil.rmtree(payload)
    payload.mkdir(parents=True)
    staged = payload / "Riviumanagersphone.app"
    if staged.exists():
        shutil.rmtree(staged)
    shutil.copytree(app_dir, staged)

    if OUT_IPA.exists():
        OUT_IPA.unlink()
    # zip as ipa
    shutil.make_archive(str(OUT_IPA.with_suffix("")), "zip", OUT_DIR, "Payload")
    zipped = OUT_IPA.with_suffix(".zip")
    if zipped.exists():
        zipped.rename(OUT_IPA)
    return OUT_IPA


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--wda-app",
        type=Path,
        help="Optional path to an existing WebDriverAgentRunner-Runner.app to rebrand",
    )
    args = parser.parse_args()

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    if OUT_APP.exists():
        shutil.rmtree(OUT_APP)
    OUT_APP.mkdir(parents=True)

    if args.wda_app and args.wda_app.exists():
        shutil.copytree(args.wda_app, OUT_APP, dirs_exist_ok=True)
    else:
        # Stub agent shell — signer/build pipeline replaces binary with real WDA build.
        (OUT_APP / "README.txt").write_text(
            "Place WebDriverAgentRunner-Runner binary here or pass --wda-app.\n"
            "Info.plist and icons are branded as Riviumanagersphone.\n"
        )

    write_info_plist(OUT_APP)
    copy_icons(OUT_APP)
    write_brand_manifest()
    ipa = package_ipa(OUT_APP)
    print(json.dumps({"ok": True, "ipa": str(ipa), "bundleId": BUNDLE_ID, "displayName": DISPLAY_NAME}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
