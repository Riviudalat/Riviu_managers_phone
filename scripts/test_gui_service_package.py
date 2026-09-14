"""Verify the staged Python service without using a development interpreter at runtime."""
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import struct
import time
import urllib.request
import zlib


def template_fixture():
    """PNG fixture made with the standard library, independent of dev packages."""
    def encode(rows):
        width, height = len(rows[0]), len(rows)

        def chunk(kind, data):
            return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

        data = (
            b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(b"".join(b"\x00" + bytes(row) for row in rows)))
            + chunk(b"IEND", b"")
        )
        return {"bytesBase64": base64.b64encode(data).decode(), "sha256": hashlib.sha256(data).hexdigest(),
                "width": width, "height": height}

    template = [[(x * x * 17 + y * y * 31 + x * y * 13) % 256 for x in range(23)] for y in range(19)]
    screenshot = [[0] * 80 for _ in range(100)]
    for y, row in enumerate(template):
        screenshot[y + 39][27:50] = row
    return {"protocolVersion": 1, "requestId": "package-template-request", "observationId": "package-observation",
            "sessionEpoch": "package-epoch", "generation": 1, "remainingMs": 10000,
            "screenshot": encode(screenshot), "template": encode(template)}


def verify(runtime: Path):
    exe = runtime / ("riviu-gui-service.exe" if os.name == "nt" else "riviu-gui-service")
    token = "package-check-" + "f" * 48
    env = {k: v for k, v in os.environ.items() if not k.upper().startswith(("PYTHON", "RIVIU", "VIRTUAL_ENV"))}
    env["PATH"] = os.environ.get("SystemRoot", "C:\\Windows") + "\\System32" if os.name == "nt" else "/usr/bin:/bin"
    child = subprocess.Popen([str(exe.resolve())], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True, encoding="utf8",
                             creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    try:
        child.stdin.write(json.dumps({"token": token, "provider": {}}) + "\n")
        child.stdin.flush()
        import concurrent.futures
        with concurrent.futures.ThreadPoolExecutor(1) as reader:
            future = reader.submit(child.stdout.readline)
            try:
                hello = json.loads(future.result(timeout=15))
            except BaseException:
                child.terminate()
                raise
        assert hello["protocolVersion"] == 1
        url = f"http://127.0.0.1:{hello['port']}/health/ready"
        for _ in range(50):
            try:
                request = urllib.request.Request(url, headers={"Authorization": "Bearer " + token})
                with urllib.request.urlopen(request, timeout=1) as response:
                    ready = json.load(response)
                break
            except OSError:
                time.sleep(0.1)
        else:
            raise AssertionError("packaged service readiness timeout")
        assert ready["ready"] and not ready["providerReady"] and ready["protocolVersion"] == 1
        assert "localTemplateMatch" in ready["capabilities"]
        assert {"localOcr", "ocr.vi", "ocr.en"}.issubset(ready["capabilities"])
        template_request = urllib.request.Request(
            f"http://127.0.0.1:{hello['port']}/v1/gui/template-match",
            data=json.dumps(template_fixture()).encode(),
            headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"},
        )
        with urllib.request.urlopen(template_request, timeout=15) as response:
            matched = json.load(response)
        assert matched["status"] == "resolved" and matched["requestId"] == "package-template-request"
        assert matched["observationId"] == "package-observation" and matched["sessionEpoch"] == "package-epoch"
        assert matched["candidates"][0]["bounds"] == {"x": 27, "y": 39, "width": 23, "height": 19}
        # Real multilingual OCR on a clean PATH verifies the native library and
        # packaged traineddata, including Vietnamese letters absent in Latin-only models.
        fixture = Path(__file__).resolve().parents[1] / "sidecars/gui-service/tests/fixtures/ocr-vietnamese-english.png"
        png = fixture.read_bytes()
        ocr_body = {"protocolVersion": 1, "requestId": "package-ocr-request", "observationId": "package-ocr-image",
                    "sessionEpoch": "package-epoch", "generation": 1, "remainingMs": 30000,
                    "screenshot": {"bytesBase64": base64.b64encode(png).decode(), "sha256": hashlib.sha256(png).hexdigest(),
                                   "width": 900, "height": 280}, "languages": ["vi", "en"], "minConfidence": 0.7}
        ocr_request = urllib.request.Request(
            f"http://127.0.0.1:{hello['port']}/v1/gui/ocr", data=json.dumps(ocr_body).encode(),
            headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"},
        )
        with urllib.request.urlopen(ocr_request, timeout=35) as response:
            recognized = json.load(response)
        assert recognized["text"] == "Xin chào Việt Nam\nRiviu Manager - local OCR", recognized
        assert recognized["screenshotSha256"] == ocr_body["screenshot"]["sha256"]
        assert recognized["requestId"] == "package-ocr-request" and recognized["generation"] == 1
        assert len(recognized["lines"]) == 2 and all(line["confidence"] >= 0.7 for line in recognized["lines"])
        child.stdin.close()
        child.wait(timeout=10)
        assert child.returncode == 0
        return {"ready": True, "protocolVersion": 1, "localTemplateMatch": True, "localOcr": True,
                "ocrVietnameseEnglish": recognized["text"], "ocrElapsedMs": recognized["elapsedMs"], "parentExitStopsService": True}
    finally:
        if child.poll() is None:
            child.terminate()
            child.wait(timeout=5)


if __name__ == "__main__":
    import sys
    print(json.dumps(verify(Path(sys.argv[1]))))
