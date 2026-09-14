"""Local image matching contract. All coordinates refer to the submitted screenshot."""

from typing import Annotated, Literal

from pydantic import Field, model_validator

from .models import WireModel


class EncodedImage(WireModel):
    bytes_base64: str = Field(min_length=1, max_length=12 * 1024 * 1024)
    sha256: str = Field(pattern=r"^[a-f0-9]{64}$")
    width: int = Field(strict=True, ge=1, le=8192)
    height: int = Field(strict=True, ge=1, le=8192)


class PixelRect(WireModel):
    x: int = Field(strict=True, ge=0)
    y: int = Field(strict=True, ge=0)
    width: int = Field(strict=True, ge=1)
    height: int = Field(strict=True, ge=1)


class TemplateMatchRequest(WireModel):
    protocol_version: Literal[1]
    request_id: str = Field(min_length=1, max_length=128)
    observation_id: str = Field(min_length=1, max_length=128)
    session_epoch: str = Field(min_length=1, max_length=128)
    generation: int = Field(strict=True, gt=0)
    remaining_ms: int = Field(strict=True, gt=0, le=30000)
    screenshot: EncodedImage
    template: EncodedImage
    roi: PixelRect | None = None
    scales: list[Annotated[float, Field(ge=0.5, le=2, allow_inf_nan=False)]] = Field(
        default_factory=lambda: [1.0], min_length=1, max_length=9
    )
    threshold: float = Field(default=0.9, ge=0.5, le=1, allow_inf_nan=False)

    @model_validator(mode="after")
    def bound_work(self):
        if self.screenshot.width * self.screenshot.height > 8 * 1024 * 1024:
            raise ValueError("screenshot pixel budget exceeded")
        if self.template.width * self.template.height > 1024 * 1024:
            raise ValueError("template pixel budget exceeded")
        if len(set(self.scales)) != len(self.scales):
            raise ValueError("duplicate scale")
        roi = self.search_region
        if roi.x + roi.width > self.screenshot.width or roi.y + roi.height > self.screenshot.height:
            raise ValueError("ROI outside screenshot")
        positions = 0
        for scale in self.scales:
            width = max(1, round(self.template.width * scale))
            height = max(1, round(self.template.height * scale))
            if width <= roi.width and height <= roi.height:
                positions += (roi.width - width + 1) * (roi.height - height + 1)
        if positions > 32 * 1024 * 1024:
            raise ValueError("search budget exceeded; narrow ROI or scales")
        return self

    @property
    def search_region(self) -> PixelRect:
        return self.roi or PixelRect(x=0, y=0, width=self.screenshot.width, height=self.screenshot.height)


class TemplateCandidate(WireModel):
    bounds: PixelRect
    score: float = Field(ge=0, le=1, allow_inf_nan=False)
    scale: float = Field(ge=0.5, le=2, allow_inf_nan=False)
    method: Literal["templateCorrelation"] = "templateCorrelation"


class TemplateMatchResponse(WireModel):
    protocol_version: Literal[1] = 1
    request_id: str
    observation_id: str
    session_epoch: str
    generation: int
    screenshot_sha256: str
    template_sha256: str
    status: Literal["resolved", "unresolved", "ambiguous"]
    candidates: list[TemplateCandidate] = Field(max_length=8)
    reason: str
    elapsed_ms: int = Field(ge=0)
    searched_scales: list[float]
