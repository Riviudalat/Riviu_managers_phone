"""Local OCR runs in resident, killable children: a warm worker reuses its loaded models, and
cancellation or a deadline still stops native inference by killing that worker."""

import asyncio
import collections
import dataclasses
import hashlib
import json
import os
import pathlib
import sys
import time
import uuid

from .ocr_models import OcrLine, OcrRequest, OcrResponse
from .template_models import PixelRect

MANIFEST_PATH = pathlib.Path(__file__).with_name("ocr-models.json")

# Resident workers, and the local OCR/template admission cap that matches them. Not measured:
# the farm PC has 20 logical cores and each worker runs one native thread (OMP_THREAD_LIMIT=1),
# so four leave the rest of the CPU to the phones' own work. The desktop sends its own value at
# bootstrap (RIVIU_OCR_WORKERS); both sides default to this one.
DEFAULT_WORKERS = 4
# Not measured: a ceiling on RAM and CPU however the knob is set.
MAX_WORKERS = 8
# Replacement spawns allowed per window after the first `size` workers. Not measured: a cancel
# or deadline kills its worker, which is rare while callers' budgets (8-30 s) exceed the queue,
# so this only stops a crash loop from spawning without end; past it the pool fails closed.
RESPAWNS_PER_WORKER = 4
RESPAWN_WINDOW_S = 60.0
# A replacement waits this long, doubled per retirement since the last served request.
RESPAWN_BACKOFF_S = 0.25
RESPAWN_BACKOFF_MAX_S = 4.0
# One request envelope: an 8 MiB screenshot is about 11 MiB of base64 plus the identifiers.
WORKER_LINE_LIMIT = 16 * 1024 * 1024 + 1
RESPONSE_LIMIT = 128 * 1024


class OcrWorkersUnavailable(RuntimeError):
    """The pool is closed or its respawn budget is spent."""


@dataclasses.dataclass
class OcrTiming:
    """Where one request's time went inside the sidecar, for its measurement line."""

    queue_ms: int = 0
    work_ms: int = 0
    worker: int | None = None
    spawned: bool = False


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


async def spawn_worker() -> asyncio.subprocess.Process:
    command = [sys.executable]
    if not getattr(sys, "frozen", False):
        command.append(str(pathlib.Path(__file__).resolve().parents[1] / "serve.py"))
    command.append("--ocr-worker")
    kwargs = {"creationflags": 0x08000000} if sys.platform == "win32" else {}
    # One native thread each. NumPy's OpenBLAS otherwise commits a buffer per core: a resident
    # worker held 696 MB private instead of 85 MB (1080x2220 frame, vie+eng and vie loaded,
    # measured 2026-10-15 on the 20-core farm PC).
    worker_env = {**os.environ, "OMP_THREAD_LIMIT": "1", "OPENBLAS_NUM_THREADS": "1"}
    return await asyncio.create_subprocess_exec(
        *command, stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.DEVNULL, limit=2 * RESPONSE_LIMIT, env=worker_env, **kwargs,
    )


def bound_to(response: OcrResponse, request: OcrRequest) -> bool:
    return (response.request_id, response.observation_id, response.session_epoch,
            response.generation, response.screenshot_sha256) == (
        request.request_id, request.observation_id, request.session_epoch,
        request.generation, request.screenshot.sha256)


class OcrWorkerPool:
    """At most `size` resident workers, each serving one request at a time.

    Workers start on demand and stay warm. A worker whose request fails, is cancelled or runs
    out of time is killed and reaped before the request returns, so the caller's admission
    slot stays held until the worker has drained; its replacement starts on the next demand.
    The app admits at most `size` local requests, which bounds the waiters here, and every
    wait is bounded by the caller's deadline.
    """

    def __init__(self, size: int = DEFAULT_WORKERS):
        if not 1 <= size <= MAX_WORKERS:
            raise ValueError("gui_ocr_worker_count")
        self.size = size
        self._idle: list[asyncio.subprocess.Process] = []
        self._live = 0  # spawning, idle or busy
        self._spawned = 0
        self._faults = 0  # retirements since the last served request
        self._respawns: collections.deque[float] = collections.deque()
        self._changed = asyncio.Event()
        self._closed = False

    def _respawn_delay(self) -> float:
        now = time.monotonic()
        while self._respawns and now - self._respawns[0] > RESPAWN_WINDOW_S:
            self._respawns.popleft()
        if len(self._respawns) >= RESPAWNS_PER_WORKER * self.size:
            raise OcrWorkersUnavailable("gui_ocr_worker_respawn_budget")
        self._respawns.append(now)
        return min(RESPAWN_BACKOFF_MAX_S, RESPAWN_BACKOFF_S * 2 ** max(0, self._faults - 1))

    async def _acquire(self, timing: OcrTiming) -> asyncio.subprocess.Process:
        while True:
            if self._closed:
                raise OcrWorkersUnavailable("gui_ocr_pool_closed")
            if self._idle:
                worker = self._idle.pop()
                if worker.returncode is None:
                    return worker
                await self._retire(worker)
                continue
            if self._live < self.size:
                break
            self._changed.clear()
            await self._changed.wait()
        self._live += 1
        try:
            if self._spawned >= self.size:
                await asyncio.sleep(self._respawn_delay())
            worker = await spawn_worker()
        except BaseException:
            self._live -= 1
            self._changed.set()
            raise
        self._spawned += 1
        timing.spawned = True
        return worker

    async def _retire(self, worker: asyncio.subprocess.Process) -> None:
        self._faults += 1
        try:
            if worker.returncode is None:
                worker.kill()
        except ProcessLookupError:
            pass
        try:
            await worker.wait()
        finally:
            self._live -= 1
            self._changed.set()

    async def run(self, request: OcrRequest, deadline: float, timing: OcrTiming) -> OcrResponse:
        loop = asyncio.get_running_loop()
        queued = loop.time()
        worker = await self._acquire(timing)
        started = loop.time()
        timing.queue_ms = int((started - queued) * 1000)
        timing.worker = worker.pid
        try:
            response = await exchange(worker, request, max(1, int((deadline - started) * 1000)))
        except BaseException:
            await self._retire(worker)
            raise
        finally:
            timing.work_ms = int((loop.time() - started) * 1000)
        if self._closed:
            await self._retire(worker)
        else:
            self._faults = 0
            self._idle.append(worker)
            self._changed.set()
        return response

    async def close(self) -> None:
        self._closed = True
        idle, self._idle = self._idle, []
        for worker in idle:
            await self._retire(worker)
        self._changed.set()


async def exchange(worker: asyncio.subprocess.Process, request: OcrRequest, remaining_ms: int) -> OcrResponse:
    """One line in, one line out. The ticket and the observation binding keep a stale or
    foreign reply from answering this request; any mismatch retires the worker."""
    ticket = uuid.uuid4().hex
    dispatched = request.model_copy(update={"remaining_ms": remaining_ms})
    stdin, stdout = worker.stdin, worker.stdout
    if stdin is None or stdout is None:
        raise RuntimeError("gui_ocr_inference_failed")
    try:
        stdin.write(b'{"ticket":"%s","request":%s}\n' % (
            ticket.encode(), dispatched.model_dump_json(by_alias=True).encode("utf8")))
        await stdin.drain()
        reply = await stdout.readline()
    except (BrokenPipeError, ConnectionResetError) as error:
        raise RuntimeError("gui_ocr_inference_failed") from error
    if not reply.endswith(b"\n"):
        raise RuntimeError("gui_ocr_inference_failed")
    if len(reply) > RESPONSE_LIMIT:
        raise ValueError("gui_ocr_response_budget")
    try:
        message = json.loads(reply)
        if not isinstance(message, dict) or message.get("ticket") != ticket or "response" not in message:
            raise RuntimeError("gui_ocr_inference_failed")
        response = OcrResponse.model_validate(message["response"])
    except ValueError as error:
        raise RuntimeError("gui_ocr_inference_failed") from error
    if not bound_to(response, request):
        raise RuntimeError("gui_ocr_inference_failed")
    return response


async def run_ocr(request: OcrRequest, pool: OcrWorkerPool, timing: OcrTiming | None = None) -> OcrResponse:
    deadline = asyncio.get_running_loop().time() + request.remaining_ms / 1000
    # The worker re-verifies hashes before each recognition.
    await asyncio.to_thread(verified_models)
    from .template_match import decode_image

    await asyncio.to_thread(decode_image, request.screenshot, 8 * 1024 * 1024)
    async with asyncio.timeout_at(deadline):
        return await pool.run(request, deadline, timing or OcrTiming())


class Engines:
    """Tesseract engines a worker keeps loaded, one per model folder and language set."""

    def __init__(self):
        self._engines = {}

    def get(self, folder: pathlib.Path, language: str):
        import tesserocr

        key = (str(folder), language)
        if key not in self._engines:
            self._engines[key] = tesserocr.PyTessBaseAPI(
                path=str(folder), lang=language, oem=tesserocr.OEM.LSTM_ONLY, psm=tesserocr.PSM.SPARSE_TEXT)
        return self._engines[key]

    def close(self) -> None:
        for engine in self._engines.values():
            engine.End()
        self._engines.clear()


def recognize(request: OcrRequest, engines: Engines | None = None) -> OcrResponse:
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
    owned = engines is None
    engines = Engines() if engines is None else engines
    # Native Tesseract runs inside the killable worker; no files, URLs, GPU,
    # language guessing, or provider correction are involved in recognition.
    try:
        engine = engines.get(models["vie"].parent, language)
        engine.SetVariable("user_defined_dpi", "150")
        engine.SetImage(image)
        try:
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
        finally:
            # The loaded models stay; this frame's image and results go.
            engine.Clear()
    finally:
        if owned:
            engines.close()
    return OcrResponse(
        request_id=request.request_id, observation_id=request.observation_id,
        session_epoch=request.session_epoch, generation=request.generation,
        screenshot_sha256=request.screenshot.sha256,
        status="resolved" if lines else "unresolved", text="\n".join(line.text for line in lines),
        lines=lines, engine=f"{tesserocr.tesseract_version().splitlines()[0]} / tesserocr 2.10.0 / tessdata_fast CPU",
        elapsed_ms=int((time.monotonic() - started) * 1000),
    )


def worker_main() -> None:
    """Serve the pool one request per line until it closes stdin. A failed request ends the
    worker, so no native state survives an error into the next request."""
    engines = Engines()
    source, sink = sys.stdin.buffer, sys.stdout.buffer
    try:
        while line := source.readline(WORKER_LINE_LIMIT):
            if not line.endswith(b"\n"):
                raise SystemExit(2)
            envelope = json.loads(line)
            try:
                result = recognize(OcrRequest.model_validate(envelope["request"]), engines)
                reply = {"ticket": envelope["ticket"], "response": result.model_dump(mode="json", by_alias=True)}
            except Exception:
                sink.write(json.dumps({"ticket": envelope.get("ticket"), "error": "gui_ocr_inference_failed"}).encode() + b"\n")
                sink.flush()
                raise SystemExit(1) from None
            sink.write(json.dumps(reply).encode("ascii") + b"\n")
            sink.flush()
    finally:
        engines.close()
