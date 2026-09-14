import asyncio
import base64
import hashlib
import json
import pathlib
from unittest.mock import AsyncMock

import pytest
from fastapi.testclient import TestClient

from riviu_gui.app import create_app
from riviu_gui.ocr import recognize, run_ocr, verified_models
from riviu_gui.ocr_models import OcrRequest

TOKEN = "ocr-test-token-" * 4
HEADERS = {"Authorization": "Bearer " + TOKEN}
FIXTURE = pathlib.Path(__file__).parent / "fixtures/ocr-vietnamese-english.png"


def request_body(**changes):
    data = FIXTURE.read_bytes()
    return {
        "protocolVersion": 1, "requestId": "request-ocr", "observationId": "ocr-fixture",
        "sessionEpoch": "test-session", "generation": 1, "remainingMs": 30000,
        "screenshot": {"bytesBase64": base64.b64encode(data).decode(), "sha256": hashlib.sha256(data).hexdigest(),
                       "width": 900, "height": 280},
        "languages": ["vi", "en"], "minConfidence": 0.7, **changes,
    }


def test_shared_rust_python_request_fixture_roundtrips():
    shared = pathlib.Path(__file__).resolve().parents[3] / "crates/core/fixtures/ocr-request.json"
    raw = json.loads(shared.read_text(encoding="utf8"))
    request = OcrRequest.model_validate(raw)
    assert request.model_dump(by_alias=True, exclude_none=True) == raw


@pytest.fixture(scope="module")
def staged_models():
    # Unit contracts also run on a clean checkout; the real inference gate is
    # explicitly enabled after stage_ocr_models.py by the package/build workflow.
    try:
        verified_models()
    except FileNotFoundError:
        pytest.skip("run stage_ocr_models.py for the real inference benchmark")


def test_real_vietnamese_english_recognition_preserves_diacritics(staged_models):
    result = recognize(OcrRequest.model_validate(request_body()))
    assert result.text == "Xin chào Việt Nam\nRiviu Manager - local OCR"
    assert result.status == "resolved"
    assert len(result.lines) == 2
    assert all(line.confidence >= 0.7 for line in result.lines)
    assert result.elapsed_ms < 30000


def test_real_roi_returns_global_coordinates_and_blank_is_unresolved(staged_models):
    request = OcrRequest.model_validate(request_body(roi={"x": 0, "y": 100, "width": 900, "height": 100}))
    result = recognize(request)
    assert result.text == "Riviu Manager - local OCR"
    assert result.lines[0].bounds.y >= 100
    blank = recognize(OcrRequest.model_validate(request_body(roi={"x": 500, "y": 180, "width": 300, "height": 80})))
    assert blank.status == "unresolved" and blank.text == "" and not blank.lines
    strict = recognize(OcrRequest.model_validate(request_body(minConfidence=1.0)))
    assert strict.status == "unresolved" and strict.text == ""


@pytest.mark.parametrize("changes", [
    {"remainingMs": 30001}, {"remainingMs": 0}, {"generation": 0},
    {"languages": ["vi", "vi"]}, {"languages": ["zh"]}, {"languages": []},
    {"minConfidence": -0.1}, {"minConfidence": 1.1},
    {"roi": {"x": 890, "y": 0, "width": 20, "height": 50}},
])
def test_rejects_unbounded_or_unsupported_work(changes):
    with TestClient(create_app(TOKEN)) as client:
        assert client.post("/v1/gui/ocr", headers=HEADERS, json=request_body(**changes)).status_code == 422


def test_missing_models_report_capability_without_ever_downloading(monkeypatch, tmp_path):
    monkeypatch.setenv("RIVIU_OCR_MODEL_DIR", str(tmp_path))
    with TestClient(create_app(TOKEN)) as client:
        ready = client.get("/health/ready", headers=HEADERS).json()
        assert "ocrModelsMissing" in ready["capabilities"]
        assert "localOcr" not in ready["capabilities"]
        response = client.post("/v1/gui/ocr", headers=HEADERS, json=request_body())
        assert response.status_code == 503 and response.json()["detail"] == "gui_ocr_models_missing"
    assert list(tmp_path.iterdir()) == []


def test_corrupt_models_are_rejected(monkeypatch, tmp_path):
    manifest = json.loads((pathlib.Path(__file__).parents[1] / "riviu_gui/ocr-models.json").read_text())
    (tmp_path / manifest["files"][0]["path"]).write_bytes(b"corrupt")
    monkeypatch.setenv("RIVIU_OCR_MODEL_DIR", str(tmp_path))
    with pytest.raises(ValueError, match="gui_ocr_model_hash"):
        verified_models()


def test_ocr_http_is_authenticated_and_does_not_need_provider(staged_models):
    with TestClient(create_app(TOKEN)) as client:
        assert client.post("/v1/gui/ocr", json=request_body()).status_code == 401
        ready = client.get("/health/ready", headers=HEADERS).json()
        assert "localOcr" in ready["capabilities"] and not ready["providerReady"]
        response = client.post("/v1/gui/ocr", headers=HEADERS, json=request_body())
        assert response.status_code == 200
        assert "Việt Nam" in response.json()["text"]


def test_bad_image_hash_is_input_error_before_any_worker_spawn(staged_models, monkeypatch):
    import riviu_gui.ocr as module

    spawn = AsyncMock()
    monkeypatch.setattr(module.asyncio, "create_subprocess_exec", spawn)
    body = request_body()
    body["screenshot"]["sha256"] = "0" * 64
    with TestClient(create_app(TOKEN)) as client:
        assert client.post("/v1/gui/ocr", headers=HEADERS, json=body).status_code == 422
    spawn.assert_not_called()


def test_ocr_capacity_two_admitted_requests_then_releases_after_cancel(monkeypatch):
    import httpx

    import riviu_gui.app as module

    async def exercise():
        entered = 0
        finished = 0

        async def held(_request):
            nonlocal entered, finished
            entered += 1
            try:
                await asyncio.Event().wait()
            finally:
                finished += 1

        monkeypatch.setattr(module, "run_ocr", held)
        transport = httpx.ASGITransport(app=create_app(TOKEN))
        async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
            tasks = [asyncio.create_task(client.post("/v1/gui/ocr", headers=HEADERS, json=request_body(requestId=str(i)))) for i in range(2)]
            while entered != 2:
                await asyncio.sleep(0.001)
            third = await client.post("/v1/gui/ocr", headers=HEADERS, json=request_body(requestId="third"))
            assert third.status_code == 429 and third.headers["Retry-After"] == "1"
            for task in tasks:
                task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)
            assert finished == 2
            ready = await client.get("/health/ready", headers=HEADERS)
            assert ready.json()["activeRequests"] == 0

    asyncio.run(exercise())


def test_cancel_kills_and_reaps_the_native_worker(monkeypatch):
    import riviu_gui.ocr as module

    monkeypatch.setattr(module, "verified_models", lambda: ({}, {}))

    class Child:
        returncode = None
        killed = False
        reaped = False

        async def communicate(self, _data):
            await asyncio.Event().wait()

        def kill(self):
            self.killed = True
            self.returncode = -9

        async def wait(self):
            self.reaped = True

    async def exercise():
        child = Child()
        spawn = AsyncMock(return_value=child)
        monkeypatch.setattr(module.asyncio, "create_subprocess_exec", spawn)
        task = asyncio.create_task(run_ocr(OcrRequest.model_validate(request_body())))
        while not spawn.called:
            await asyncio.sleep(0.001)
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
        assert child.killed and child.reaped

    asyncio.run(exercise())


def test_native_worker_deadline_kills_and_reaps(monkeypatch):
    import riviu_gui.ocr as module

    monkeypatch.setattr(module, "verified_models", lambda: ({}, {}))
    child = type("Child", (), {})()
    child.returncode = None
    child.communicate = AsyncMock(side_effect=asyncio.TimeoutError)
    child.kill = lambda: setattr(child, "returncode", -9)
    child.wait = AsyncMock()
    monkeypatch.setattr(module.asyncio, "create_subprocess_exec", AsyncMock(return_value=child))
    with pytest.raises(TimeoutError):
        asyncio.run(run_ocr(OcrRequest.model_validate(request_body(remainingMs=1))))
    assert child.returncode == -9
    child.wait.assert_awaited_once()
