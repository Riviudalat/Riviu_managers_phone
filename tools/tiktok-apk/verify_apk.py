"""Verify an APK's identity before any analysis (read-only).

Reads package, versionName and versionCode with ``aapt2 dump badging`` and the
signature with ``apksigner verify --print-certs --verbose``, then hashes the file.
Fails (exit 1) when the signature does not verify, when the signer differs from the
known signer for the package, or when an ``--expect-*`` value does not match.

    py -3.12 -I verify_apk.py --sdk %LOCALAPPDATA%\\Android\\Sdk base.apk ^
        --expect-version 45.7.3 --json-out target\\tiktok-45.7.3-research\\identity.json

apksigner needs a Java runtime on PATH.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import sdk  # noqa: E402

# Signing certificate SHA-256 per package, read with apksigner from the unmodified
# TikTok Global 45.7.3 APK (versionCode 2024507030) on 2026-10-09.
KNOWN_SIGNERS = {
    "com.zhiliaoapp.musically": "9041803e91bcb814b4b4399fb5c85a91640b755e5e8ba76813814bf4cf2ab5ba",
}

BADGING = re.compile(r"^package: name='([^']*)' versionCode='([^']*)' versionName='([^']*)'", re.M)
SIGNER_SHA256 = re.compile(r"^Signer #(\d+) certificate SHA-256 digest: ([0-9a-f]{64})\s*$", re.M)
SIGNER_DN = re.compile(r"^Signer #(\d+) certificate DN: (.*?)\s*$", re.M)
SCHEME = re.compile(r"^Verified using (v[\d.]+) scheme[^:]*: (true|false)\s*$", re.M)


def parse_badging(text: str) -> dict[str, str]:
    match = BADGING.search(text)
    if not match:
        raise ValueError("aapt2 badging has no package line")
    return {"package": match.group(1), "versionCode": match.group(2), "versionName": match.group(3)}


def parse_apksigner(text: str) -> dict:
    return {
        "verifies": bool(re.search(r"^Verifies\s*$", text, re.M)),
        "schemes": {name: value == "true" for name, value in SCHEME.findall(text)},
        "signers": [
            {"sha256": sha, "dn": dict(SIGNER_DN.findall(text)).get(number, "")}
            for number, sha in SIGNER_SHA256.findall(text)
        ],
    }


def problems(identity: dict, expect: dict[str, str | None]) -> list[str]:
    found = []
    signature = identity["signature"]
    if not signature["verifies"]:
        found.append("chữ ký không hợp lệ (apksigner không báo Verifies)")
    shas = [signer["sha256"] for signer in signature["signers"]]
    if len(shas) != 1:
        found.append(f"cần đúng 1 signer, thấy {len(shas)}")
    known = KNOWN_SIGNERS.get(identity["package"])
    wanted = expect.get("signer") or known
    if wanted and shas != [wanted.lower()]:
        found.append(f"signer {shas} khác {wanted.lower()}")
    if not wanted:
        found.append(f"chưa biết signer của {identity['package']}; truyền --expect-signer-sha256 sau khi tự đối chiếu")
    for key, field in (("package", "package"), ("version", "versionName"), ("version_code", "versionCode")):
        if expect.get(key) and identity[field] != expect[key]:
            found.append(f"{field}={identity[field]!r}, mong đợi {expect[key]!r}")
    return found


def run(tool: Path, args: list[str]) -> str:
    result = subprocess.run([str(tool), *args], capture_output=True, text=True, encoding="utf-8", errors="replace", check=False, timeout=600)
    if result.returncode != 0:
        raise RuntimeError(f"{tool.name} exited {result.returncode}: {(result.stderr or result.stdout)[-600:]}")
    return result.stdout


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as src:
        while chunk := src.read(1 << 20):
            digest.update(chunk)
    return digest.hexdigest()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Verify APK package, version and signer.")
    parser.add_argument("apk", type=Path)
    parser.add_argument("--sdk", type=Path, required=True, help="Android SDK root (or one build-tools dir)")
    parser.add_argument("--expect-package")
    parser.add_argument("--expect-version", help="versionName, e.g. 45.7.3")
    parser.add_argument("--expect-version-code")
    parser.add_argument("--expect-signer-sha256", help="overrides KNOWN_SIGNERS for the package")
    parser.add_argument("--json-out", type=Path)
    args = parser.parse_args(argv)
    for stream in (sys.stdout, sys.stderr):
        getattr(stream, "reconfigure", lambda **_: None)(encoding="utf-8")

    try:
        badging = parse_badging(run(sdk.build_tool(args.sdk, "aapt2"), ["dump", "badging", str(args.apk)]))
        signature = parse_apksigner(run(sdk.build_tool(args.sdk, "apksigner"), ["verify", "--print-certs", "--verbose", str(args.apk)]))
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    identity = {
        **badging,
        "file": args.apk.name,
        "size": args.apk.stat().st_size,
        "sha256": sha256_file(args.apk),
        "signature": signature,
    }
    found = problems(identity, {
        "package": args.expect_package,
        "version": args.expect_version,
        "version_code": args.expect_version_code,
        "signer": args.expect_signer_sha256,
    })
    identity["ok"] = not found
    identity["problems"] = found
    text = json.dumps(identity, ensure_ascii=False, indent=1)
    if args.json_out:
        args.json_out.write_text(text + "\n", encoding="utf-8")
    print(text)
    return 0 if not found else 1


if __name__ == "__main__":
    sys.exit(main())
