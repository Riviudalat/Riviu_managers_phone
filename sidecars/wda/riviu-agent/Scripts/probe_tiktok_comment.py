#!/usr/bin/env python3
"""Run one real TikTok comment and publish frame-backed evidence.

This probe deliberately has no default comment. The operator supplies a
sentence that matches the video being viewed and confirms the sent comment in
the captured frame before the evidence is accepted.
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any

try:
    from PIL import Image
except ImportError as exc:  # pragma: no cover - dependency is pinned for live probes
    raise SystemExit("Pillow is required; install requirements-mac.txt") from exc


TOKEN_ENV = "RIVIU_AGENT_TOKEN"
TARGET_BUNDLE = "com.ss.iphone.ugc.Ame"
LIVE_ENVIRONMENT = "LIVE_MAC_DEVICE"
PLACEHOLDER_WORDS = ("riviu test", "fixture", "placeholder", "sample comment")
LOGICAL_SIZE = (375.0, 667.0)


class ProbeError(RuntimeError):
    pass


@dataclass(frozen=True)
class ProbeConfig:
    udid: str
    control_url: str
    mjpeg_port: int
    comment_text: str
    comment_point: tuple[float, float]
    composer_point: tuple[float, float]
    send_point: tuple[float, float]
    output: Path
    frames_dir: Path
    sidecar: Path
    operator_confirmed_comment_visible: bool


class ControlClient:
    def __init__(self, base_url: str, token: str) -> None:
        self.base_url = base_url.rstrip("/")
        self.token = token
        self.session_id: str | None = None

    def _remember_session(self, payload: Any) -> None:
        if not isinstance(payload, dict):
            return
        session_id = payload.get("sessionId")
        if not session_id and isinstance(payload.get("value"), dict):
            session_id = payload["value"].get("sessionId")
        if isinstance(session_id, str) and session_id:
            self.session_id = session_id

    def request(self, method: str, path: str, body: dict[str, Any] | None = None) -> Any:
        raw = None
        headers = {"X-Riviu-Token": self.token}
        if body is not None:
            raw = json.dumps(body, ensure_ascii=False, separators=(",", ":")).encode()
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(
            f"{self.base_url}{path}", data=raw, headers=headers, method=method
        )
        try:
            with urllib.request.urlopen(request, timeout=20) as response:
                status = response.status
                payload = json.loads(response.read().decode("utf-8"))
        except (OSError, urllib.error.HTTPError, json.JSONDecodeError) as exc:
            raise ProbeError(f"HTTP {method} {path} failed") from exc
        if status != 200:
            raise ProbeError(f"HTTP {method} {path} returned {status}")
        self._remember_session(payload)
        return payload

    def tap(self, x: float, y: float) -> None:
        self.request("POST", "/wda/tap", {"x": x, "y": y})

    def fresh_session(self) -> str:
        payload = self.request(
            "POST",
            "/session",
            {"capabilities": {"alwaysMatch": {}, "firstMatch": [{}]}},
        )
        session_id = payload.get("sessionId")
        if not session_id and isinstance(payload.get("value"), dict):
            session_id = payload["value"].get("sessionId")
        if not isinstance(session_id, str) or not session_id:
            raise ProbeError("fresh session response omitted sessionId")
        self.session_id = session_id
        return session_id

    def type_text(self, session_id: str, text: str) -> None:
        self.request(
            "POST",
            f"/session/{session_id}/wda/keys",
            {"value": [text]},
        )


def _validate_comment_text(value: str) -> str:
    text = " ".join(value.split())
    if len(text.encode("utf-8")) < 4:
        raise ProbeError("comment text must contain at least four UTF-8 bytes")
    lowered = text.casefold()
    if any(marker in lowered for marker in PLACEHOLDER_WORDS):
        raise ProbeError("comment text must describe the real video, not a probe placeholder")
    return text


def _parse_point(value: str) -> tuple[float, float]:
    try:
        x_text, y_text = value.split(",", 1)
        x, y = float(x_text), float(y_text)
    except (ValueError, TypeError) as exc:
        raise argparse.ArgumentTypeError("point must be x,y") from exc
    if not (0 <= x <= LOGICAL_SIZE[0] and 0 <= y <= LOGICAL_SIZE[1]):
        raise argparse.ArgumentTypeError("point must be inside the 375x667 logical surface")
    return x, y


def _capture_frame(config: ProbeConfig, name: str, token: str) -> Path:
    config.frames_dir.mkdir(parents=True, exist_ok=True)
    raw_path = config.frames_dir / f".{name}.framed"
    output_path = config.frames_dir / f"{name}.jpg"
    environment = os.environ.copy()
    environment[TOKEN_ENV] = token
    command = [
        sys.executable,
        str(config.sidecar),
        "stream",
        "--udid",
        config.udid,
        "--mode",
        "mjpeg",
        "--max-frames",
        "1",
        "--mjpeg-port",
        str(config.mjpeg_port),
    ]
    try:
        completed = subprocess.run(
            command,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=30,
            check=True,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise ProbeError(f"MJPEG capture failed for {name}") from exc
    if len(completed.stdout) < 4:
        raise ProbeError(f"MJPEG capture returned no frame for {name}")
    size = struct.unpack(">I", completed.stdout[:4])[0]
    frame = completed.stdout[4 : 4 + size]
    if len(frame) != size or not frame.startswith(b"\xff\xd8"):
        raise ProbeError(f"MJPEG capture returned a truncated JPEG for {name}")
    raw_path.write_bytes(completed.stdout)
    output_path.write_bytes(frame)
    return output_path


def _send_button_redness(path: Path) -> float:
    with Image.open(path) as image:
        rgb = image.convert("RGB")
        scale_x = rgb.width / LOGICAL_SIZE[0]
        scale_y = rgb.height / LOGICAL_SIZE[1]
        left = int(300 * scale_x)
        top = int(390 * scale_y)
        right = int(375 * scale_x)
        bottom = int(465 * scale_y)
        crop = rgb.crop((left, top, right, bottom))
        pixels = list(crop.getdata())
    if not pixels:
        return 0.0
    red = sum(1 for r, g, b in pixels if r >= 180 and r - g >= 70 and r - b >= 35)
    return red / len(pixels)


def run(config: ProbeConfig) -> dict[str, Any]:
    token = os.environ.get(TOKEN_ENV, "").strip()
    if len(token.encode("utf-8")) < 32:
        raise ProbeError(f"{TOKEN_ENV} must contain at least 256 bits")
    config.output.parent.mkdir(parents=True, exist_ok=True)

    launch = subprocess.run(
        [sys.executable, str(config.sidecar), "launch", "--udid", config.udid, "--bundle-id", TARGET_BUNDLE],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=30,
        check=True,
        text=True,
    )
    if '"ok": true' not in launch.stdout.lower():
        raise ProbeError("TikTok DVT launch did not return ok=true")
    time.sleep(2.0)

    client = ControlClient(config.control_url, token)
    session_id = client.fresh_session()
    before = _capture_frame(config, "before", token)
    client.tap(*config.comment_point)
    time.sleep(3.0)
    drawer = _capture_frame(config, "drawer", token)
    client.tap(*config.composer_point)
    time.sleep(0.8)
    # Focusing TikTok's composer can rotate the session in the native agent;
    # use the envelope returned by that tap for the text request.
    session_id = client.session_id or session_id
    client.type_text(session_id, config.comment_text)
    armed = _capture_frame(config, "armed", token)
    armed_redness = _send_button_redness(armed)
    if armed_redness < 0.02:
        raise ProbeError("Send button was not visibly armed after the real comment text")

    client.tap(*config.send_point)
    time.sleep(2.0)
    sent = _capture_frame(config, "sent", token)
    sent_redness = _send_button_redness(sent)
    composer_cleared = sent_redness < armed_redness * 0.6
    if not composer_cleared:
        raise ProbeError("composer did not visibly clear after tapping Send")
    if not config.operator_confirmed_comment_visible:
        raise ProbeError(
            "inspect sent.jpg and rerun with --operator-confirmed-comment-visible"
        )

    evidence = {
        "schemaVersion": 1,
        "environment": LIVE_ENVIRONMENT,
        "gateStatus": "PASS",
        "targetBundle": TARGET_BUNDLE,
        "agentControlUrl": config.control_url,
        "sessionCreatedFresh": True,
        "commentText": config.comment_text,
        "frames": {
            "before": str(before),
            "drawer": str(drawer),
            "armed": str(armed),
            "sent": str(sent),
        },
        "composerArmed": True,
        "composerClearedAfterSend": True,
        "armedSendButtonRedness": round(armed_redness, 4),
        "sentSendButtonRedness": round(sent_redness, 4),
        "operatorConfirmedCommentVisible": True,
    }
    temporary = config.output.with_suffix(config.output.suffix + ".tmp")
    temporary.write_text(json.dumps(evidence, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, config.output)
    return evidence


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--udid", required=True)
    parser.add_argument("--control-url", default="http://127.0.0.1:18100")
    parser.add_argument("--mjpeg-port", type=int, default=9094)
    parser.add_argument("--comment-text", required=True)
    parser.add_argument("--comment-point", type=_parse_point, default=(343.0, 377.0))
    parser.add_argument("--composer-point", type=_parse_point, default=(120.0, 640.0))
    parser.add_argument("--send-point", type=_parse_point, default=(337.0, 427.0))
    parser.add_argument(
        "--frames-dir",
        type=Path,
        default=Path("docs/re/riviu-agent/tiktok-comment-live"),
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("docs/re/riviu-agent/tiktok-comment-live.json"),
    )
    parser.add_argument(
        "--sidecar",
        type=Path,
        default=Path("sidecars/pymobiledevice3/riviu_pmd.py"),
    )
    parser.add_argument("--operator-confirmed-comment-visible", action="store_true")
    return parser


def main() -> int:
    args = _parser().parse_args()
    try:
        config = ProbeConfig(
            udid=args.udid,
            control_url=args.control_url,
            mjpeg_port=args.mjpeg_port,
            comment_text=_validate_comment_text(args.comment_text),
            comment_point=args.comment_point,
            composer_point=args.composer_point,
            send_point=args.send_point,
            output=args.output,
            frames_dir=args.frames_dir,
            sidecar=args.sidecar,
            operator_confirmed_comment_visible=args.operator_confirmed_comment_visible,
        )
        evidence = run(config)
    except ProbeError as exc:
        print(json.dumps({"ok": False, "error": str(exc)}, ensure_ascii=True))
        return 1
    print(
        json.dumps(
            {
                "ok": True,
                "gateStatus": evidence["gateStatus"],
                "evidence": str(args.output),
                "frames": str(args.frames_dir),
            },
            ensure_ascii=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
