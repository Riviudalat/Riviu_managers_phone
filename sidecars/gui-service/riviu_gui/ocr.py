"""Local OCR runs in a killable child, so cancellation also stops native inference."""

import asyncio
import hashlib
import json
import os
import pathlib
import sys
import time

from .ocr_models import OcrLine, OcrRequest, OcrResponse
from .template_models import PixelRect

MANIFEST_PATH = pathlib.Path(__file__).with_name("ocr-models.json")


def models_directory() -> pathlib.Path:
    if override := os.environ.get("RIVIU_OCR_MODEL_DIR"):
        return pathlib.Path(override)
    if getattr(sys, "frozen", False):
        return pathlib.Path(sys.executable).parent / "ocr-models"
    return pathlib.Path(__file__).resolve().parents[3] / "target/ocr-models"


def verified_models() -> tuple[dict, dict[str, pathlib.Path]]:
    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf8"))
    folder = models_directory()
    models = {}
    for entry in manifest["files"]:
        path = folder / entry["path"]
        if not path.is_file():
            raise FileNotFoundError("gui_ocr_models_missing")
        with path.open("rb") as stream:
            if hashlib.file_digest(stream, "sha256").hexdigest() != entry["sha256"]:
                raise ValueError("gui_ocr_model_hash")
        models[entry["role"]] = path
    return manifest, models


def ocr_ready() -> bool:
    try:
        verified_models()
        return True
    except (OSError, ValueError):
        return False


async def run_ocr(request: OcrRequest) -> OcrResponse:
    # The child re-verifies hashes before importing the inference engine.
    await asyncio.to_thread(verified_models)
    from .template_match import decode_image

    await asyncio.to_thread(decode_image, request.screenshot, 8 * 1024 * 1024)
    command = [sys.executable]
    if not getattr(sys, "frozen", False):
        command.append(str(pathlib.Path(__file__).resolve().parents[1] / "serve.py"))
    command.append("--ocr-worker")
    kwargs = {"creationflags": 0x08000000} if sys.platform == "win32" else {}
    worker_env = {**os.environ, "OMP_THREAD_LIMIT": "1"}
    child = await asyncio.create_subprocess_exec(
        *command, stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.DEVNULL, limit=256 * 1024, env=worker_env, **kwargs,
    )
    try:
        async with asyncio.timeout(request.remaining_ms / 1000):
            stdout, _ = await child.communicate(request.model_dump_json(by_alias=True).encode("utf8"))
        if child.returncode != 0:
            raise RuntimeError("gui_ocr_inference_failed")
        if len(stdout) > 128 * 1024:
            raise ValueError("gui_ocr_response_budget")
        return OcrResponse.model_validate_json(stdout)
    finally:
        if child.returncode is None:
            child.kill()
        await child.wait()


def recognize(request: OcrRequest) -> OcrResponse:
    started = time.monotonic()
    _, models = verified_models()
    import tesserocr
    from PIL import Image

    from .template_match import decode_image

    image = decode_image(request.screenshot, 8 * 1024 * 1024)
    region = request.search_region
    image = Image.fromarray(image[region.y:region.y + region.height, region.x:region.x + region.width])
    language = "+".join("vie" if name == "vi" else "eng" for name in request.languages)
    lines: list[OcrLine] = []
    text_size = 0
    # Native Tesseract runs inside the disposable worker; no files, URLs, GPU,
    # language guessing, or provider correction are involved in recognition.
    with tesserocr.PyTessBaseAPI(path=str(models["vie"].parent), lang=language,
                               oem=tesserocr.OEM.LSTM_ONLY, psm=tesserocr.PSM.SPARSE_TEXT) as engine:
        engine.SetVariable("user_defined_dpi", "150")
        engine.SetImage(image)
        if not engine.Recognize(timeout=request.remaining_ms):
            raise TimeoutError("gui_deadline")
        iterator = engine.GetIterator()
        if iterator and not iterator.Empty(tesserocr.RIL.TEXTLINE):
            for line in tesserocr.iterate_level(iterator, tesserocr.RIL.TEXTLINE):
                text = (line.GetUTF8Text(tesserocr.RIL.TEXTLINE) or "").strip()
                score = line.Confidence(tesserocr.RIL.TEXTLINE) / 100
                if not text or score < request.min_confidence:
                    continue
                if len(lines) >= 128 or len(text.encode("utf8")) > 1024:
                    raise ValueError("gui_ocr_response_budget")
                text_size += len(text.encode("utf8")) + bool(lines)
                if text_size > 4096:
                    raise ValueError("gui_ocr_response_budget")
                left, top, right, bottom = line.BoundingBox(tesserocr.RIL.TEXTLINE)
                lines.append(OcrLine(text=text, confidence=min(1.0, max(0.0, float(score))), bounds=PixelRect(
                    x=region.x + left, y=region.y + top, width=right-left, height=bottom-top,
                )))
    return OcrResponse(
        request_id=request.request_id, observation_id=request.observation_id,
        session_epoch=request.session_epoch, generation=request.generation,
        screenshot_sha256=request.screenshot.sha256,
        status="resolved" if lines else "unresolved", text="\n".join(line.text for line in lines),
        lines=lines, engine=f"{tesserocr.tesseract_version().splitlines()[0]} / tesserocr 2.10.0 / tessdata_fast CPU",
        elapsed_ms=int((time.monotonic() - started) * 1000),
    )


def worker_main() -> None:
    request = OcrRequest.model_validate_json(sys.stdin.buffer.read(16 * 1024 * 1024 + 1))
    result = recognize(request)
    sys.stdout.buffer.write(result.model_dump_json(by_alias=True).encode("utf8"))
    sys.stdout.buffer.flush()
