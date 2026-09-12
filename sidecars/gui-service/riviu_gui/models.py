from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator
from pydantic.alias_generators import to_camel


class WireModel(BaseModel):
    model_config = ConfigDict(alias_generator=to_camel, populate_by_name=True, extra="forbid")


class Rect(WireModel):
    x: float = Field(ge=0, allow_inf_nan=False)
    y: float = Field(ge=0, allow_inf_nan=False)
    width: float = Field(gt=0, allow_inf_nan=False)
    height: float = Field(gt=0, allow_inf_nan=False)


class AppContext(WireModel):
    package: str = Field(min_length=1, max_length=200, pattern=r"^[A-Za-z0-9_.]+$")
    version: str = Field(max_length=100)
    system_locale: str = Field(max_length=100)
    observed_language: str | None
    width: int = Field(gt=0, le=16384)
    height: int = Field(gt=0, le=16384)


class GuiNode(WireModel):
    id: int = Field(ge=0)
    parent: int | None
    text: str = Field(max_length=8192)
    description: str = Field(max_length=8192)
    resource_id: str = Field(max_length=512)
    class_name: str = Field(max_length=512)
    bounds: Rect
    enabled: bool | None
    clickable: bool | None
    visible: bool | None = None


class GuiScope(WireModel):
    run_id: str
    assignment_id: str | None
    device_id: str
    deadline_ms: int | None


class GuiRequest(WireModel):
    scope: GuiScope | None = None
    protocol_version: Literal[1]
    request_id: str = Field(min_length=1, max_length=128)
    observation_id: str = Field(min_length=1, max_length=128)
    session_epoch: str = Field(min_length=1, max_length=128)
    generation: int = Field(gt=0)
    app: AppContext
    target: str = Field(min_length=1, max_length=200)
    expected_screen: str = Field(min_length=1, max_length=100)
    remaining_ms: int = Field(gt=0, le=90000)
    nodes: list[GuiNode] = Field(max_length=32768)
    screenshot: str = Field(max_length=16 * 1024 * 1024)
    screenshot_sha256: str = Field(pattern=r"^[a-f0-9]{64}$")

    @model_validator(mode="after")
    def unique_nodes(self):
        ids = {node.id for node in self.nodes}
        if len(ids) != len(self.nodes):
            raise ValueError("duplicate node id")
        for node in self.nodes:
            if (
                node.bounds.x + node.bounds.width > self.app.width
                or node.bounds.y + node.bounds.height > self.app.height
            ):
                raise ValueError("node outside viewport")
        return self


class TargetCandidate(WireModel):
    node_id: int | None
    bounds: Rect
    method: str
    evidence: list[str]


class GuiResponse(WireModel):
    protocol_version: Literal[1] = 1
    request_id: str
    observation_id: str
    session_epoch: str
    generation: int
    status: Literal["resolved", "unresolved", "ambiguous"]
    candidates: list[TargetCandidate]
    reason: str
    model: str | None = None
    elapsed_ms: int = 0
    prompt_tokens: int | None = None
    completion_tokens: int | None = None
    cost_usd: float | None = Field(default=None, ge=0, allow_inf_nan=False)


class ModelDecision(WireModel):
    status: Literal["resolved", "unresolved", "ambiguous"]
    node_ids: list[Annotated[int, Field(ge=0)]] = Field(max_length=8)
    reason: str = Field(max_length=1000)


class Readiness(WireModel):
    service_version: str
    protocol_version: Literal[1] = 1
    ready: bool
    provider_ready: bool
    capabilities: list[str]
    active_requests: int
