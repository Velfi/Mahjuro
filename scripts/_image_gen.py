"""Shared Google Gemini "Nano Banana 2" image-gen helper.

Every `scripts/generate_*.py` and `scripts/gen_*.py` asset generator routes
its API call through here. Keeps a single place to update model names, env
var conventions, and request shapes.

Default model: `gemini-3.1-flash-image-preview` (Nano Banana 2). Reads the
API key from `GEMINI_API_KEY` (preferred) or `GOOGLE_API_KEY`.

Public surface:

  • `init_client()` — return an authenticated `genai.Client`.
  • `generate_image_bytes(client, prompt, *, model, aspect_ratio,
    image_size, refs=())` — text-to-image or image-reference/edit, returns
    PNG bytes from the first inline_data part in the response.
  • `parse_size(size)` — translate legacy `--size` strings (e.g.
    "1024x1536", "1K", "1:1@2K") into Gemini `(aspect_ratio, image_size)`.
"""

from __future__ import annotations

import io
import os
import re
import sys
import time
from pathlib import Path
from typing import Iterable, Sequence, Union

try:
    from google import genai
    from google.genai import types as genai_types
except ImportError:  # pragma: no cover - exercised only without dep installed
    genai = None  # type: ignore[assignment]
    genai_types = None  # type: ignore[assignment]

try:
    from PIL import Image
except ImportError:  # pragma: no cover - Pillow is a hard dep elsewhere
    Image = None  # type: ignore[assignment]


# Public defaults.
DEFAULT_MODEL = "gemini-3.1-flash-image-preview"

# Valid Nano Banana 2 aspect ratios + image-size tiers.
# Source: Vertex AI Gemini 3.1 Flash Image docs (Feb 2026).
GEMINI_ASPECT_RATIOS: tuple[str, ...] = (
    "1:1",
    "1:4",
    "1:8",
    "2:3",
    "3:2",
    "3:4",
    "4:1",
    "4:3",
    "4:5",
    "5:4",
    "8:1",
    "9:16",
    "16:9",
    "21:9",
)
GEMINI_IMAGE_SIZES: tuple[str, ...] = ("512px", "1K", "2K", "4K")

# Older google-genai builds spelled the smallest tier `"512"`. Accept either.
_SIZE_ALIASES = {"512": "512px"}


def _normalize_image_size(size: str | None) -> str | None:
    if size is None:
        return None
    normalized = _SIZE_ALIASES.get(size, size)
    if normalized not in GEMINI_IMAGE_SIZES:
        raise ValueError(
            f"image_size {size!r} not supported; valid: {GEMINI_IMAGE_SIZES} "
            "(or '512' as alias for '512px')"
        )
    return normalized


def _supports_image_size() -> bool:
    """Detect whether the installed google-genai exposes ImageConfig.image_size."""
    if genai_types is None:
        return False
    fields = getattr(genai_types.ImageConfig, "model_fields", {})
    return "image_size" in fields


RefImage = Union[bytes, bytearray, "Image.Image", Path]


def init_client(api_key: str | None = None) -> "genai.Client":
    """Authenticate against Gemini using GEMINI_API_KEY / GOOGLE_API_KEY."""
    if genai is None:
        print(
            "Error: google-genai not installed. "
            "Run: pip install -r scripts/requirements.txt",
            file=sys.stderr,
        )
        sys.exit(1)
    key = (
        api_key
        or os.environ.get("GEMINI_API_KEY")
        or os.environ.get("GOOGLE_API_KEY")
    )
    if not key:
        print(
            "Error: GEMINI_API_KEY (or GOOGLE_API_KEY) environment variable "
            "not set.",
            file=sys.stderr,
        )
        sys.exit(1)
    return genai.Client(api_key=key)


def parse_size(size: str) -> tuple[str, str]:
    """Translate a CLI `--size` value into `(aspect_ratio, image_size)`.

    Accepted forms:

      • Gemini tier alone (`"1K"`, `"2K"`, `"4K"`, `"512px"`) — square 1:1.
      • Legacy `"WxH"` pixel pair (e.g. `"1024x1536"`) — closest Gemini
        aspect ratio is picked, image size is the smallest tier whose long
        edge meets the requested pixels.
      • Explicit `"ASPECT@TIER"` (e.g. `"9:16@2K"`).
    """
    s = size.strip()
    if "@" in s:
        ar, tier = (part.strip() for part in s.split("@", 1))
        if ar not in GEMINI_ASPECT_RATIOS:
            raise ValueError(
                f"unknown aspect ratio {ar!r}; valid: {GEMINI_ASPECT_RATIOS}"
            )
        if tier not in GEMINI_IMAGE_SIZES:
            raise ValueError(
                f"unknown image size {tier!r}; valid: {GEMINI_IMAGE_SIZES}"
            )
        return ar, tier
    if s in GEMINI_IMAGE_SIZES:
        return "1:1", s
    m = re.fullmatch(r"(\d+)\s*x\s*(\d+)", s, re.I)
    if not m:
        raise ValueError(
            f"unrecognised --size value {s!r}; expected WxH, a Gemini tier "
            f"({'/'.join(GEMINI_IMAGE_SIZES)}), or ASPECT@TIER"
        )
    w, h = int(m.group(1)), int(m.group(2))
    return _closest_aspect(w, h), _closest_tier(max(w, h))


def _closest_aspect(w: int, h: int) -> str:
    target = w / h
    best = "1:1"
    best_d = float("inf")
    for ratio in GEMINI_ASPECT_RATIOS:
        rw, rh = (int(p) for p in ratio.split(":"))
        d = abs((rw / rh) - target)
        if d < best_d:
            best_d = d
            best = ratio
    return best


def _closest_tier(long_edge: int) -> str:
    if long_edge <= 512:
        return "512px"
    if long_edge <= 1024:
        return "1K"
    if long_edge <= 2048:
        return "2K"
    return "4K"


def _coerce_ref(ref: RefImage):
    """Normalise a reference image into something the SDK accepts."""
    assert genai_types is not None
    if isinstance(ref, Path):
        data = ref.read_bytes()
        return genai_types.Part.from_bytes(data=data, mime_type="image/png")
    if isinstance(ref, (bytes, bytearray)):
        return genai_types.Part.from_bytes(
            data=bytes(ref), mime_type="image/png"
        )
    if Image is not None and isinstance(ref, Image.Image):
        buf = io.BytesIO()
        ref.save(buf, format="PNG")
        return genai_types.Part.from_bytes(
            data=buf.getvalue(), mime_type="image/png"
        )
    raise TypeError(f"unsupported ref type: {type(ref).__name__}")


def _candidate_image_parts(candidate) -> list:
    """Return inline image parts from one candidate, or [] if content is missing."""
    content = getattr(candidate, "content", None)
    if content is None:
        return []
    parts = getattr(content, "parts", None)
    if not parts:
        return []
    return [
        part
        for part in parts
        if getattr(getattr(part, "inline_data", None), "data", None)
    ]


def _describe_gemini_response(response, candidate=None) -> str:
    """Summarise why a Gemini image response did not include usable bytes."""
    details: list[str] = []

    prompt_feedback = getattr(response, "prompt_feedback", None)
    if prompt_feedback is not None:
        block_reason = getattr(prompt_feedback, "block_reason", None)
        if block_reason:
            details.append(f"prompt block_reason={block_reason!r}")
        block_message = getattr(prompt_feedback, "block_reason_message", None)
        if block_message:
            details.append(f"prompt block_message={block_message!r}")

    if candidate is not None:
        finish_reason = getattr(candidate, "finish_reason", None)
        if finish_reason:
            details.append(f"finish_reason={finish_reason!r}")
        if getattr(candidate, "content", None) is None:
            details.append("candidate content missing")
        else:
            parts = getattr(candidate.content, "parts", None)
            if not parts:
                details.append("candidate parts missing")

        safety_ratings = getattr(candidate, "safety_ratings", None) or []
        blocked = [
            f"{getattr(r, 'category', '?')}={getattr(r, 'probability', '?')}"
            for r in safety_ratings
            if str(getattr(r, "probability", "")).endswith("HIGH")
            or str(getattr(r, "blocked", False)).lower() == "true"
        ]
        if blocked:
            details.append("safety: " + ", ".join(blocked))

    return "; ".join(details) if details else "unknown failure"


def _generate_image_bytes_once(
    client: "genai.Client",
    prompt: str,
    *,
    model: str,
    aspect_ratio: str,
    image_size: str | None,
    refs: Sequence[RefImage],
) -> bytes:
    contents: list = [prompt]
    for ref in refs:
        contents.append(_coerce_ref(ref))

    image_config_kwargs: dict[str, str] = {"aspect_ratio": aspect_ratio}
    if image_size is not None and _supports_image_size():
        image_config_kwargs["image_size"] = image_size

    response = client.models.generate_content(
        model=model,
        contents=contents,
        config=genai_types.GenerateContentConfig(
            response_modalities=["Image"],
            image_config=genai_types.ImageConfig(**image_config_kwargs),
        ),
    )

    candidates = getattr(response, "candidates", None) or []
    if not candidates:
        detail = _describe_gemini_response(response)
        raise RuntimeError(f"Gemini response had no candidates ({detail})")

    for candidate in candidates:
        for part in _candidate_image_parts(candidate):
            return part.inline_data.data

    detail = _describe_gemini_response(response, candidates[0])
    raise RuntimeError(f"Gemini response had no inline image data ({detail})")


def generate_image_bytes(
    client: "genai.Client",
    prompt: str,
    *,
    model: str = DEFAULT_MODEL,
    aspect_ratio: str = "1:1",
    image_size: str | None = "1K",
    refs: Sequence[RefImage] = (),
    max_attempts: int = 3,
    retry_delay: float = 2.0,
) -> bytes:
    """Run one Nano Banana 2 generation and return PNG bytes.

    `refs` are appended to the prompt as additional content parts — use
    for image-edit / reference flows (the previous OpenAI `images.edit`
    pattern). Each ref may be PNG/JPEG bytes, a `PIL.Image`, or a `Path`.

    `image_size` is silently ignored on older google-genai builds whose
    `ImageConfig` only exposes `aspect_ratio`.

    Retries transient empty/blocked responses up to `max_attempts` times.
    """
    if genai_types is None:
        raise RuntimeError("google-genai not installed")
    if aspect_ratio not in GEMINI_ASPECT_RATIOS:
        raise ValueError(
            f"aspect_ratio {aspect_ratio!r} not supported; valid: "
            f"{GEMINI_ASPECT_RATIOS}"
        )
    image_size = _normalize_image_size(image_size)

    attempts = max(1, max_attempts)
    last_error: RuntimeError | None = None
    for attempt in range(attempts):
        try:
            return _generate_image_bytes_once(
                client,
                prompt,
                model=model,
                aspect_ratio=aspect_ratio,
                image_size=image_size,
                refs=refs,
            )
        except RuntimeError as exc:
            last_error = exc
            if attempt + 1 >= attempts:
                break
            delay = retry_delay * (2**attempt)
            print(
                f"  Gemini image gen failed ({exc}); retrying in {delay:.1f}s …",
                file=sys.stderr,
            )
            time.sleep(delay)

    assert last_error is not None
    raise last_error
