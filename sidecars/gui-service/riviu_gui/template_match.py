"""Bounded, observation-only template search; no filesystem or device operations."""

import base64
import hashlib
import io
import threading
import time

import cv2
import numpy as np
from PIL import Image

from .template_models import (
    EncodedImage,
    PixelRect,
    TemplateCandidate,
    TemplateMatchRequest,
    TemplateMatchResponse,
)

# Two admitted requests, each with one native worker. Do not multiply CPU demand
# by OpenCV's default machine-wide thread count on a device farm host.
cv2.setNumThreads(1)
cv2.ocl.setUseOpenCL(False)


def decode_image(image: EncodedImage, byte_limit: int) -> np.ndarray:
    if len(image.bytes_base64) > ((byte_limit + 2) // 3) * 4:
        raise ValueError("template_image_invalid")
    try:
        data = base64.b64decode(image.bytes_base64, validate=True)
        if len(data) > byte_limit or hashlib.sha256(data).hexdigest() != image.sha256:
            raise ValueError("image hash or byte budget mismatch")
        with Image.open(io.BytesIO(data)) as decoded:
            if decoded.format not in {"PNG", "JPEG"} or decoded.size != (image.width, image.height):
                raise ValueError("image format or dimensions mismatch")
            if getattr(decoded, "n_frames", 1) != 1:
                raise ValueError("animated images are not observations")
            decoded.verify()
        with Image.open(io.BytesIO(data)) as decoded:
            # Partial transparency changes meaning with the background; never
            # silently discard it and match invisible RGB data.
            if "A" in decoded.getbands() or "transparency" in decoded.info:
                if decoded.convert("RGBA").getchannel("A").getextrema() != (255, 255):
                    raise ValueError("transparent image")
            return np.asarray(decoded.convert("L"), dtype=np.uint8).copy()
    except (OSError, ValueError, Image.DecompressionBombError) as error:
        raise ValueError("template_image_invalid") from error


def same_location(left: PixelRect, right: PixelRect) -> bool:
    intersection = max(0, min(left.x + left.width, right.x + right.width) - max(left.x, right.x)) * max(
        0, min(left.y + left.height, right.y + right.height) - max(left.y, right.y)
    )
    union = left.width * left.height + right.width * right.height - intersection
    return intersection / union > 0.3


def match_template(
    request: TemplateMatchRequest, stop: threading.Event, deadline: float
) -> TemplateMatchResponse:
    started = time.monotonic()

    def check_deadline():
        if stop.is_set() or time.monotonic() >= deadline:
            raise TimeoutError("gui_deadline")

    check_deadline()
    screenshot = decode_image(request.screenshot, 8 * 1024 * 1024)
    template = decode_image(request.template, 2 * 1024 * 1024)
    check_deadline()
    roi = request.search_region
    region = screenshot[roi.y : roi.y + roi.height, roi.x : roi.x + roi.width]
    candidates: list[TemplateCandidate] = []
    searched: list[float] = []
    low_detail = float(template.std()) < 2.0 or min(template.shape) < 3
    truncated = False
    if not low_detail:
        seen_sizes: set[tuple[int, int]] = set()
        for scale in request.scales:
            check_deadline()
            width = max(1, round(request.template.width * scale))
            height = max(1, round(request.template.height * scale))
            if width > roi.width or height > roi.height or (width, height) in seen_sizes:
                continue
            seen_sizes.add((width, height))
            scaled = cv2.resize(
                template, (width, height), interpolation=cv2.INTER_AREA if scale < 1 else cv2.INTER_LINEAR
            )
            if min(width, height) < 3 or float(scaled.std()) < 2.0:
                continue
            searched.append(scale)
            scores = cv2.matchTemplate(region, scaled, cv2.TM_CCOEFF_NORMED)
            np.nan_to_num(scores, copy=False, nan=-1, posinf=-1, neginf=-1)
            # At most nine peaks per scale. The ninth is enough to establish
            # ambiguity even when the response caps evidence at eight boxes.
            for peak in range(9):
                check_deadline()
                _, score, _, (x, y) = cv2.minMaxLoc(scores)
                if score < request.threshold:
                    break
                candidate = TemplateCandidate(
                    bounds=PixelRect(x=x + roi.x, y=y + roi.y, width=width, height=height),
                    score=min(1.0, max(0.0, score)),
                    scale=scale,
                )
                candidates.append(candidate)
                if peak == 8:
                    truncated = True
                # Suppress only nearby peaks for this same feature. Separate
                # copies next to one another remain candidates.
                radius_x, radius_y = max(1, width // 3), max(1, height // 3)
                scores[max(0, y - radius_y) : y + radius_y + 1, max(0, x - radius_x) : x + radius_x + 1] = -1
    check_deadline()
    distinct: list[TemplateCandidate] = []
    for candidate in sorted(candidates, key=lambda item: item.score, reverse=True):
        if not any(same_location(candidate.bounds, existing.bounds) for existing in distinct):
            distinct.append(candidate)
    if truncated or len(distinct) > 1:
        status, reason = "ambiguous", "template_multiple_matches"
    elif distinct:
        status, reason = "resolved", "template_unique_match"
    else:
        status = "unresolved"
        reason = (
            "template_low_detail"
            if low_detail
            else "template_not_found"
            if searched
            else "template_outside_roi"
        )
    return TemplateMatchResponse(
        request_id=request.request_id,
        observation_id=request.observation_id,
        session_epoch=request.session_epoch,
        generation=request.generation,
        screenshot_sha256=request.screenshot.sha256,
        template_sha256=request.template.sha256,
        status=status,
        candidates=distinct[:8],
        reason=reason,
        elapsed_ms=round((time.monotonic() - started) * 1000),
        searched_scales=searched,
    )
