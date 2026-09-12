import asyncio
import secrets
from contextlib import asynccontextmanager
from typing import Annotated

import httpx
from fastapi import APIRouter, Depends, FastAPI, Header, HTTPException
from starlette.responses import JSONResponse

from . import VERSION
from .models import GuiRequest, GuiResponse, Readiness
from .provider import ProviderConfig, VisionProvider


def create_app(token: str, config: ProviderConfig | None = None, transport=None) -> FastAPI:
    if len(token) < 32:
        raise ValueError("service token must contain at least 32 characters")
    config = config or ProviderConfig()
    active = 0

    @asynccontextmanager
    async def lifespan(app: FastAPI):
        async with httpx.AsyncClient(transport=transport, follow_redirects=False) as client:
            app.state.provider = VisionProvider(config, client)
            yield

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
            capabilities=["resolve", "recover", "imageValidation"],
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

    app.include_router(health)
    app.include_router(router)
    return app
