"""Observation-bound local OCR protocol; languages select packaged traineddata."""

from typing import Literal

from pydantic import Field, model_validator

from .models import WireModel
from .template_models import EncodedImage, PixelRect


def default_languages() -> list[Literal["vi", "en"]]:
    return ["vi", "en"]


class OcrRequest(WireModel):
    protocol_version: Literal[1]
    request_id: str = Field(min_length=1, max_length=128)
    observation_id: str = Field(min_length=1, max_length=128)
    session_epoch: str = Field(min_length=1, max_length=128)
    generation: int = Field(strict=True, gt=0)
    remaining_ms: int = Field(strict=True, gt=0, le=30000)
    screenshot: EncodedImage
    roi: PixelRect | None = None
    languages: list[Literal["vi", "en"]] = Field(default_factory=default_languages, min_length=1, max_length=2)
    min_confidence: float = Field(default=0.7, ge=0, le=1, allow_inf_nan=False)

    @model_validator(mode="after")
    def bounded_observation(self):
        if self.screenshot.width * self.screenshot.height > 8 * 1024 * 1024:
            raise ValueError("OCR image pixel budget exceeded")
        if len(set(self.languages)) != len(self.languages):
            raise ValueError("duplicate OCR language")
        region = self.search_region
        if region.x + region.width > self.screenshot.width or region.y + region.height > self.screenshot.height:
            raise ValueError("OCR region outside image")
        return self

    @property
    def search_region(self) -> PixelRect:
        return self.roi or PixelRect(x=0, y=0, width=self.screenshot.width, height=self.screenshot.height)


class OcrLine(WireModel):
    text: str = Field(min_length=1, max_length=1024)
    bounds: PixelRect
    confidence: float = Field(ge=0, le=1, allow_inf_nan=False)


class OcrResponse(WireModel):
    protocol_version: Literal[1] = 1
    request_id: str
    observation_id: str
    session_epoch: str
    generation: int
    screenshot_sha256: str
    status: Literal["resolved", "unresolved"]
    text: str = Field(max_length=4096)
    lines: list[OcrLine] = Field(max_length=128)
    engine: str
    elapsed_ms: int = Field(ge=0)
