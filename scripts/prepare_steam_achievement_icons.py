#!/usr/bin/env python3
"""Copy achievement icon sources and export Steamworks JPGs (256×256, locked + unlocked)."""

from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageEnhance, ImageOps

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "assets" / "steam_assets" / "achievements"

# Steamworks recommends 256×256 JPG; 64×64 minimum. Locked icons should be grayscale.
STEAM_SIZE = 256
JPG_QUALITY = 92
# Flatten RGBA sources onto a neutral dark field before JPG export.
FLATTEN_BG = (18, 15, 13)

# API name -> source path (relative to repo root)
ACHIEVEMENTS: list[tuple[str, str]] = [
    ("TUTORIAL_COMPLETE", "assets/textures/relics/diligence_object.png"),
    ("FIRST_STRUCTURE", "assets/textures/relics/crown_of_patterns_object.png"),
    ("FIRST_BLIND_CLEARED", "assets/textures/temptations/processed/tag_treasure_chest.png"),
    ("FIRST_BOSS_DEFEATED", "assets/textures/ordeal_icons/processed/ordeal_hermit.png"),
    ("FIRST_RUN_COMPLETED", "assets/textures/relics/gold_idol_object.png"),
    ("TEN_RUNS_PLAYED", "assets/textures/relics/curio_cabinet_object.png"),
    ("STAKE_2_UNLOCKED", "assets/textures/tile_sets/original/source_art/sunflower.png"),
    ("STAKE_3_UNLOCKED", "assets/textures/tile_sets/original/source_art/autumn.png"),
    ("STAKE_4_UNLOCKED", "assets/textures/tile_sets/original/source_art/winter.png"),
    ("ALL_BOSSES_SEEN", "assets/textures/ordeal_icons/atlas.png"),
    ("SILK_MOTH_EMERGED", "assets/textures/relics/silk_moth_object.png"),
    ("TAOTIE_AWAKENED", "assets/textures/relics/taotie_object.png"),
    ("GEESE_TAKE_FLIGHT", "assets/textures/relics/geese_object.png"),
    ("THIRTEEN_ORPHANS", "assets/textures/zodiacs/zodiac_qilin.png"),
    ("HOUSE_DEFEATED", "assets/textures/ordeal_icons/processed/ordeal_house.png"),
]

DISPLAY_NAMES = {
    "TUTORIAL_COMPLETE": "Tutorial Graduate",
    "FIRST_STRUCTURE": "First Structure",
    "FIRST_BLIND_CLEARED": "First Chamber",
    "FIRST_BOSS_DEFEATED": "Ordeal Down",
    "FIRST_RUN_COMPLETED": "Run Won",
    "TEN_RUNS_PLAYED": "Dedicated",
    "STAKE_2_UNLOCKED": "Summer Unlocked",
    "STAKE_3_UNLOCKED": "Autumn Unlocked",
    "STAKE_4_UNLOCKED": "Winter Unlocked",
    "ALL_BOSSES_SEEN": "Full Roster",
    "SILK_MOTH_EMERGED": "Silk Moth",
    "TAOTIE_AWAKENED": "Taotie",
    "GEESE_TAKE_FLIGHT": "Geese",
    "THIRTEEN_ORPHANS": "Thirteen Orphans",
    "HOUSE_DEFEATED": "Beat the House",
}


def square_crop(img: Image.Image) -> Image.Image:
    w, h = img.size
    side = min(w, h)
    left = (w - side) // 2
    top = (h - side) // 2
    return img.crop((left, top, left + side, top + side))


def _luminance(r: int, g: int, b: int) -> float:
    return 0.299 * r + 0.587 * g + 0.114 * b


def largest_bright_region_bbox(
    img: Image.Image,
    *,
    lum_threshold: int = 100,
    row_threshold: int = 50,
) -> tuple[int, int, int, int]:
    """Bounding box of the tallest band of bright pixels (e.g. ribbon animal art)."""
    w, h = img.size
    pixels = img.load()
    row_counts = [
        sum(
            1
            for x in range(w)
            if _luminance(pixels[x, y][0], pixels[x, y][1], pixels[x, y][2]) > lum_threshold
        )
        for y in range(h)
    ]

    regions: list[tuple[int, int]] = []
    in_content = False
    start = 0
    for y, count in enumerate(row_counts):
        if count > row_threshold:
            if not in_content:
                start = y
                in_content = True
        elif in_content:
            regions.append((start, y))
            in_content = False
    if in_content:
        regions.append((start, h))
    if not regions:
        return (0, 0, w, h)

    y0, y1 = max(regions, key=lambda span: span[1] - span[0])
    min_x, min_y, max_x, max_y = w, h, 0, 0
    for y in range(y0, y1):
        for x in range(w):
            r, g, b, _a = pixels[x, y]
            if _luminance(r, g, b) > lum_threshold:
                min_x = min(min_x, x)
                min_y = min(min_y, y)
                max_x = max(max_x, x)
                max_y = max(max_y, y)
    return (min_x, min_y, max_x + 1, max_y + 1)


def subject_square_crop(img: Image.Image, *, padding: float = 0.08) -> Image.Image:
    """Square crop centered on the main embroidered subject in a tall ribbon."""
    w, h = img.size
    x0, y0, x1, y1 = largest_bright_region_bbox(img)
    cx = (x0 + x1) // 2
    cy = (y0 + y1) // 2
    side = max(x1 - x0, y1 - y0)
    side = int(side * (1 + 2 * padding))
    side = min(side, w, h)
    left = max(0, min(cx - side // 2, w - side))
    top = max(0, min(cy - side // 2, h - side))
    return img.crop((left, top, left + side, top + side))


def achievement_square_crop(img: Image.Image) -> Image.Image:
    w, h = img.size
    if min(w, h) / max(w, h) < 0.5:
        return subject_square_crop(img)
    return square_crop(img)


def flatten_rgb(img: Image.Image) -> Image.Image:
    base = Image.new("RGB", img.size, FLATTEN_BG)
    base.paste(img, mask=img.split()[3])
    return base


def to_locked(img: Image.Image) -> Image.Image:
    rgb = flatten_rgb(img)
    gray = ImageOps.grayscale(rgb)
    return ImageEnhance.Brightness(gray).enhance(0.55)


def save_jpg(img: Image.Image, path: Path) -> None:
    if img.mode != "RGB":
        img = img.convert("RGB")
    img.save(path, format="JPEG", quality=JPG_QUALITY, optimize=True)


def clean_stale_outputs() -> None:
    if not OUT.is_dir():
        return
    for path in OUT.iterdir():
        if path.name == "manifest.txt":
            continue
        if path.suffix in {".png", ".jpg", ".jpeg"}:
            path.unlink()


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    clean_stale_outputs()

    manifest_lines = [
        "Mahjuro Steam achievement icons",
        "",
        "Steamworks: 256×256 JPG recommended; unlocked = colorful, locked = grayscale.",
        "Upload {API_NAME}_256_unlocked.jpg and {API_NAME}_256_locked.jpg per achievement.",
        "Regenerate: python3 scripts/prepare_steam_achievement_icons.py",
        "",
        "API Name | Display name | Steam JPG (unlocked) | source",
        "-------- | ------------ | -------------------- | ------",
    ]

    for api_name, rel_src in ACHIEVEMENTS:
        src = ROOT / rel_src
        if not src.is_file():
            raise SystemExit(f"missing source: {src}")

        with Image.open(src) as im:
            im = im.convert("RGBA")
            square = achievement_square_crop(im)
            resized = square.resize((STEAM_SIZE, STEAM_SIZE), Image.Resampling.LANCZOS)

            square.save(OUT / f"{api_name}_source.png")

            unlocked_jpg = OUT / f"{api_name}_256_unlocked.jpg"
            locked_jpg = OUT / f"{api_name}_256_locked.jpg"
            save_jpg(flatten_rgb(resized), unlocked_jpg)
            save_jpg(to_locked(resized), locked_jpg)

        manifest_lines.append(
            f"{api_name} | {DISPLAY_NAMES[api_name]} | {api_name}_256_unlocked.jpg | {rel_src}"
        )

    (OUT / "manifest.txt").write_text("\n".join(manifest_lines) + "\n", encoding="utf-8")
    print(f"wrote {len(ACHIEVEMENTS)} achievements to {OUT.relative_to(ROOT)}/")


if __name__ == "__main__":
    main()
