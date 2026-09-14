import asyncio
import secrets
import threading
import time
from contextlib import asynccontextmanager
from typing import Annotated

import httpx
from fastapi import APIRouter, Depends, FastAPI, Header, HTTPException
from fastapi import Request as HttpRequest
from starlette.responses import JSONResponse

from . import VERSION
from .models import GuiRequest, GuiResponse, Readiness
from .ocr import ocr_ready, run_ocr
from .ocr_models import OcrRequest, OcrResponse
from .provider import ProviderConfig, VisionProvider
from .template_match import match_template
from .template_models import TemplateMatchRequest, TemplateMatchResponse


def create_app(token: str, config: ProviderConfig | None = None, transport=None) -> FastAPI:
    if len(token) < 32:
        raise ValueError("service token must contain at least 32 characters")
    config = config or ProviderConfig()
    active = 0
    local_tasks: set[asyncio.Task] = set()

    @asynccontextmanager
    async def lifespan(app: FastAPI):
        async with httpx.AsyncClient(transport=transport, follow_redirects=False) as client:
            app.state.provider = VisionProvider(config, client)
            try:
                yield
            finally:
                if local_tasks:
                    await asyncio.gather(*local_tasks, return_exceptions=True)

    app = FastAPI(
        title="Riviu GUI Service", version=VERSION, lifespan=lifespan, docs_url=None, redoc_url=None
    )

    async def authorize(authorization: Annotated[str, Header()] = "") -> None:
        if not secrets.compare_digest(authorization, "Bearer " + token):
            raise HTTPException(401, detail="gui_unauthorized")

    @app.middleware("http")
    async def limits(request, call_next):
        if not secrets.compare_digest(request.headers.get("authorization", ""), "Bearer " + token):
            return JSONResponse({"detail": "gui_unauthorized"}, status_code=401)
        if request.method == "POST":
            length = request.headers.get("content-length", "")
            if not length.isdigit() or int(length) > 20 * 1024 * 1024:
                return JSONResponse({"detail": "gui_request_too_large"}, status_code=413)
        return await call_next(request)

    health = APIRouter(prefix="/health", dependencies=[Depends(authorize)])

    @health.get("/live")
    def live() -> dict[str, bool]:
        return {"alive": True}

    @health.get("/ready")
    def ready() -> Readiness:
        return Readiness(
            service_version=VERSION,
            ready=True,
            provider_ready=config.ready,
            capabilities=["resolve", "recover", "imageValidation", "localTemplateMatch"]
            + (["localOcr", "ocr.vi", "ocr.en"] if ocr_ready() else ["ocrModelsMissing"]),
            active_requests=active,
        )

    router = APIRouter(prefix="/v1/gui", dependencies=[Depends(authorize)])

    async def perceive(request: GuiRequest) -> GuiResponse:
        nonlocal active
        if active >= 2:
            raise HTTPException(429, detail="gui_capacity", headers={"Retry-After": "1"})
        active += 1
        try:
            async with asyncio.timeout(min(30.0, request.remaining_ms / 1000)):
                return await app.state.provider.resolve(request)
        except TimeoutError as error:
            raise HTTPException(504, detail="gui_deadline") from error
        except ValueError as error:
            raise HTTPException(422, detail="gui_input_invalid") from error
        except httpx.HTTPError as error:
            raise HTTPException(502, detail="gui_provider_unavailable") from error
        finally:
            active -= 1

    @router.post("/resolve")
    async def resolve(request: GuiRequest) -> GuiResponse:
        return await perceive(request)

    @router.post("/recover")
    async def recover(request: GuiRequest) -> GuiResponse:
        return await perceive(request)

    @router.post("/ocr")
    async def ocr(request: OcrRequest, http_request: HttpRequest) -> OcrResponse:
        nonlocal active
        if active >= 2:
            raise HTTPException(429, detail="gui_capacity", headers={"Retry-After": "1"})
        active += 1
        task = asyncio.create_task(run_ocr(request))
        local_tasks.add(task)

        def release_ocr(completed: asyncio.Task):
            nonlocal active
            active -= 1
            local_tasks.discard(completed)
            if not completed.cancelled():
                completed.exception()

        task.add_done_callback(release_ocr)

        async def wait_for_disconnect():
            while not task.done():
                if await http_request.is_disconnected():
                    task.cancel()
                    return
                await asyncio.sleep(0.05)

        disconnect = asyncio.create_task(wait_for_disconnect())
        try:
            async with asyncio.timeout(request.remaining_ms / 1000):
                return await asyncio.shield(task)
        except TimeoutError as error:
            raise HTTPException(504, detail="gui_deadline") from error
        except FileNotFoundError as error:
            raise HTTPException(503, detail="gui_ocr_models_missing") from error
        except ValueError as error:
            raise HTTPException(422, detail="gui_ocr_input_invalid") from error
        except RuntimeError as error:
            raise HTTPException(502, detail="gui_ocr_inference_failed") from error
        finally:
            disconnect.cancel()
            if not task.done() and not task.cancelling():
                task.cancel()
            await asyncio.gather(task, disconnect, return_exceptions=True)

    @router.post("/template-match")
    async def template_match(request: TemplateMatchRequest) -> TemplateMatchResponse:
        nonlocal active
        if active >= 2:
            raise HTTPException(429, detail="gui_capacity", headers={"Retry-After": "1"})
        active += 1
        stop = threading.Event()
        duration = request.remaining_ms / 1000
        deadline = time.monotonic() + duration

        async def work() -> TemplateMatchResponse:
            nonlocal active
            try:
                return await asyncio.to_thread(match_template, request, stop, deadline)
            finally:
                # Cancellation of an HTTP caller does not stop a native call.
                # Keep its capacity slot until the thread actually completes.
                active -= 1

        task = asyncio.create_task(work())
        local_tasks.add(task)

        def finish(completed: asyncio.Task):
            local_tasks.discard(completed)
            if not completed.cancelled():
                completed.exception()  # consume a detached timeout/error

        task.add_done_callback(finish)
        try:
            return await asyncio.wait_for(asyncio.shield(task), duration)
        except TimeoutError as error:
            stop.set()
            raise HTTPException(504, detail="gui_deadline") from error
        except asyncio.CancelledError:
            stop.set()
            raise
        except ValueError as error:
            raise HTTPException(422, detail="gui_input_invalid") from error

    app.include_router(health)
    app.include_router(router)
    return app
