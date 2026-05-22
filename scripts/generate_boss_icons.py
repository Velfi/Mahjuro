#!/usr/bin/env python3
"""
Generate boss-blind encounter icons for Mahjuro and pack them into a sprite sheet.

Each boss (`BossKind` in `src/core/boss.rs`) gets a distinct faceted low-poly
inventory icon in the Walnut, Brass & Felt palette. Calls Google's Nano Banana 2
(`gemini-3.1-flash-image-preview`) for source art, then the shared post-process
pipeline (`_icon_atlas_postprocess.py`), then packs a 5×5 row-major atlas.

**Writes (under `assets/textures/boss_icons/` by default)**

  • `source/boss_{slug}.png` — raw API output (RGBA)
  • `processed/boss_{slug}.png` — cleaned icon, square `CELL_SIZE` (512×512)
  • `atlas.png` / `atlas.toml` — grid aligned with `BossKind::ALL`

Slug order MUST match `BossKind::ALL` in `src/core/boss.rs` and ids in
`assets/data/bosses.json`.

Usage:
    pip install google-genai pillow
    export GEMINI_API_KEY="..."
    python3 scripts/generate_boss_icons.py                  # missing only
    python3 scripts/generate_boss_icons.py --force         # regenerate all
    python3 scripts/generate_boss_icons.py --name drought
    python3 scripts/generate_boss_icons.py --boss 3        # 1-indexed BossKind::ALL
    python3 scripts/generate_boss_icons.py --list
    python3 scripts/generate_boss_icons.py --dry-run
    python3 scripts/generate_boss_icons.py --pack-only
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import time
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _icon_atlas_postprocess import (  # noqa: E402
    icon_postprocess,
    pack_processed_icons,
)
from _image_gen import (  # noqa: E402
    DEFAULT_MODEL,
    GEMINI_IMAGE_SIZES,
    generate_image_bytes,
    init_client,
)

try:
    from PIL import Image
except ImportError:
    print("Error: Pillow not installed. Run: pip install Pillow")
    sys.exit(1)


REPO_ROOT = Path(__file__).resolve().parent.parent
BOSS_JSON_PATH = REPO_ROOT / "assets" / "data" / "bosses.json"
BOSS_RS_PATH = REPO_ROOT / "src" / "core" / "boss.rs"
OUTPUT_DIR = REPO_ROOT / "assets" / "textures" / "boss_icons"

COLUMNS = 5
CELL_SIZE = 512
CONTENT_FILL = 0.82
WORK_PX = CELL_SIZE

# Per-tier rim accent (mirrors boss card tier read in-game).
BOSS_TIER_ACCENTS: dict[str, str] = {
    "soft": (
        "Tier accent: cool twilight-blue mist along the outer silhouette — "
        "soft indigo edge bloom, early-ante omen."
    ),
    "medium": (
        "Tier accent: warm brass-gold rim light tracing the silhouette — "
        "mid-run threat, confident metallic glints."
    ),
    "hard": (
        "Tier accent: hot amber-ruby rim fire along the silhouette — "
        "late-ante pressure, sharp warm edge highlights."
    ),
    "final": (
        "Tier accent: dramatic ruby-champagne rim flare on the silhouette — "
        "climactic final-boss weight, bold specular crest."
    ),
}

# One iconic, high-contrast subject per boss (64 px legibility). Keys = bosses.json `id`.
BOSS_VISUALS: dict[str, str] = {
    "drought": (
        "A cracked dry lakebed forming a harsh X of fissures with one bone-dry "
        "water gourd split open beside it — drought, no relief. Faceted earth "
        "planes in umber and dust-parchment."
    ),
    "whisper": (
        "Three curling wind ribbons cut through a single hollow ear-shaped "
        "listening shell — sound without voice. Bold S-curves, twilight-blue "
        "shadow facets."
    ),
    "tribute": (
        "A small brass offering bowl with one heavy gold coin half-dropped in "
        "and a tiny downward arrow carved on the bowl lip — pay to play. "
        "Chunky low-poly metal."
    ),
    "gate": (
        "A miniature torii gate made of ink-black lacquer bars with one wet "
        "ink splash frozen in mid-air before the threshold — characters blocked "
        "at the gate. Flat black vs parchment negative space."
    ),
    "grove": (
        "A tight cluster of three bamboo shoots bent under invisible weight — "
        "jade-green segmented stalks, one snapped segment on the ground."
    ),
    "coin": (
        "A thick copper wheel / coin seen almost edge-on with a single large "
        "dot pip embossed on the visible face — dots suit under pressure. "
        "Warm copper planes."
    ),
    "rot": (
        "A lotus bud half rotted: outer petals faceted jade, inner folds "
        "slumped to umber rot with one fallen petal tile — flowers withered, "
        "no longer wild."
    ),
    "hermit": (
        "Two parallel ivory mahjong tiles standing close together with a red "
        "X slash across both — pairs forbidden. Simple tile blocks, no numbers."
    ),
    "forest": (
        "A winding river of bamboo tiles rendered as a snaking path of green "
        "rectangles breaking into half-height blocks mid-stream — sequences "
        "choked. Geometric path read."
    ),
    "bureaucrat": (
        "Exactly five identical small ivory tiles in a rigid straight row "
        "locked inside a brass counting frame — must play five. Abacus-like rails."
    ),
    "drunkard": (
        "A tilted sake cup spilling with one bold Arabic numeral 5 tile face "
        "leaning against it — rank fives cursed. Chunky cup, simple 5 glyph."
    ),
    "ash": (
        "A pile of simple-suited tile shapes half buried in gray ash with one "
        "charred corner — simples smothered. Ash facets vs warm ivory tile tops."
    ),
    "furnace": (
        "A brick furnace mouth glowing orange with three terminal-rank tiles "
        "(1/9 stick shapes) silhouetted in the heat — terminals burned. "
        "Readable 1-9 hints without numbers."
    ),
    "relic": (
        "A cracked honor mask / wind plaque snapped in two with dragon curl "
        "motif still visible — honors iconoclast. Lacquer red and gold shards."
    ),
    "blight": (
        "A single mushroom cluster of sickly purple caps with black spore "
        "veins eating into a generic tile silhouette beneath — blight spreads."
    ),
    "hex": (
        "A wax-sealed hexagram paper talisman with one broken chain link "
        "piercing through — relic hexed. Parchment, umber seal, iron chain."
    ),
    "famine": (
        "An empty rice bowl on a scale plate tipped steeply upward with a "
        "towering target bullseye flag behind — doubled hunger for points. "
        "Brass scale, bold target disk."
    ),
    "tempest": (
        "A wall segment made of stacked tile backs with one tile mid-fall and "
        "ember sparks trailing — wall burns after each play. Vertical wall read."
    ),
    "censor": (
        "A heavy red censor stamp square crushing a repeating spiral of tiny "
        "yaku-name ribbons — repeats silenced. Bold red stamp, gray ribbons."
    ),
    "mirror": (
        "A fractured hand mirror reflecting a crossed-out bar chart column — "
        "strongest axis muted. Mirror shards, one bold X over the tallest bar."
    ),
    "counterweight": (
        "A brass balance scale with one pan slammed to the floor and the other "
        "holding a single oversized relic-shaped weight — family countered. "
        "Clear scale silhouette."
    ),
    "tax_collector": (
        "An open ledger book with a stack of gold coins sliding off the edge "
        "into a waiting tax bag — hoard taxed per play. Parchment pages, brass corners."
    ),
    "dragon": (
        "A coiled eastern dragon silhouette forming a ring around a hollow "
        "honorless center — honors required. Bold ruby scales, gold belly facets."
    ),
    "house": (
        "A stacked house of playing-card backs with a brass padlock on the "
        "discard chute — cannot cash in until discards spent. Casino-house dread."
    ),
}

STYLE_BASE = (
    "Stylized low-poly vector encounter icon for a mahjong roguelike set in "
    "a dark curio House. Faceted game UI art — flat color planes with hard "
    "edges between facets, a thin light outline stroke around the silhouette, "
    "and simple cel-shaded highlights. Clean vector-style polygon shading "
    "with crisp edges, readable at 64×64 pixels.\n\n"
    "Composition rules (strict):\n"
    "  • ONE subject only, centered, filling ~85% of the square frame.\n"
    "  • Mild isometric or gentle three-quarter view.\n"
    "  • Solid matte #000000 backdrop outside the subject.\n"
    "  • Subject alone on flat black with only a thin outline stroke.\n"
    "  • Limited palette: walnut browns, brass gold, emerald felt, parchment "
    "ivory, ruby red, jade green, twilight blue shadows.\n"
    "  • Chunky geometric shapes with 3–6 flat color regions per surface.\n"
    "  • Wordless pictogram icon — shapes and colors only.\n"
    "  • Flat cel-shaded color planes with hard facet boundaries.\n\n"
    "Style reference: a wrapped present with faceted green planes, a bold "
    "red bow, and a crisp gold outline — clean vector-like edges, readable "
    "when displayed at 64×64 pixels."
)


@dataclass(frozen=True)
class BossDef:
    slug: str
    name: str
    description: str
    tier: str


def pascal_to_snake(name: str) -> str:
    s1 = re.sub(r"(.)([A-Z][a-z]+)", r"\1_\2", name)
    return re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s1).lower()


def load_boss_kind_order() -> list[str]:
    """Parse `BossKind::ALL` order from src/core/boss.rs."""
    if not BOSS_RS_PATH.exists():
        raise SystemExit(f"Cannot read boss order: {BOSS_RS_PATH} missing")
    text = BOSS_RS_PATH.read_text(encoding="utf-8")
    m = re.search(
        r"pub const ALL: &'static \[BossKind\] = &\[(.*?)\];",
        text,
        re.DOTALL,
    )
    if not m:
        raise SystemExit("Failed to parse BossKind::ALL from boss.rs")
    kinds = re.findall(r"BossKind::(\w+)", m.group(1))
    if not kinds:
        raise SystemExit("BossKind::ALL contained no variants")
    return [pascal_to_snake(k) for k in kinds]


def load_bosses() -> list[BossDef]:
    """Load bosses.json rows in `BossKind::ALL` order."""
    if not BOSS_JSON_PATH.exists():
        raise SystemExit(f"Cannot read bosses: {BOSS_JSON_PATH} missing")
    raw = json.loads(BOSS_JSON_PATH.read_text(encoding="utf-8"))
    by_id = {row["id"]: row for row in raw}
    expected = load_boss_kind_order()
    missing_json = [s for s in expected if s not in by_id]
    if missing_json:
        raise SystemExit(
            "bosses.json missing ids: " + ", ".join(missing_json)
        )
    missing_visuals = [s for s in expected if s not in BOSS_VISUALS]
    if missing_visuals:
        raise SystemExit(
            "BOSS_VISUALS missing entries for: " + ", ".join(missing_visuals)
        )
    out: list[BossDef] = []
    for slug in expected:
        row = by_id[slug]
        out.append(
            BossDef(
                slug=slug,
                name=row["name"],
                description=row["description"],
                tier=row["tier"],
            )
        )
    return out


BOSSES = load_bosses()
LAYOUT = [b.slug for b in BOSSES] + [""]  # 5×5 pad cell


def layout_rows() -> list[list[str]]:
    rows: list[list[str]] = []
    for i in range(0, len(LAYOUT), COLUMNS):
        row = LAYOUT[i : i + COLUMNS]
        while len(row) < COLUMNS:
            row.append("")
        rows.append(row)
    return rows


def build_prompt(boss: BossDef) -> str:
    visual = BOSS_VISUALS[boss.slug]
    accent = BOSS_TIER_ACCENTS.get(
        boss.tier,
        BOSS_TIER_ACCENTS["soft"],
    )
    subject = (
        f'Encounter icon for boss blind "{boss.name}" ({boss.description}): '
        f"{visual}"
    )
    return "\n\n".join([STYLE_BASE, subject, accent])


def pack_atlas(processed_dir: Path, output_dir: Path) -> Path:
    return pack_processed_icons(
        processed_dir,
        output_dir,
        layout=LAYOUT,
        columns=COLUMNS,
        cell_size=CELL_SIZE,
        file_prefix="boss",
    )


def generate_image(
    client,
    prompt: str,
    output_path: Path,
    model: str,
    image_size: str | None,
) -> None:
    img_bytes = generate_image_bytes(
        client,
        prompt,
        model=model,
        aspect_ratio="1:1",
        image_size=image_size,
    )
    output_path.write_bytes(img_bytes)


def process_source(source_path: Path, processed_path: Path) -> None:
    img = Image.open(source_path)
    out = icon_postprocess(
        img,
        cell_size=CELL_SIZE,
        work_px=WORK_PX,
        content_fill=CONTENT_FILL,
    )
    processed_path.parent.mkdir(parents=True, exist_ok=True)
    out.save(processed_path)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate boss encounter icons and pack a sprite atlas"
    )
    parser.add_argument(
        "--boss",
        type=int,
        default=None,
        help="Generate only boss number N (1-indexed, BossKind::ALL order).",
    )
    parser.add_argument(
        "--name",
        type=str,
        default=None,
        help="Generate only the boss with this slug (e.g. drought).",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="List all bosses and exit.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print prompts without calling the API.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Regenerate even if source PNG already exists.",
    )
    parser.add_argument(
        "--pack-only",
        action="store_true",
        help="Skip API calls; post-process existing sources and repack atlas.",
    )
    parser.add_argument(
        "--skip-post",
        action="store_true",
        help="Save raw API output only; do not post-process or pack.",
    )
    parser.add_argument(
        "--model",
        type=str,
        default=DEFAULT_MODEL,
        help=f"Gemini image model (default: {DEFAULT_MODEL}).",
    )
    parser.add_argument(
        "--size",
        type=str,
        default="512px",
        choices=list(GEMINI_IMAGE_SIZES),
        help="API image size tier (default: 512px).",
    )
    parser.add_argument(
        "--output-dir",
        type=str,
        default=None,
        help=f"Output root (default: {OUTPUT_DIR}).",
    )
    parser.add_argument(
        "--delay",
        type=float,
        default=2.0,
        help="Seconds between API calls (default: 2.0).",
    )
    args = parser.parse_args()

    if args.list:
        for i, boss in enumerate(BOSSES, 1):
            print(
                f"  {i:2d}. {boss.name:<22s}  {boss.slug:<18s}  "
                f"[{boss.tier}]  {boss.description}"
            )
        print(f"\nAtlas layout ({COLUMNS} columns):")
        for row in layout_rows():
            print("   ", "  ".join(f"{c or '—':<18s}" for c in row))
        return

    out_dir = Path(args.output_dir) if args.output_dir else OUTPUT_DIR
    source_dir = out_dir / "source"
    processed_dir = out_dir / "processed"
    source_dir.mkdir(parents=True, exist_ok=True)
    processed_dir.mkdir(parents=True, exist_ok=True)

    if args.boss is not None and args.name is not None:
        print("Error: pass --boss OR --name, not both.")
        sys.exit(1)

    if args.boss is not None:
        if args.boss < 1 or args.boss > len(BOSSES):
            print(f"Error: --boss must be between 1 and {len(BOSSES)}")
            sys.exit(1)
        targets = [BOSSES[args.boss - 1]]
    elif args.name is not None:
        match = next((b for b in BOSSES if b.slug == args.name), None)
        if match is None:
            print(f"Error: no boss with slug '{args.name}'. Try --list.")
            sys.exit(1)
        targets = [match]
    else:
        targets = BOSSES

    client = None
    if not args.dry_run and not args.pack_only:
        client = init_client()

    generated = 0
    skipped = 0
    failed = 0
    total_jobs = len(targets)

    for job, boss in enumerate(targets, 1):
        source_path = source_dir / f"boss_{boss.slug}.png"
        processed_path = processed_dir / f"boss_{boss.slug}.png"
        prompt = build_prompt(boss)

        print(f"\n[{job}/{total_jobs}] {boss.name} ({boss.slug})")

        if args.dry_run:
            print(f"  Source:      {source_path.name}")
            print(f"  Processed:   {processed_path.name}")
            print(f"  Prompt:\n    {prompt.replace(chr(10), chr(10) + '    ')}\n")
            continue

        need_api = not args.pack_only
        if need_api:
            if source_path.exists() and not args.force:
                print(f"  Skipping API (exists): {source_path.name}")
                skipped += 1
            else:
                try:
                    assert client is not None
                    generate_image(
                        client, prompt, source_path, args.model, args.size
                    )
                    print(f"  Saved source: {source_path.name}")
                    generated += 1
                except Exception as e:
                    print(f"  Error generating {boss.name}: {e}")
                    failed += 1
                    if job < total_jobs:
                        time.sleep(args.delay)
                    continue
                if job < total_jobs:
                    time.sleep(args.delay)

        if args.skip_post:
            continue

        if not source_path.exists():
            print(f"  Missing source, cannot post-process: {source_path.name}")
            failed += 1
            continue

        try:
            process_source(source_path, processed_path)
            print(f"  Post-processed: {processed_path.name}")
        except Exception as e:
            print(f"  Error post-processing {boss.name}: {e}")
            failed += 1

    if args.dry_run or args.skip_post:
        return

    try:
        pack_atlas(processed_dir, out_dir)
    except Exception as e:
        print(f"\nAtlas pack failed: {e}")
        sys.exit(1)

    print("\nDone.")
    if not args.dry_run:
        print(
            f"  generated={generated}  skipped={skipped}  failed={failed}"
            f"  → {out_dir}"
        )


if __name__ == "__main__":
    main()
