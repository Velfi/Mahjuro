#!/usr/bin/env python3
"""
Generate relic icon art for Mahjuro using OpenAI's image generation API.

The relic list is kept in lockstep with `RelicId` in src/core/relic.rs.
Style direction: "Midnight Gold" — bold flat-color cartoon icons with thick
black outlines, sitting on a deep indigo radial vignette with warm gold rim
light. Every icon should drop onto the gameplay background without looking
like a sticker from a different game.

Usage:
    pip install openai requests
    export OPENAI_API_KEY="sk-..."
    python scripts/generate_relic_art.py                  # all missing relics
    python scripts/generate_relic_art.py --force          # regenerate all
    python scripts/generate_relic_art.py --relic 17       # one relic by index
    python scripts/generate_relic_art.py --name kan_drum  # one relic by slug
    python scripts/generate_relic_art.py --list           # list all relics
    python scripts/generate_relic_art.py --dry-run        # print prompts only
"""

import argparse
import base64
import os
import sys
import time
from pathlib import Path

try:
    from openai import OpenAI
except ImportError:
    print("Error: openai package not installed. Run: pip install openai")
    sys.exit(1)


OUTPUT_DIR = Path(__file__).resolve().parent.parent / "assets" / "relics"

# Shared style prefix injected into every prompt. Tuned to "Midnight Gold":
# cool indigo backdrop + warm gold rim, bold flat cartoon icons that match
# the existing relic art voice while sitting cleanly on the game's background.
STYLE_PREFIX = (
    "Square icon for a mahjong roguelite video game in a 'Midnight Gold' art "
    "direction. Bold flat colors with thick black outlines, 2-4 accent colors, "
    "slight cartoon personality (objects look mildly self-aware, deadpan). "
    "Subject is centered with a clean readable silhouette. "
    "Background is a soft deep-indigo radial vignette (#0a1422 core fading to "
    "near-black at the corners) with a warm gold rim-light glow behind the "
    "subject — NOT a sepia or khaki backdrop. "
    "No text, no letters, no numbers, no logos, no borders or frames. "
    "Painted as a single icon, not on a card."
)


# Each tuple: (filename_slug, display_name, visual_description, palette_hint)
# Order and slugs MUST match RelicId::asset_filename in src/core/relic.rs.
RELICS = [
    # ── 15 retuned keepers ────────────────────────────────────────────────
    (
        "triplet_boost",
        "Triplet Boost",
        "Three identical ivory mahjong tiles stacked and bound together with a "
        "gold ribbon, the top tile peering out with a slightly nervous expression. "
        "A bold gold multiplication cross hovers above the stack.",
        "Ivory tiles, gold ribbon and cross, deep indigo backdrop.",
    ),
    (
        "sequence_surge",
        "Sequence Surge",
        "Three mahjong tiles numbered 1-2-3 riding a forked gold lightning bolt "
        "like a surfboard. The middle tile looks bored and unimpressed; the "
        "outer two are wide-eyed.",
        "Ivory tiles, bright gold lightning, indigo sky.",
    ),
    (
        "pair_power",
        "Pair Power",
        "Two ivory mahjong tiles aggressively fist-bumping. Gold impact lines "
        "and a small spark burst radiate from the contact point. Both tiles "
        "wear matching tiny red sweatbands.",
        "Ivory tiles, red sweatbands, gold impact burst.",
    ),
    (
        "honor_fury",
        "Honor Fury",
        "A single mahjong honor tile with an angry furrowed expression, gold "
        "steam jetting from its top, tiny crack lines splintering from its base. "
        "It is mid-yell.",
        "Ivory tile face, dark red character glyph, gold steam.",
    ),
    (
        "red_dragon_rage",
        "Red Dragon Rage",
        "A red mahjong dragon tile completely engulfed in stylized flames, "
        "calmly sipping from a tiny teacup with its eyes closed. 'This is fine' "
        "energy. The flames curl upward in flat shapes.",
        "Crimson dragon glyph, orange and gold flames, ivory teacup.",
    ),
    (
        "green_luck",
        "Green Luck",
        "A four-leaf clover where one leaf is clearly wilting and patched up "
        "with a strip of beige tape. The clover has a deadpan face. A small "
        "gold coin floats next to it with a plus sign.",
        "Green clover, beige tape, gold coin.",
    ),
    (
        "white_silence",
        "White Silence",
        "A white mahjong dragon tile (blank face) wearing oversized matte-black "
        "noise-canceling headphones, eyes closed in serene bliss. Tiny pale "
        "blue snowflakes drift around it.",
        "Ivory tile, matte black headphones, pale blue snowflakes.",
    ),
    (
        "joker_tile",
        "Joker Tile",
        "An ivory mahjong tile wearing a comically oversized fake black "
        "mustache and a tilted tiny red jester hat with a gold bell. A single "
        "gold question mark on its face.",
        "Ivory tile, black mustache, red hat, gold bell and question mark.",
    ),
    (
        "overflow",
        "Overflow",
        "A wooden bucket tipped sideways with mahjong tiles cascading out in "
        "an arc. The bucket has a small exasperated face on its side. A few "
        "tiles in the air still have surprised expressions.",
        "Warm brown bucket, ivory tiles, indigo backdrop.",
    ),
    (
        "quick_draw",
        "Quick Draw",
        "A single mahjong tile dressed as a cowboy — tan ten-gallon hat, "
        "leather holster belt — mid-quickdraw with a tile in each tiny hand. "
        "A small dust puff at its base.",
        "Tan hat, brown holster, ivory tile, sandy dust.",
    ),
    (
        "chain_reaction",
        "Chain Reaction",
        "A line of ivory mahjong tiles toppling like dominoes. The first tile "
        "leans back with arms crossed, smug, watching the chain fall. Small "
        "gold star bursts at each impact point.",
        "Ivory tiles, gold impact stars, indigo shadow.",
    ),
    (
        "multiplier_master",
        "Multiplier Master",
        "A single mahjong tile wearing a black square graduation cap with a "
        "gold tassel and tiny round spectacles, holding a small dark green "
        "chalkboard covered in white multiplication crosses. Looks tired but "
        "proud.",
        "Black cap, gold tassel, green chalkboard, ivory tile.",
    ),
    (
        "set_magnet",
        "Set Magnet",
        "A classic red and silver horseshoe magnet crackling with gold energy "
        "arcs, pulling a startled ivory mahjong tile through the air toward it. "
        "Motion lines trail behind the tile.",
        "Red and silver magnet, gold arcs, ivory tile.",
    ),
    (
        "wild_winds",
        "Wild Winds",
        "Four mahjong wind tiles caught spinning in a small swirling vortex, "
        "their directional symbols blurred mid-swap. All four tiles have "
        "spiral-eye expressions.",
        "Indigo vortex, ivory tiles, gold motion lines.",
    ),
    (
        "dragon_echo",
        "Dragon Echo",
        "A red dragon mahjong tile shouting toward the right edge of the "
        "frame, with three progressively smaller and more faded copies of "
        "itself bouncing back as echoes. Tiny gold sound notes float around.",
        "Crimson dragon, faded red echoes, gold notes.",
    ),
    # ── 15 new Patch C relics ─────────────────────────────────────────────
    (
        "shanten_lens",
        "Shanten Lens",
        "An antique brass magnifying glass tilted over a single mahjong tile, "
        "with faint geometric guide lines and a tiny gold reticle drawn across "
        "the lens. The tile underneath looks slightly self-conscious.",
        "Brass magnifier, ivory tile, gold reticle lines.",
    ),
    (
        "wall_peek",
        "Wall Peek",
        "A stack of mahjong tiles forming a small wall. Two tiles in the "
        "middle have been pushed forward slightly, revealing a single curious "
        "eye peeking through the gap from behind the wall.",
        "Ivory wall, single dark peeking eye, gold highlight.",
    ),
    (
        "kan_drum",
        "Kan Drum",
        "A short, wide taiko-style drum with a deep red body and gold rim, "
        "four ivory mahjong tiles arranged in a square on top of the drumhead. "
        "Two crossed wooden mallets float above. Small impact rings ripple out.",
        "Crimson drum body, gold rim, ivory tiles, brown mallets.",
    ),
    (
        "dora_crown",
        "Dora Crown",
        "A small ornate gold crown with five points, a single faceted red gem "
        "set in the center, resting at a slight tilt on top of one ivory "
        "mahjong tile. Tiny gold sparkles drift up.",
        "Gold crown, red gem, ivory tile.",
    ),
    (
        "riichi_stick",
        "Riichi Stick",
        "A traditional white riichi betting stick laid horizontally, with a "
        "single bright red dot in its center, a faint gold glow underneath. "
        "Two tiny gold tassels dangle from one end.",
        "White stick, red dot, gold tassels and glow.",
    ),
    (
        "tenpai_talisman",
        "Tenpai Talisman",
        "A vertical paper ofuda talisman strip with a thick red border and a "
        "single bold gold seal stamp in the middle (abstract sigil shape, NOT "
        "a real character). A short red string hangs from the top.",
        "Cream paper, red border, gold sigil seal.",
    ),
    (
        "river_eraser",
        "River Eraser",
        "A pink rubber eraser with a deadpan face, mid-erase, sweeping a "
        "trail of three ghostly fading mahjong tiles into nothing. Tiny eraser "
        "shavings curl off behind it.",
        "Pink eraser, ivory fading tiles, grey shavings.",
    ),
    (
        "furiten_ward",
        "Furiten Ward",
        "A small round shield with a thick gold rim and a deep indigo face, "
        "stamped with an abstract gold barrier sigil. A faint translucent "
        "force-field bubble surrounds it. A single red mahjong tile is "
        "deflecting off the shield.",
        "Gold-rimmed shield, indigo face, red tile, gold sigil.",
    ),
    (
        "round_compass",
        "Round Compass",
        "A small ornate brass compass with four cardinal points labeled by "
        "tiny mahjong wind tiles instead of letters. The needle is a stylized "
        "red dragon arrow pointing east. Closed lid hangs to the side.",
        "Brass compass body, ivory wind tiles, red needle.",
    ),
    (
        "zodiac_pouch",
        "Zodiac Pouch",
        "A small drawstring leather pouch with a gold zodiac star sigil "
        "embroidered on the front, slightly open at the top with a single "
        "glowing card edge poking out. A gold cord ties the neck.",
        "Brown leather pouch, gold sigil and cord, glowing card edge.",
    ),
    (
        "lunar_almanac",
        "Lunar Almanac",
        "An old leather-bound book lying open. The two visible pages show "
        "abstract crescent moon phases and tiny constellation dots in gold "
        "ink. A small gold ribbon bookmark hangs from the spine.",
        "Indigo leather, cream pages, gold moons and stars.",
    ),
    (
        "yaku_scholar",
        "Yaku Scholar",
        "A single mahjong tile wearing a tiny scholar's mortarboard cap with "
        "a red tassel, holding up a small scroll with one hand. Wears tiny "
        "round wire glasses. Earnest expression.",
        "Black cap, red tassel, cream scroll, ivory tile.",
    ),
    (
        "eight_treasures",
        "Eight Treasures",
        "An open ornate gold-rimmed treasure chest overflowing with mahjong "
        "tiles and gold coins, a few zodiac star tokens floating up out of it "
        "with little sparkle trails. The chest has a tiny smug face on its "
        "lock.",
        "Gold-rimmed chest, ivory tiles, gold coins, indigo interior glow.",
    ),
    (
        "kongs_blessing",
        "Kong's Blessing",
        "Four ivory mahjong tiles arranged in a tight square formation, "
        "wreathed by a soft halo of gold light. A pair of small folded hands "
        "in a blessing gesture hovers above the tiles. Reverent, calm.",
        "Ivory tiles, gold halo, warm cream hands.",
    ),
    (
        "codex_compass",
        "Codex Compass",
        "A small open book with a brass compass embedded into the right page, "
        "the needle pointing diagonally up. The left page shows abstract "
        "swirling gold sigils. A red ribbon bookmark trails out the bottom.",
        "Indigo book, brass compass, gold sigils, red ribbon.",
    ),
]


def build_prompt(visual: str, palette: str) -> str:
    """Combine the shared style prefix with the relic-specific description."""
    return f"{STYLE_PREFIX}\n\nSubject: {visual}\n\nColor palette: {palette}"


def generate_image(
    client: OpenAI, prompt: str, output_path: Path, model: str, size: str
) -> None:
    """Call the image API and save the resulting PNG."""
    response = client.images.generate(
        model=model,
        prompt=prompt,
        n=1,
        size=size,
        quality="high",
    )

    data = response.data[0]
    image_url = getattr(data, "url", None)
    if image_url is None:
        b64 = getattr(data, "b64_json", None)
        if b64 is None:
            print("  Error: No image URL or b64 data returned.")
            return
        img_bytes = base64.b64decode(b64)
    else:
        import requests

        img_response = requests.get(image_url, timeout=120)
        img_response.raise_for_status()
        img_bytes = img_response.content

    output_path.write_bytes(img_bytes)
    print(f"  Saved: {output_path}")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate Mahjuro relic art via the OpenAI image API"
    )
    parser.add_argument(
        "--relic",
        type=int,
        default=None,
        help="Generate only relic number N (1-indexed). Omit for all.",
    )
    parser.add_argument(
        "--name",
        type=str,
        default=None,
        help="Generate only the relic with this filename slug (e.g. kan_drum).",
    )
    parser.add_argument(
        "--list", action="store_true", help="List all relics and exit."
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print prompts without calling the API.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Regenerate even if the output file already exists.",
    )
    parser.add_argument(
        "--model",
        type=str,
        default="gpt-image-1",
        help="Image model to use (default: gpt-image-1).",
    )
    parser.add_argument(
        "--size",
        type=str,
        default="1024x1024",
        help="Image size (default: 1024x1024).",
    )
    parser.add_argument(
        "--output-dir",
        type=str,
        default=None,
        help=f"Output directory (default: {OUTPUT_DIR}).",
    )
    parser.add_argument(
        "--delay",
        type=float,
        default=2.0,
        help="Seconds to sleep between API calls (default: 2.0).",
    )
    args = parser.parse_args()

    if args.list:
        for i, (slug, name, _, _) in enumerate(RELICS, 1):
            print(f"  {i:2d}. {name:<22s}  ({slug}.png)")
        return

    out_dir = Path(args.output_dir) if args.output_dir else OUTPUT_DIR
    out_dir.mkdir(parents=True, exist_ok=True)

    # Select which relics to generate.
    if args.relic is not None and args.name is not None:
        print("Error: pass --relic OR --name, not both.")
        sys.exit(1)

    if args.relic is not None:
        if args.relic < 1 or args.relic > len(RELICS):
            print(f"Error: --relic must be between 1 and {len(RELICS)}")
            sys.exit(1)
        targets = [(args.relic - 1, RELICS[args.relic - 1])]
    elif args.name is not None:
        match = next(
            ((i, r) for i, r in enumerate(RELICS) if r[0] == args.name), None
        )
        if match is None:
            print(f"Error: no relic with slug '{args.name}'. Try --list.")
            sys.exit(1)
        targets = [match]
    else:
        targets = list(enumerate(RELICS))

    client = None
    if not args.dry_run:
        api_key = os.environ.get("OPENAI_API_KEY")
        if not api_key:
            print("Error: OPENAI_API_KEY environment variable not set.")
            sys.exit(1)
        client = OpenAI(api_key=api_key)

    generated = 0
    skipped = 0
    failed = 0

    for idx, (slug, name, visual, palette) in targets:
        prompt = build_prompt(visual, palette)
        output_path = out_dir / f"{slug}.png"

        print(f"\n[{idx + 1}/{len(RELICS)}] {name}")

        if args.dry_run:
            print(f"  Prompt:\n    {prompt}\n")
            continue

        if output_path.exists() and not args.force:
            print(f"  Skipping (exists): {output_path.name}  — use --force to regenerate")
            skipped += 1
            continue

        try:
            assert client is not None
            generate_image(client, prompt, output_path, args.model, args.size)
            generated += 1
        except Exception as e:
            print(f"  Error generating {name}: {e}")
            failed += 1
            continue

        if len(targets) > 1:
            time.sleep(args.delay)

    print("\nDone.")
    if not args.dry_run:
        print(
            f"  generated={generated}  skipped={skipped}  failed={failed}"
            f"  → {out_dir}"
        )


if __name__ == "__main__":
    main()
