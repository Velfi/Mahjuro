"""Tests for talisman heightmap → mask derivation."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from _talisman_art_common import (  # noqa: E402
    _foreground_ratio,
    mask_from_height_image,
)


def _enclosed_void_areas(mask: Image.Image) -> list[int]:
    w, h = mask.size
    data = list(mask.getdata())
    n = w * h

    def idx(x: int, y: int) -> int:
        return y * w + x

    seen = [False] * n
    areas: list[int] = []
    for y in range(h):
        for x in range(w):
            i = idx(x, y)
            if data[i] >= 128 or seen[i]:
                continue
            touches_border = False
            count = 0
            stack = [(x, y)]
            seen[i] = True
            while stack:
                cx, cy = stack.pop()
                count += 1
                if cx == 0 or cy == 0 or cx == w - 1 or cy == h - 1:
                    touches_border = True
                for nx, ny in ((cx - 1, cy), (cx + 1, cy), (cx, cy - 1), (cx, cy + 1)):
                    if 0 <= nx < w and 0 <= ny < h:
                        j = idx(nx, ny)
                        if not seen[j] and data[j] < 128:
                            seen[j] = True
                            stack.append((nx, ny))
            if not touches_border:
                areas.append(count)
    return areas


@pytest.mark.parametrize(
    "height_name",
    [
        "memorial_skipper.png",
        "talisman_souzu.png",
        "talisman_wildflower.png",
    ],
)
def test_auto_mask_foreground_ratio_sane(height_name: str) -> None:
    path = ROOT / "assets" / "textures" / "talismans" / height_name
    if not path.exists():
        pytest.skip(f"missing fixture {path}")
    with Image.open(path) as im:
        mask = mask_from_height_image(im, method="auto")
    ratio = _foreground_ratio(mask)
    assert 0.08 <= ratio <= 0.88, f"{height_name} foreground {ratio:.3f} out of range"


def test_strip_border_matte_frame_flattens_edge_band() -> None:
    from _talisman_art_common import strip_border_matte_frame

    path = ROOT / "assets" / "textures" / "talismans" / "talisman_wildflower.png"
    if not path.exists():
        pytest.skip("wildflower heightmap missing")
    with Image.open(path) as im:
        before = im.convert("L")
        synthetic = before.copy()
        px = synthetic.load()
        w, h = synthetic.size
        for x in range(w):
            px[x, 0] = 105
            px[x, h - 1] = 105
        for y in range(h):
            px[0, y] = 105
            px[w - 1, y] = 105
        after = strip_border_matte_frame(synthetic)
    assert after.getpixel((0, 0)) == 128
    assert after.getpixel((w // 2, h // 2)) == before.getpixel((w // 2, h // 2))


def test_memorial_skipper_preserves_large_internal_piercings() -> None:
    path = ROOT / "assets" / "textures" / "talismans" / "memorial_skipper.png"
    if not path.exists():
        pytest.skip("memorial_skipper heightmap missing")
    with Image.open(path) as im:
        mask = mask_from_height_image(im, method="auto")
    areas = _enclosed_void_areas(mask)
    assert any(a >= 200 for a in areas), (
        f"expected a large enclosed void, got top {sorted(areas, reverse=True)[:5]}"
    )
