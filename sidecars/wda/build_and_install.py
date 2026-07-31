#!/usr/bin/env python3
"""Build branded Riviumanagersphone agent (WebDriverAgent) and install on device.

Requires:
  - Full Xcode.app (not only Command Line Tools)
  - Apple ID added in Xcode → Settings → Accounts (free Personal Team OK)
  - Device trusted over USB

Usage:
  python3 build_and_install.py --udid <UDID> [--team-id TEAMID]
"""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import os
import plistlib
import re
import shutil
import stat
import subprocess
import sys
from pathlib import Path

RESOURCE_ROOT = Path(__file__).resolve().parent
SOURCE_TEMPLATE = RESOURCE_ROOT / "WebDriverAgent"
SOURCE_LOCK = RESOURCE_ROOT / "legacy-wda-source-lock.json"
PACKAGED_LOGO = RESOURCE_ROOT / "logo.jpg"
DEVELOPMENT_LOGO = RESOURCE_ROOT.parents[1] / "logo.jpg"
LOGO = PACKAGED_LOGO if PACKAGED_LOGO.is_file() else DEVELOPMENT_LOGO
ICONSET = RESOURCE_ROOT / "AppIcon.appiconset"


def _default_work_root() -> Path:
    configured = os.environ.get("RIVIU_SIGNING_WORK_ROOT")
    if configured:
        return Path(configured).expanduser().resolve()
    if sys.platform == "darwin":
        return (
            Path.home()
            / "Library"
            / "Caches"
            / "com.riviu.managersphone"
            / "signing"
        )
    if sys.platform == "win32":
        local_app_data = os.environ.get("LOCALAPPDATA")
        if local_app_data:
            return Path(local_app_data) / "RiviuManagersPhone" / "signing"
    return Path.home() / ".cache" / "riviu-managers-phone" / "signing"


WORK_ROOT = _default_work_root()
BUILD_ROOT = WORK_ROOT / "unconfigured"
WDA_SRC = BUILD_ROOT / "WebDriverAgent"
BUNDLE_ID = "com.riviu.managersphone.agent"
DISPLAY_NAME = "Riviumanagersphone"
DERIVED = BUILD_ROOT / "DerivedData"
PRODUCT_DIR = DERIVED / "Build" / "Products" / "Debug-iphoneos"


def emit(obj: dict) -> None:
    print(json.dumps(obj, ensure_ascii=True))


def configure_build_workspace(udid: str) -> None:
    global BUILD_ROOT, WDA_SRC, DERIVED, PRODUCT_DIR
    device_key = hashlib.sha256(udid.encode("utf-8")).hexdigest()[:20]
    BUILD_ROOT = WORK_ROOT / "devices" / device_key
    WDA_SRC = BUILD_ROOT / "WebDriverAgent"
    DERIVED = BUILD_ROOT / "DerivedData"
    PRODUCT_DIR = DERIVED / "Build" / "Products" / "Debug-iphoneos"


def run(cmd: list[str], cwd: Path | None = None, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(
        cmd,
        cwd=str(cwd) if cwd else None,
        check=check,
        text=True,
        capture_output=True,
    )


def require_xcode() -> Path:
    apps = sorted(Path("/Applications").glob("Xcode*.app"))
    if not apps:
        raise RuntimeError(
            "Chưa có Xcode.app. Cài Xcode từ App Store (miễn phí), mở 1 lần, "
            "đăng nhập Apple ID tại Xcode → Settings → Accounts, rồi chạy lại."
        )
    xcode = apps[0]
    # Point active developer dir at full Xcode when possible
    dev_dir = xcode / "Contents" / "Developer"
    current = subprocess.run(["xcode-select", "-p"], capture_output=True, text=True)
    if "CommandLineTools" in (current.stdout or ""):
        # May need sudo; try and surface guidance if fails
        switched = subprocess.run(
            ["sudo", "-n", "xcode-select", "-s", str(dev_dir)],
            capture_output=True,
            text=True,
        )
        if switched.returncode != 0:
            raise RuntimeError(
                f"Cần chuyển developer dir sang Xcode:\n"
                f"  sudo xcode-select -s '{dev_dir}'\n"
                f"Rồi mở Xcode một lần để chấp nhận license."
            )
    version = run(["xcodebuild", "-version"])
    if version.returncode != 0:
        raise RuntimeError(version.stderr or "xcodebuild unavailable")
    return xcode


def _canonical_file_payload(path: Path) -> bytes:
    payload = path.read_bytes()
    try:
        payload.decode("utf-8")
    except UnicodeDecodeError:
        return payload
    return payload.replace(b"\r\n", b"\n")


def source_tree_sha256(
    root: Path, *, executable_paths: tuple[str, ...] = ()
) -> str:
    expected_executables = set(executable_paths)
    seen_files: set[str] = set()
    digest = hashlib.sha256()
    paths = sorted(
        root.rglob("*"),
        key=lambda path: path.relative_to(root).as_posix().encode("utf-8"),
    )
    for path in paths:
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            target = os.readlink(path)
            payload = target.encode("utf-8")
            kind = b"symlink"
            mode = 0o777
        elif path.is_file():
            payload = _canonical_file_payload(path)
            kind = b"file"
            seen_files.add(relative)
            mode = 0o755 if relative in expected_executables else 0o644
            if os.name != "nt":
                actual_executable = bool(stat.S_IMODE(path.stat().st_mode) & 0o111)
                if actual_executable != (relative in expected_executables):
                    raise RuntimeError(
                        f"Pinned source mode mismatch for {relative}: expected {mode:o}"
                    )
        else:
            continue
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(kind)
        digest.update(b"\0")
        digest.update(f"{mode:o}".encode("ascii"))
        digest.update(b"\0")
        digest.update(str(len(payload)).encode("ascii"))
        digest.update(b"\0")
        digest.update(hashlib.sha256(payload).hexdigest().encode("ascii"))
        digest.update(b"\n")
    missing_executables = sorted(expected_executables - seen_files)
    if missing_executables:
        raise RuntimeError(
            "Pinned executable paths are missing: " + ", ".join(missing_executables)
        )
    return digest.hexdigest()


def verify_resource_bundle() -> dict:
    try:
        lock = json.loads(SOURCE_LOCK.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise RuntimeError(f"Invalid pinned WDA source lock: {exc}") from exc
    expected = {
        "schemaVersion": 2,
        "package": "appium-webdriveragent",
        "version": "16.0.0",
    }
    for key, value in expected.items():
        if lock.get(key) != value:
            raise RuntimeError(
                f"Pinned WDA source lock {key} mismatch: expected {value!r}"
            )
    for field in ("treeSha256", "logoSha256", "iconSetTreeSha256"):
        digest = lock.get(field)
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise RuntimeError(f"Pinned WDA source lock has an invalid {field}")
    executable_paths = lock.get("executablePaths")
    if not isinstance(executable_paths, list) or not all(
        isinstance(path, str) for path in executable_paths
    ):
        raise RuntimeError("Pinned WDA source lock has invalid executablePaths")
    if executable_paths != sorted(set(executable_paths)) or not all(
        path
        and Path(path).as_posix() == path
        and not Path(path).is_absolute()
        and ".." not in Path(path).parts
        for path in executable_paths
    ):
        raise RuntimeError("Pinned WDA source lock has invalid executablePaths")
    executable_paths_tuple = tuple(executable_paths)
    tree_sha256 = lock["treeSha256"]
    if not (SOURCE_TEMPLATE / "WebDriverAgent.xcodeproj").is_dir():
        raise RuntimeError("Pinned WebDriverAgent source is missing from app resources")
    package = json.loads((SOURCE_TEMPLATE / "package.json").read_text(encoding="utf-8"))
    if package.get("name") != expected["package"] or package.get("version") != expected["version"]:
        raise RuntimeError("Pinned WebDriverAgent package identity does not match its lock")
    actual = source_tree_sha256(
        SOURCE_TEMPLATE, executable_paths=executable_paths_tuple
    )
    if actual != tree_sha256:
        raise RuntimeError(
            "Pinned WebDriverAgent source integrity mismatch: "
            f"expected {tree_sha256}, got {actual}"
        )
    if not LOGO.is_file() or not ICONSET.is_dir():
        raise RuntimeError("Pinned WDA branding assets are missing from app resources")
    if hashlib.sha256(LOGO.read_bytes()).hexdigest() != lock["logoSha256"]:
        raise RuntimeError("Pinned WDA logo integrity mismatch")
    if source_tree_sha256(ICONSET) != lock["iconSetTreeSha256"]:
        raise RuntimeError("Pinned WDA icon set integrity mismatch")
    resource_root = RESOURCE_ROOT.resolve()
    work_root = WORK_ROOT.resolve()
    if work_root == resource_root or resource_root in work_root.parents:
        raise RuntimeError("Signing work directory must be outside packaged app resources")
    return lock


def ensure_wda_checkout() -> None:
    lock = verify_resource_bundle()
    WORK_ROOT.mkdir(parents=True, exist_ok=True)
    work_root = WORK_ROOT.resolve()
    destination = WDA_SRC.resolve()
    if work_root not in destination.parents:
        raise RuntimeError("Refusing to prepare WDA outside the signing work directory")

    build_root = WDA_SRC.parent
    temporary = build_root / f".WebDriverAgent-{os.getpid()}.tmp"
    build_root.mkdir(parents=True, exist_ok=True)
    for path in (temporary, WDA_SRC):
        resolved = path.resolve()
        if work_root not in resolved.parents:
            raise RuntimeError("Refusing to replace a path outside the signing work directory")
        if path.exists() or path.is_symlink():
            if path.is_dir() and not path.is_symlink():
                shutil.rmtree(path)
            else:
                path.unlink()
    shutil.copytree(SOURCE_TEMPLATE, temporary, symlinks=True)
    actual = source_tree_sha256(
        temporary, executable_paths=tuple(lock["executablePaths"])
    )
    if actual != lock["treeSha256"]:
        shutil.rmtree(temporary)
        raise RuntimeError("Copied WebDriverAgent source failed integrity verification")
    temporary.replace(WDA_SRC)


def sync_app_icons() -> None:
    """Regenerate AppIcon asset catalog from repo logo.jpg (source of truth)."""
    if not LOGO.exists():
        raise RuntimeError(f"Missing brand logo: {LOGO}")

    dest = (
        WDA_SRC
        / "WebDriverAgentRunner"
        / "Assets.xcassets"
        / "AppIcon.appiconset"
    )
    dest.mkdir(parents=True, exist_ok=True)

    # Wipe previous (often a washed-out/broken 1024)
    for old in dest.glob("*.png"):
        old.unlink()

    # Master 1024 from logo
    master = dest / "icon-1024.png"
    run(
        [
            "sips",
            "-z",
            "1024",
            "1024",
            "-s",
            "format",
            "png",
            str(LOGO),
            "--out",
            str(master),
        ]
    )

    # Explicit iPhone/iPad slots so older iOS doesn't get a blank SpringBoard icon
    sizes = {
        "AppIcon-20x20@2x.png": 40,
        "AppIcon-20x20@3x.png": 60,
        "AppIcon-29x29@2x.png": 58,
        "AppIcon-29x29@3x.png": 87,
        "AppIcon-40x40@2x.png": 80,
        "AppIcon-40x40@3x.png": 120,
        "AppIcon-60x60@2x.png": 120,
        "AppIcon-60x60@3x.png": 180,
        "AppIcon-76x76@1x.png": 76,
        "AppIcon-76x76@2x.png": 152,
        "AppIcon-83.5x83.5@2x.png": 167,
    }
    for name, px in sizes.items():
        run(
            [
                "sips",
                "-z",
                str(px),
                str(px),
                str(master),
                "--out",
                str(dest / name),
            ]
        )

    contents = {
        "images": [
            {"filename": "AppIcon-20x20@2x.png", "idiom": "iphone", "scale": "2x", "size": "20x20"},
            {"filename": "AppIcon-20x20@3x.png", "idiom": "iphone", "scale": "3x", "size": "20x20"},
            {"filename": "AppIcon-29x29@2x.png", "idiom": "iphone", "scale": "2x", "size": "29x29"},
            {"filename": "AppIcon-29x29@3x.png", "idiom": "iphone", "scale": "3x", "size": "29x29"},
            {"filename": "AppIcon-40x40@2x.png", "idiom": "iphone", "scale": "2x", "size": "40x40"},
            {"filename": "AppIcon-40x40@3x.png", "idiom": "iphone", "scale": "3x", "size": "40x40"},
            {"filename": "AppIcon-60x60@2x.png", "idiom": "iphone", "scale": "2x", "size": "60x60"},
            {"filename": "AppIcon-60x60@3x.png", "idiom": "iphone", "scale": "3x", "size": "60x60"},
            {"filename": "AppIcon-76x76@1x.png", "idiom": "ipad", "scale": "1x", "size": "76x76"},
            {"filename": "AppIcon-76x76@2x.png", "idiom": "ipad", "scale": "2x", "size": "76x76"},
            {"filename": "AppIcon-83.5x83.5@2x.png", "idiom": "ipad", "scale": "2x", "size": "83.5x83.5"},
            {
                "filename": "icon-1024.png",
                "idiom": "ios-marketing",
                "scale": "1x",
                "size": "1024x1024",
            },
        ],
        "info": {"author": "riviu", "version": 1},
    }
    (dest / "Contents.json").write_text(json.dumps(contents, indent=2) + "\n")

    # Sanity: orange R must survive resize
    try:
        from PIL import Image

        pixels = list(Image.open(master).convert("RGB").getdata())
        orange = sum(1 for r, g, b in pixels if r > 180 and g < 140 and b < 80)
        if orange / max(len(pixels), 1) < 0.05:
            raise RuntimeError(
                "Generated AppIcon looks washed-out (too little orange). "
                f"Check {LOGO}"
            )
    except ImportError:
        pass


def brand_info_plists() -> None:
    # Patch integration app / runner display names where present
    for plist_path in WDA_SRC.rglob("Info.plist"):
        try:
            with plist_path.open("rb") as f:
                info = plistlib.load(f)
        except Exception:
            continue
        changed = False
        if "CFBundleDisplayName" in info or plist_path.parent.name.endswith("Runner"):
            info["CFBundleDisplayName"] = DISPLAY_NAME
            changed = True
        if "CFBundleName" in info:
            info["CFBundleName"] = DISPLAY_NAME
            changed = True
        if changed:
            with plist_path.open("wb") as f:
                plistlib.dump(info, f)


def _team_from_pbxproj() -> str | None:
    pbx = WDA_SRC / "WebDriverAgent.xcodeproj" / "project.pbxproj"
    if not pbx.exists():
        return None
    text = pbx.read_text(errors="ignore")
    # Prefer non-empty DEVELOPMENT_TEAM written by Xcode UI
    teams = re.findall(r"DEVELOPMENT_TEAM\s*=\s*([A-Z0-9]{10})\s*;", text)
    return teams[-1] if teams else None


def _team_from_apple_dev_cert_ou() -> str | None:
    """Real Team ID is certificate OU, not the (XXXXXXXXXX) in the common name."""
    pem = subprocess.run(
        ["security", "find-certificate", "-c", "Apple Development", "-p"],
        capture_output=True,
        text=True,
    )
    if pem.returncode != 0 or "BEGIN CERTIFICATE" not in (pem.stdout or ""):
        return None
    subject = subprocess.run(
        ["openssl", "x509", "-noout", "-subject"],
        input=pem.stdout,
        capture_output=True,
        text=True,
    ).stdout
    # subject=.../OU=VJQ9MM29VH/...  or OU = VJQ9MM29VH
    match = re.search(r"OU\s*=\s*([A-Z0-9]{10})", subject or "")
    return match.group(1) if match else None


def detect_team_id(explicit: str | None) -> str:
    if explicit:
        return explicit
    env = os.environ.get("RIVIU_DEVELOPMENT_TEAM") or os.environ.get("DEVELOPMENT_TEAM")
    if env:
        return env

    for candidate in (_team_from_pbxproj(), _team_from_apple_dev_cert_ou()):
        if candidate:
            return candidate

    raise RuntimeError(
        "Chưa có Apple Development certificate / Team ID.\n"
        "Mở Xcode → Settings → Accounts → Add Apple ID → "
        "Manage Certificates → + Apple Development.\n"
        "Rồi trong WebDriverAgentRunner → Signing & Capabilities → chọn Team.\n"
        "Hoặc truyền --team-id XXXXXXXXXX (Personal Team)."
    )


def _codesign_identity_ok() -> bool:
    out = subprocess.run(
        ["security", "find-identity", "-v", "-p", "codesigning"],
        capture_output=True,
        text=True,
    ).stdout
    return "Apple Development" in (out or "") and "0 valid identities found" not in (out or "")


def _friendly_xcode_error(stdout: str, stderr: str, team_id: str) -> str:
    blob = f"{stdout}\n{stderr}"
    tips: list[str] = []
    if "No Account for Team" in blob:
        tips.append(
            f"Xcode chưa đăng nhập team {team_id}.\n"
            "Mở Xcode → Settings → Accounts → Add Apple ID "
            "(cattfan239@gmail.com hoặc Apple ID của team) → Download Manual Profiles."
        )
    if "No profiles for" in blob:
        tips.append(
            "Chưa có provisioning profile.\n"
            "Trong Xcode Accounts, chọn team → Manage Certificates → "
            "+ Apple Development (tạo mới trên máy này).\n"
            "Rồi mở sidecars/wda/WebDriverAgent/WebDriverAgent.xcodeproj → "
            "chọn Signing & Capabilities → Team → build 1 lần."
        )
    if "requires a development team" in blob.lower():
        tips.append("Thiếu Development Team — đăng nhập Apple ID trong Xcode Accounts.")
    if not _codesign_identity_ok():
        tips.append(
            "Certificate Apple Development có trong Keychain nhưng KHÔNG hợp lệ để ký "
            "(thường thiếu private key).\n"
            "Xóa cert cũ trong Keychain Access / Xcode Manage Certificates, "
            "rồi tạo lại + Apple Development trên Mac này."
        )
    if tips:
        return "\n\n".join(tips)
    # fallback: last error lines only
    err_lines = [
        ln.strip()
        for ln in blob.splitlines()
        if "error:" in ln.lower() or "** TEST BUILD FAILED" in ln
    ]
    return "\n".join(err_lines[-8:]) or blob[-2000:]


def build_wda(udid: str, team_id: str) -> Path:
    DERIVED.mkdir(parents=True, exist_ok=True)
    sync_app_icons()
    brand_info_plists()
    if not _codesign_identity_ok():
        raise RuntimeError(
            "Certificate Apple Development có nhưng không hợp lệ để codesign "
            "(thiếu private key).\n"
            "Xcode → Settings → Accounts → chọn Apple ID → Manage Certificates → "
            "xóa cert cũ nếu có → + Apple Development.\n"
            "Cert phải được tạo trên Mac này (private key nằm local)."
        )
    cmd = [
        "xcodebuild",
        "build-for-testing",
        "-allowProvisioningUpdates",
        "-allowProvisioningDeviceRegistration",
        "-project",
        "WebDriverAgent.xcodeproj",
        "-scheme",
        "WebDriverAgentRunner",
        "-destination",
        f"id={udid}",
        "-derivedDataPath",
        str(DERIVED),
        "CODE_SIGN_STYLE=Automatic",
        f"DEVELOPMENT_TEAM={team_id}",
        f"PRODUCT_BUNDLE_IDENTIFIER={BUNDLE_ID}",
        "COMPILER_INDEX_STORE_ENABLE=NO",
        "OTHER_CFLAGS=-Wno-error=poison-system-directories",
        # Runner Info.plist is generated by Xcode — inject display name at build time
        f"INFOPLIST_KEY_CFBundleDisplayName={DISPLAY_NAME}",
    ]
    result = run(cmd, cwd=WDA_SRC, check=False)
    if result.returncode != 0:
        raise RuntimeError(_friendly_xcode_error(result.stdout, result.stderr, team_id))

    # Prefer runner .app
    candidates = list(PRODUCT_DIR.glob("*Runner*.app")) + list(PRODUCT_DIR.glob("*.app"))
    if not candidates:
        candidates = list(DERIVED.rglob("WebDriverAgentRunner-Runner.app"))
    if not candidates:
        raise RuntimeError(f"Build succeeded but .app not found under {PRODUCT_DIR}")
    app = candidates[0]
    _brand_and_resign(app)
    return app


def _signing_identity() -> str:
    out = subprocess.run(
        ["security", "find-identity", "-v", "-p", "codesigning"],
        capture_output=True,
        text=True,
    ).stdout
    match = re.search(r'"(Apple Development: [^"]+)"', out or "")
    if not match:
        raise RuntimeError("Không tìm thấy Apple Development identity hợp lệ để ký lại.")
    return match.group(1)


def _embed_xcode26_testing_libs(app: Path) -> None:
    """Xcode 26 XCTRunner needs lib_TestingInterop at runtime; iOS 16 does not ship it."""
    fw = app / "Frameworks"
    if not fw.is_dir():
        return
    sdk_lib = Path(
        "/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform"
        "/Developer/usr/lib/lib_TestingInterop.dylib"
    )
    if sdk_lib.exists():
        shutil.copy2(sdk_lib, fw / "lib_TestingInterop.dylib")
    foundation_src = Path(
        "/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform"
        "/Developer/Library/Frameworks/_Testing_Foundation.framework"
    )
    if foundation_src.exists():
        dest = fw / "_Testing_Foundation.framework"
        if dest.exists():
            shutil.rmtree(dest)
        shutil.copytree(foundation_src, dest)


def _brand_and_resign(app: Path) -> None:
    """Embed icons into XCTRunner host, set display name, bump version, re-sign."""
    _embed_xcode26_testing_libs(app)

    # Apple's XCTRunner host does not inherit icons from PlugIns/*.xctest —
    # without this lift SpringBoard shows a blank icon (Appium WDA known issue).
    xctest = app / "PlugIns" / "WebDriverAgentRunner.xctest"
    if xctest.is_dir():
        for png in xctest.glob("AppIcon*.png"):
            shutil.copy2(png, app / png.name)
        assets = xctest / "Assets.car"
        if assets.exists():
            shutil.copy2(assets, app / "Assets.car")

    # Prefer freshly generated icons from asset catalog / sidecar set
    for src in [
        WDA_SRC
        / "WebDriverAgentRunner"
        / "Assets.xcassets"
        / "AppIcon.appiconset"
        / "AppIcon-60x60@2x.png",
        ICONSET / "AppIcon-60x60@2x.png",
    ]:
        if src.exists():
            shutil.copy2(src, app / "AppIcon60x60@2x.png")
            break
    for src in [
        WDA_SRC
        / "WebDriverAgentRunner"
        / "Assets.xcassets"
        / "AppIcon.appiconset"
        / "AppIcon-60x60@3x.png",
        ICONSET / "AppIcon-60x60@3x.png",
    ]:
        if src.exists():
            shutil.copy2(src, app / "AppIcon60x60@3x.png")
            break
    for src in [
        WDA_SRC
        / "WebDriverAgentRunner"
        / "Assets.xcassets"
        / "AppIcon.appiconset"
        / "AppIcon-76x76@2x.png",
        ICONSET / "AppIcon-76x76@2x.png",
    ]:
        if src.exists():
            shutil.copy2(src, app / "AppIcon76x76@2x.png")
            break

    info_path = app / "Info.plist"
    with info_path.open("rb") as f:
        info = plistlib.load(f)

    info["CFBundleDisplayName"] = DISPLAY_NAME
    info["CFBundleName"] = DISPLAY_NAME
    # Bump build so SpringBoard drops cached blank icon
    try:
        build = int(str(info.get("CFBundleVersion") or "1"))
    except ValueError:
        build = 1
    info["CFBundleVersion"] = str(build + 1)
    info["CFBundleShortVersionString"] = info.get("CFBundleShortVersionString") or "1.0"

    info["CFBundleIcons"] = {
        "CFBundlePrimaryIcon": {
            "CFBundleIconName": "AppIcon",
            "CFBundleIconFiles": ["AppIcon60x60"],
        }
    }
    info["CFBundleIcons~ipad"] = {
        "CFBundlePrimaryIcon": {
            "CFBundleIconName": "AppIcon",
            "CFBundleIconFiles": ["AppIcon60x60", "AppIcon76x76"],
        }
    }

    with info_path.open("wb") as f:
        plistlib.dump(info, f)

    identity = _signing_identity()

    def resign(path: Path) -> None:
        result = subprocess.run(
            [
                "codesign",
                "-f",
                "-s",
                identity,
                "--preserve-metadata=entitlements,flags",
                str(path),
            ],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise RuntimeError(f"codesign failed for {path.name}: {result.stderr[-800:]}")

    frameworks = app / "Frameworks"
    if frameworks.is_dir():
        for item in sorted(frameworks.iterdir()):
            if item.suffix in {".framework", ".dylib"}:
                resign(item)
    for plugin in sorted((app / "PlugIns").glob("*.xctest")) if (app / "PlugIns").exists() else []:
        resign(plugin)
    resign(app)


def package_ipa(app: Path) -> Path:
    """Zip signed .app into IPA without renaming (rename breaks code signature)."""
    stage = BUILD_ROOT / "build_payload"
    if stage.exists():
        shutil.rmtree(stage)
    payload = stage / "Payload"
    payload.mkdir(parents=True)
    shutil.copytree(app, payload / app.name)
    ipa = BUILD_ROOT / "Riviumanagersphone.ipa"
    if ipa.exists():
        ipa.unlink()
    zip_path = stage / "Riviumanagersphone.zip"
    shutil.make_archive(str(zip_path.with_suffix("")), "zip", stage, "Payload")
    zipped = stage / "Riviumanagersphone.zip"
    zipped.rename(ipa)
    return ipa


async def install_app(udid: str, app: Path) -> None:
    """Install signed .app (Developer) via pymobiledevice3, fallback to devicectl."""
    from pymobiledevice3.lockdown import create_using_usbmux
    from pymobiledevice3.services.installation_proxy import InstallationProxyService

    try:
        lockdown = await create_using_usbmux(serial=udid)
        try:
            async with InstallationProxyService(lockdown=lockdown) as proxy:
                await proxy.install_from_local(str(app), developer=True)
            return
        finally:
            await lockdown.close()
    except Exception as pmd_err:
        # Fallback: Apple's devicectl (Xcode 15+)
        cmd = [
            "xcrun",
            "devicectl",
            "device",
            "install",
            "app",
            "--device",
            udid,
            str(app),
        ]
        result = subprocess.run(cmd, capture_output=True, text=True)
        if result.returncode != 0:
            raise RuntimeError(
                f"Install failed.\npymobiledevice3: {pmd_err}\n"
                f"devicectl: {result.stdout[-1500:]}\n{result.stderr[-1500:]}"
            ) from pmd_err


async def launch_wda(udid: str) -> None:
    """Best-effort launch via xcodebuild test-without-building if available."""
    # xcodebuild test-without-building keeps WDA running
    cmd = [
        "xcodebuild",
        "test-without-building",
        "-project",
        "WebDriverAgent.xcodeproj",
        "-scheme",
        "WebDriverAgentRunner",
        "-destination",
        f"id={udid}",
        "-derivedDataPath",
        str(DERIVED),
    ]
    # Don't wait forever — spawn detached
    subprocess.Popen(
        cmd,
        cwd=str(WDA_SRC),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--udid")
    parser.add_argument("--team-id", default=None)
    parser.add_argument("--skip-launch", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    try:
        if args.self_test:
            lock = verify_resource_bundle()
            emit(
                {
                    "ok": True,
                    "kind": "packagedSigningResources",
                    "sourceVersion": lock["version"],
                    "sourceTreeSha256": lock["treeSha256"],
                    "logoSha256": lock["logoSha256"],
                    "iconSetTreeSha256": lock["iconSetTreeSha256"],
                    "executablePathCount": len(lock["executablePaths"]),
                    "workspaceOutsideResources": True,
                }
            )
            return 0
        if not args.udid:
            raise RuntimeError("--udid is required unless --self-test is used")
        configure_build_workspace(args.udid)
        require_xcode()
        ensure_wda_checkout()
        team = detect_team_id(args.team_id)
        app = build_wda(args.udid, team)
        ipa = package_ipa(app)  # keep artifact for UI / re-sign flows
        asyncio.run(install_app(args.udid, app))
        if not args.skip_launch:
            try:
                asyncio.run(launch_wda(args.udid))
            except Exception:
                pass
        # Read actual bundle id from signed app
        info_path = app / "Info.plist"
        bundle_id = BUNDLE_ID
        try:
            with info_path.open("rb") as f:
                bundle_id = str(plistlib.load(f).get("CFBundleIdentifier") or BUNDLE_ID)
        except Exception:
            pass
        emit(
            {
                "ok": True,
                "udid": args.udid,
                "displayName": DISPLAY_NAME,
                "bundleId": bundle_id,
                "ipa": str(ipa),
                "app": str(app),
                "teamId": team,
                "message": f"Đã cài {DISPLAY_NAME} lên thiết bị {args.udid}",
            }
        )
        return 0
    except Exception as exc:
        emit({"ok": False, "error": str(exc)})
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
