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


def generate_image_bytes(
    client: "genai.Client",
    prompt: str,
    *,
    model: str = DEFAULT_MODEL,
    aspect_ratio: str = "1:1",
    image_size: str | None = "1K",
    refs: Sequence[RefImage] = (),
) -> bytes:
    """Run one Nano Banana 2 generation and return PNG bytes.

    `refs` are appended to the prompt as additional content parts — use
    for image-edit / reference flows (the previous OpenAI `images.edit`
    pattern). Each ref may be PNG/JPEG bytes, a `PIL.Image`, or a `Path`.

    `image_size` is silently ignored on older google-genai builds whose
    `ImageConfig` only exposes `aspect_ratio`.
    """
    if genai_types is None:
        raise RuntimeError("google-genai not installed")
    if aspect_ratio not in GEMINI_ASPECT_RATIOS:
        raise ValueError(
            f"aspect_ratio {aspect_ratio!r} not supported; valid: "
            f"{GEMINI_ASPECT_RATIOS}"
        )
    image_size = _normalize_image_size(image_size)

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
        raise RuntimeError("Gemini response had no candidates")
    for part in candidates[0].content.parts:
        inline = getattr(part, "inline_data", None)
        if inline is not None and getattr(inline, "data", None):
            return inline.data
    raise RuntimeError("Gemini response had no inline image data")
