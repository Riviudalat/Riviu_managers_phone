import base64
import hashlib
import io
import json
from pathlib import Path

import httpx
import pytest
from fastapi.testclient import TestClient
from PIL import Image

from riviu_gui.app import create_app
from riviu_gui.models import GuiRequest
from riviu_gui.provider import ProviderConfig

TOKEN = "fixture-service-token-" + "a" * 32
HEADERS = {"Authorization": "Bearer " + TOKEN}


def test_shared_rust_python_wire_fixture():
    fixture = Path(__file__).resolve().parents[3] / "crates/core/fixtures/gui-request.json"
    request = GuiRequest.model_validate_json(fixture.read_text(encoding="utf8"))
    assert request.app.system_locale == "en-US" and request.nodes[0].id == 3
    assert "observationId" in request.model_dump(by_alias=True)


def observation():
    image = io.BytesIO()
    Image.new("RGB", (200, 400), "white").save(image, format="PNG")
    png = image.getvalue()
    return {
        "protocolVersion": 1,
        "requestId": "request-1",
        "observationId": "observation-2",
        "sessionEpoch": "epoch-3",
        "generation": 7,
        "app": {
            "package": "com.example.app",
            "version": "2",
            "systemLocale": "en-US",
            "observedLanguage": "en",
            "width": 200,
            "height": 400,
        },
        "target": "profile",
        "expectedScreen": "feed",
        "remainingMs": 30000,
        "nodes": [
            {
                "id": 3,
                "parent": None,
                "text": "Profile",
                "description": "",
                "resourceId": "com.example.app:id/new",
                "className": "Button",
                "bounds": {"x": 10, "y": 300, "width": 50, "height": 50},
                "enabled": True,
                "clickable": True,
            }
        ],
        "screenshot": base64.b64encode(png).decode(),
        "screenshotSha256": hashlib.sha256(png).hexdigest(),
    }


def provider_transport(decision):
    def answer(request):
        assert request.headers["Authorization"] == "Bearer model-fixture-key"
        body = json.loads(request.content)
        assert "untrusted data" in body["messages"][0]["content"]
        return httpx.Response(
            200,
            json={
                "choices": [{"message": {"content": json.dumps(decision)}}],
                "usage": {"prompt_tokens": 17, "completion_tokens": 8},
            },
        )

    return httpx.MockTransport(answer)


@pytest.mark.parametrize("endpoint", ["/health/live", "/health/ready"])
def test_health_requires_token_and_reports_protocol(endpoint):
    with TestClient(create_app(TOKEN)) as client:
        assert client.get(endpoint).status_code == 401
        response = client.get(endpoint, headers=HEADERS)
        assert response.status_code == 200
        if endpoint.endswith("ready"):
            assert response.json()["protocolVersion"] == 1
            assert response.json()["providerReady"] is False


@pytest.mark.parametrize("endpoint", ["/v1/gui/resolve", "/v1/gui/recover"])
def test_model_result_is_bound_to_observation_and_never_dispatches(endpoint):
    config = ProviderConfig("https://fixture.invalid/v1", "vision-fixture", "model-fixture-key")
    decision = {"status": "resolved", "nodeIds": [3], "reason": "Profile navigation"}
    with TestClient(create_app(TOKEN, config, provider_transport(decision))) as client:
        response = client.post(endpoint, headers=HEADERS, json=observation())
        assert response.status_code == 200
        body = response.json()
        assert body["status"] == "resolved"
        assert body["requestId"] == "request-1" and body["observationId"] == "observation-2"
        assert body["generation"] == 7 and body["sessionEpoch"] == "epoch-3"
        assert body["candidates"][0]["nodeId"] == 3
        assert body["promptTokens"] == 17
        assert "model-fixture-key" not in response.text


@pytest.mark.parametrize("ids", [[99], [3, 3], []])
def test_untrusted_model_cannot_invent_or_duplicate_nodes(ids):
    config = ProviderConfig("https://fixture.invalid/v1", "vision-fixture", "model-fixture-key")
    with TestClient(
        create_app(
            TOKEN, config, provider_transport({"status": "resolved", "nodeIds": ids, "reason": "guess"})
        )
    ) as client:
        body = client.post("/v1/gui/resolve", headers=HEADERS, json=observation()).json()
        assert body["status"] == "unresolved" and body["candidates"] == []
        assert body["promptTokens"] == 17  # billed invalid outputs are still accounted for


@pytest.mark.parametrize("change", ["bad_hash", "bounds", "duplicate", "protocol", "unknown_field"])
def test_invalid_observation_rejected_before_provider(change):
    request = observation()
    if change == "bad_hash":
        request["screenshotSha256"] = "0" * 64
    elif change == "bounds":
        request["nodes"][0]["bounds"]["width"] = 500
    elif change == "duplicate":
        request["nodes"].append(request["nodes"][0])
    elif change == "protocol":
        request["protocolVersion"] = 99
    else:
        request["shell"] = "unexpected"
    with TestClient(create_app(TOKEN)) as client:
        assert client.post("/v1/gui/resolve", headers=HEADERS, json=request).status_code == 422


def test_transport_failure_has_stable_error_without_secrets():
    def fail(_):
        raise httpx.ConnectError("credential model-fixture-key")

    config = ProviderConfig("https://fixture.invalid/v1", "vision-fixture", "model-fixture-key")
    with TestClient(create_app(TOKEN, config, httpx.MockTransport(fail))) as client:
        response = client.post("/v1/gui/resolve", headers=HEADERS, json=observation())
        assert response.status_code == 502
        assert "model-fixture-key" not in response.text
        assert client.get("/health/ready", headers=HEADERS).json()["activeRequests"] == 0


def test_request_timeout_releases_capacity():
    import asyncio

    async def slow(_):
        await asyncio.sleep(1)
        return httpx.Response(200, json={})

    config = ProviderConfig("https://fixture.invalid/v1", "vision-fixture", "model-fixture-key")
    with TestClient(create_app(TOKEN, config, httpx.MockTransport(slow))) as client:
        request = observation()
        request["remainingMs"] = 20
        assert client.post("/v1/gui/resolve", headers=HEADERS, json=request).status_code == 504
        assert client.get("/health/ready", headers=HEADERS).json()["activeRequests"] == 0


def test_public_effect_nodes_are_excluded_from_navigation_candidates():
    config = ProviderConfig("https://fixture.invalid/v1", "vision-fixture", "model-fixture-key")
    with TestClient(
        create_app(
            TOKEN, config, provider_transport({"status": "resolved", "nodeIds": [3], "reason": "send"})
        )
    ) as client:
        request = observation()
        request["nodes"][0]["text"] = "Post"
        response = client.post("/v1/gui/resolve", headers=HEADERS, json=request).json()
        assert response["status"] == "unresolved" and response["candidates"] == []
