import asyncio
import base64
import hashlib
import io
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import cv2
import httpx
import numpy as np
import pytest
from fastapi.testclient import TestClient
from PIL import Image

from riviu_gui.app import create_app
from riviu_gui.provider import ProviderConfig
from riviu_gui.template_match import match_template
from riviu_gui.template_models import TemplateMatchRequest

TOKEN = "local-template-fixture-" + "b" * 32
HEADERS = {"Authorization": "Bearer " + TOKEN}
ENDPOINT = "/v1/gui/template-match"


def test_shared_rust_python_template_contract():
    fixture = Path(__file__).parent / "fixtures/template-request.json"
    request = TemplateMatchRequest.model_validate_json(fixture.read_text(encoding="utf8"))
    result = match_template(request, threading.Event(), time.monotonic() + 5)
    assert result.status == "resolved"
    assert result.candidates[0].bounds.x == 27 and result.candidates[0].bounds.y == 39


def encoded(pixels, *, image_format="PNG"):
    image = Image.fromarray(pixels)
    buffer = io.BytesIO()
    image.save(buffer, format=image_format)
    data = buffer.getvalue()
    return {
        "bytesBase64": base64.b64encode(data).decode(),
        "sha256": hashlib.sha256(data).hexdigest(),
        "width": image.width,
        "height": image.height,
    }


def observation(*, positions=((27, 39),), scale=1.0, size=(200, 240)):
    template = np.random.default_rng(842).integers(0, 256, size=(19, 23), dtype=np.uint8)
    pixels = np.random.default_rng(92).integers(0, 70, size=size, dtype=np.uint8)
    tile = cv2.resize(template, None, fx=scale, fy=scale, interpolation=cv2.INTER_LINEAR)
    for x, y in positions:
        pixels[y : y + tile.shape[0], x : x + tile.shape[1]] = tile
    return {
        "protocolVersion": 1,
        "requestId": "req-template-9",
        "observationId": "obs-template-5",
        "sessionEpoch": "session-template-3",
        "generation": 22,
        "remainingMs": 30000,
        "screenshot": encoded(pixels),
        "template": encoded(template),
    }


def test_unique_match_exact_coordinates_observation_and_no_provider_call():
    def fail_provider(_):
        pytest.fail("local matching must not send images to provider")

    config = ProviderConfig("https://fixture.invalid/v1", "model", "fixture-key")
    with TestClient(create_app(TOKEN, config, httpx.MockTransport(fail_provider))) as client:
        request = observation()
        response = client.post(ENDPOINT, headers=HEADERS, json=request)
        assert response.status_code == 200
        body = response.json()
        assert body["status"] == "resolved" and body["reason"] == "template_unique_match"
        for key in ("requestId", "observationId", "sessionEpoch", "generation"):
            assert body[key] == request[key]
        assert body["screenshotSha256"] == request["screenshot"]["sha256"]
        assert body["templateSha256"] == request["template"]["sha256"]
        assert body["candidates"][0]["bounds"] == {"x": 27, "y": 39, "width": 23, "height": 19}
        assert body["candidates"][0]["score"] > 0.999
        assert body["candidates"][0]["method"] == "templateCorrelation"
        assert "localTemplateMatch" in client.get("/health/ready", headers=HEADERS).json()["capabilities"]
        assert client.get("/health/ready", headers=HEADERS).json()["activeRequests"] == 0


@pytest.mark.parametrize("positions", [((0, 0), (23, 0)), ((0, 0), (217, 181))])
def test_duplicate_targets_are_ambiguous_even_adjacent_or_at_edges(positions):
    with TestClient(create_app(TOKEN)) as client:
        response = client.post(ENDPOINT, headers=HEADERS, json=observation(positions=positions)).json()
        assert response["status"] == "ambiguous"
        assert {(c["bounds"]["x"], c["bounds"]["y"]) for c in response["candidates"]} == set(positions)


def test_roi_selects_one_instance_and_returns_screenshot_coordinates():
    request = observation(positions=((27, 39), (150, 120)))
    request["roi"] = {"x": 130, "y": 100, "width": 70, "height": 70}
    with TestClient(create_app(TOKEN)) as client:
        body = client.post(ENDPOINT, headers=HEADERS, json=request).json()
        assert body["status"] == "resolved"
        assert body["candidates"][0]["bounds"] == {"x": 150, "y": 120, "width": 23, "height": 19}


def test_missing_target_does_not_return_best_guess():
    with TestClient(create_app(TOKEN)) as client:
        body = client.post(ENDPOINT, headers=HEADERS, json=observation(positions=())).json()
        assert body["status"] == "unresolved" and body["candidates"] == []
        assert body["reason"] == "template_not_found"


def test_multiscale_search_and_same_location_suppression():
    request = observation(scale=1.5)
    request["scales"] = [0.75, 1.0, 1.5, 1.51]
    with TestClient(create_app(TOKEN)) as client:
        body = client.post(ENDPOINT, headers=HEADERS, json=request).json()
        assert body["status"] == "resolved"
        assert body["candidates"][0]["bounds"] == {"x": 27, "y": 39, "width": 34, "height": 28}
        assert body["candidates"][0]["scale"] == 1.5
        assert body["searchedScales"] == [0.75, 1.0, 1.5, 1.51]


@pytest.mark.parametrize("kind", ["blank", "tiny", "larger_than_roi"])
def test_nondiscriminating_or_unsearchable_templates_are_unresolved(kind):
    request = observation()
    if kind == "blank":
        request["template"] = encoded(np.full((20, 20), 120, dtype=np.uint8))
    elif kind == "tiny":
        request["template"] = encoded(np.array([[0, 255], [255, 0]], dtype=np.uint8))
    else:
        request["roi"] = {"x": 0, "y": 0, "width": 10, "height": 10}
    with TestClient(create_app(TOKEN)) as client:
        body = client.post(ENDPOINT, headers=HEADERS, json=request).json()
        assert body["status"] == "unresolved" and body["candidates"] == []


@pytest.mark.parametrize(
    "change",
    [
        "hash",
        "base64",
        "dimensions",
        "roi",
        "fractional_roi",
        "pixels",
        "work",
        "scale",
        "duplicates",
        "protocol",
        "extra",
        "transparent",
        "format",
        "truncated",
    ],
)
def test_invalid_inputs_are_rejected_with_released_capacity(change):
    request = observation()
    if change == "hash":
        request["template"]["sha256"] = "0" * 64
    elif change == "base64":
        request["screenshot"]["bytesBase64"] = "https://fixture.invalid/image.png"
    elif change == "dimensions":
        request["screenshot"]["width"] += 1
    elif change == "roi":
        request["roi"] = {"x": 239, "y": 199, "width": 2, "height": 2}
    elif change == "fractional_roi":
        request["roi"] = {"x": 0.5, "y": 0, "width": 2, "height": 2}
    elif change == "pixels":
        request["screenshot"].update(width=8192, height=8192)
    elif change == "work":
        request["screenshot"].update(width=2048, height=4096)
        request["scales"] = [0.5, 0.75, 1.0, 1.25, 1.5]
    elif change == "scale":
        request["scales"] = [3]
    elif change == "duplicates":
        request["scales"] = [1, 1]
    elif change == "protocol":
        request["protocolVersion"] = 2
    elif change == "extra":
        request["execute"] = "tap"
    elif change == "transparent":
        request["template"] = encoded(np.zeros((19, 23, 4), dtype=np.uint8))
    elif change == "format":
        request["template"] = encoded(np.zeros((19, 23), dtype=np.uint8), image_format="BMP")
    else:
        data = base64.b64decode(request["screenshot"]["bytesBase64"])[:-20]
        request["screenshot"]["bytesBase64"] = base64.b64encode(data).decode()
        request["screenshot"]["sha256"] = hashlib.sha256(data).hexdigest()
    with TestClient(create_app(TOKEN)) as client:
        assert client.post(ENDPOINT, headers=HEADERS, json=request).status_code == 422
        assert client.get("/health/ready", headers=HEADERS).json()["activeRequests"] == 0


def test_auth_and_body_limit_precede_image_decoding():
    with TestClient(create_app(TOKEN)) as client:
        assert client.post(ENDPOINT, json=observation()).status_code == 401
        headers = {**HEADERS, "Content-Length": str(21 * 1024 * 1024)}
        assert client.post(ENDPOINT, headers=headers, content=b"{}").status_code == 413


def test_actual_algorithm_honors_expired_deadline_before_decoding():
    with pytest.raises(TimeoutError):
        match_template(
            TemplateMatchRequest.model_validate(observation()), threading.Event(), time.monotonic() - 1
        )


def test_timed_out_native_work_retains_shared_capacity_until_it_stops(monkeypatch):
    import riviu_gui.app as module

    release = threading.Event()
    started = threading.Barrier(3)

    def slow(request, stop, deadline):
        started.wait(timeout=5)
        assert release.wait(timeout=5)
        assert stop.is_set()
        raise TimeoutError("done")

    monkeypatch.setattr(module, "match_template", slow)
    request = observation()
    request["remainingMs"] = 100
    try:
        with TestClient(create_app(TOKEN)) as client, ThreadPoolExecutor(2) as pool:
            calls = [pool.submit(client.post, ENDPOINT, headers=HEADERS, json=request) for _ in range(2)]
            started.wait(timeout=5)
            assert all(call.result(timeout=5).status_code == 504 for call in calls)
            assert client.get("/health/ready", headers=HEADERS).json()["activeRequests"] == 2
            response = client.post(ENDPOINT, headers=HEADERS, json=observation())
            assert response.status_code == 429 and response.headers["Retry-After"] == "1"
            # Existing remote routes share the same two admission slots.
            from test_api import observation as remote_observation

            assert (
                client.post("/v1/gui/resolve", headers=HEADERS, json=remote_observation()).status_code == 429
            )
            release.set()
            for _ in range(100):
                if client.get("/health/ready", headers=HEADERS).json()["activeRequests"] == 0:
                    break
                time.sleep(0.01)
            else:
                pytest.fail("capacity not released after native worker stopped")
    finally:
        release.set()


def test_client_cancellation_does_not_release_running_native_slot(monkeypatch):
    import riviu_gui.app as module

    started = threading.Event()
    release = threading.Event()

    def slow(request, stop, deadline):
        started.set()
        assert release.wait(timeout=5)
        assert stop.is_set()
        raise TimeoutError("done")

    monkeypatch.setattr(module, "match_template", slow)

    async def scenario():
        app = create_app(TOKEN)
        async with app.router.lifespan_context(app):
            async with httpx.AsyncClient(
                transport=httpx.ASGITransport(app), base_url="http://fixture"
            ) as client:
                call = asyncio.create_task(client.post(ENDPOINT, headers=HEADERS, json=observation()))
                assert await asyncio.to_thread(started.wait, 5)
                call.cancel()
                with pytest.raises(asyncio.CancelledError):
                    await call
                assert (await client.get("/health/ready", headers=HEADERS)).json()["activeRequests"] == 1
                release.set()
                for _ in range(100):
                    if (await client.get("/health/ready", headers=HEADERS)).json()["activeRequests"] == 0:
                        break
                    await asyncio.sleep(0.01)
                else:
                    pytest.fail("native slot leaked")

    try:
        asyncio.run(scenario())
    finally:
        release.set()
