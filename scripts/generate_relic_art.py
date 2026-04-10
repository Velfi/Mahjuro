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

# Shared style prefix injected into every prompt. Tuned to match the existing
# relic art: vintage 1/72 scale model kit box art. Moody oil-painting style,
# surreal industrial/military subjects, aged cardboard framing, fictional
# manufacturer branding in mixed German/Russian/Japanese.
STYLE_PREFIX = (
    "Square image styled as the box lid of a vintage 1/72 scale plastic model "
    "kit from a fictional Eastern-European or Japanese manufacturer, circa "
    "1978. Painted in a moody realistic oil-painting illustration style with "
    "muted, desaturated colors — olive, ochre, teal, rust, dark amber. "
    "The subject is a surreal machine, vehicle, or apparatus painted as if it "
    "were a real buildable scale model — dramatic but plausible, slightly "
    "uncanny. Aged cardboard border with a fictional manufacturer logo in one "
    "corner (cycle between 'ZELKUBO WERKE', 'DRAVUNA-KAI', 'KORVASHI', "
    "'MEKHARI ATELIER'), a '1/72' scale marking, and a product name in mixed "
    "German/Russian/Japanese script. The overall feel is nostalgic, mysterious, "
    "and slightly absurd — like finding a strange model kit at a flea market "
    "that you can't quite identify."
)


# Each tuple: (filename_slug, display_name, visual_description, palette_hint)
# Order and slugs MUST match RelicId::asset_filename in src/core/relic.rs.
RELICS = [
    # ── 15 retuned keepers ────────────────────────────────────────────────
    (
        "triplet_boost",
        "Triplet Boost",
        "A heavy triple-barreled siege mortar on a wheeled wooden carriage, "
        "three stubby barrels bundled together with iron bands. Parked on "
        "muddy cobblestones, dusk sky behind. Baroque-era field weapon feel.",
        "Dark iron barrels, warm brown carriage, ochre mud, amber dusk sky.",
    ),
    (
        "sequence_surge",
        "Sequence Surge",
        "A sleek experimental monorail car on a single elevated rail, three "
        "carriages linked nose to tail, speeding through a foggy valley. "
        "Streamlined 1960s futurism, motion blur on the landscape.",
        "Silver monorail, teal fog, olive hillside, rust rail supports.",
    ),
    (
        "pair_power",
        "Pair Power",
        "Two identical compact steam tractors facing each other, connected "
        "by a heavy tow chain under tension. Both are straining, smokestacks "
        "puffing. A muddy field stretches behind them.",
        "Dark green tractors, black smoke, rust chain, ochre mud.",
    ),
    (
        "honor_fury",
        "Honor Fury",
        "A ceremonial bronze temple bell on a heavy timber frame, mid-strike "
        "from a suspended log ram. Visible shockwave rings emanate from the "
        "bell. Mountain monastery setting, overcast sky.",
        "Oxidized bronze bell, dark timber, grey shockwaves, slate sky.",
    ),
    (
        "red_dragon_rage",
        "Red Dragon Rage",
        "A deep-red experimental rocket sled on a desert salt flat rail, "
        "exhaust flame blasting from twin rear nozzles. Heat shimmer warps "
        "the horizon. Ground-level perspective.",
        "Crimson fuselage, orange exhaust, bleached desert, pale sky.",
    ),
    (
        "green_luck",
        "Green Luck",
        "A dented olive-green Volkswagen Beetle parked alone in a field of "
        "wild grass, one headlight out, a four-leaf clover decal on the door. "
        "Late afternoon golden light. Quietly lucky.",
        "Olive green car, golden grass, warm amber light.",
    ),
    (
        "white_silence",
        "White Silence",
        "A white single-engine biplane parked on a frozen lake, engine off, "
        "propeller still. Perfect silence. Pale overcast sky reflected in "
        "glassy ice. No footprints, no tracks.",
        "Cream white fuselage, pale blue ice, flat grey sky.",
    ),
    (
        "joker_tile",
        "Joker Tile",
        "A peculiar multi-purpose utility vehicle with mismatched parts — "
        "half truck cab, half boat hull, with a crane arm and a small radar "
        "dish. Parked in a junkyard. It shouldn't work but clearly does.",
        "Mismatched rust, olive, and primer grey panels, amber junkyard.",
    ),
    (
        "overflow",
        "Overflow",
        "A massive cylindrical grain silo with its top hatch blown open, "
        "grain cascading down the sides in golden streams. A small conveyor "
        "belt feeds more in at the base. Industrial farmland setting.",
        "Weathered steel silo, golden grain, olive fields, overcast sky.",
    ),
    (
        "quick_draw",
        "Quick Draw",
        "A spring-loaded naval torpedo launcher on a destroyer deck, the "
        "torpedo half-ejected in a freeze-frame moment. Spray of seawater "
        "caught mid-splash. Dramatic side lighting.",
        "Dark steel launcher, brass torpedo, teal ocean spray, grey deck.",
    ),
    (
        "chain_reaction",
        "Chain Reaction",
        "A row of large industrial dominoes — concrete blocks — toppling in "
        "sequence down a factory floor. The first has already fallen, the "
        "last still stands. Dust clouds at each impact point.",
        "Grey concrete blocks, rust floor, amber dust, industrial lighting.",
    ),
    (
        "multiplier_master",
        "Multiplier Master",
        "A tall lattice radio transmission tower on a hilltop, concentric "
        "signal rings radiating outward into a hazy sky. A small control "
        "shed at the base with a single lit window. Remote, powerful.",
        "Dark steel lattice, teal signal rings, olive hill, amber window.",
    ),
    (
        "set_magnet",
        "Set Magnet",
        "A massive electromagnetic crane in a rail yard, its circular magnet "
        "dangling a cluster of steel beams and scrap. A freight train waits "
        "on adjacent tracks. Industrial scale, afternoon haze.",
        "Dark crane arm, rust-orange magnet, steel blue scrap, ochre haze.",
    ),
    (
        "wild_winds",
        "Wild Winds",
        "A small weather station on an exposed coastal cliff, four spinning "
        "anemometer cups blurred by violent wind. The instrument mast bends "
        "slightly. Storm clouds roll in from the sea.",
        "Steel grey mast, spinning chrome cups, dark teal sea, slate clouds.",
    ),
    (
        "dragon_echo",
        "Dragon Echo",
        "A large parabolic acoustic mirror — a concrete listening dish — on "
        "a coastal bluff, aimed out to sea. Three progressively fainter "
        "echo-wave arcs visible in the misty air before it.",
        "Pale concrete dish, teal mist, amber echo arcs, grey-green bluff.",
    ),
    # ── 15 new Patch C relics ─────────────────────────────────────────────
    (
        "shanten_shove",
        "Shanten Shove",
        "A hydraulic ram piston mounted on a factory floor, its chrome shaft "
        "extended mid-push against a heavy steel block. Oil gleams on the "
        "mechanism. Industrial precision, dramatic side-lighting.",
        "Chrome piston, dark steel block, amber oil sheen, concrete floor.",
    ),
    (
        "wall_peek",
        "Wall Peek",
        "A military periscope protruding from a concrete bunker slit, its "
        "twin mirrors catching a sliver of the landscape outside. Overgrown "
        "vegetation creeps over the bunker roof.",
        "Brass periscope tube, grey concrete, olive vegetation, pale sky.",
    ),
    (
        "kan_drum",
        "Kan Drum",
        "A large ceremonial taiko drum on a lacquered stand, four thick "
        "drumsticks arranged in a square on the drumhead. The drum body "
        "is deep red with brass tack rivets. Temple courtyard setting.",
        "Crimson drum body, brass rivets, dark lacquer stand, stone court.",
    ),
    (
        "dora_crown",
        "Dora Crown",
        "An ornate brass astrolabe on a velvet-lined display stand, its "
        "interlocking rings set with a single red glass cabochon at the apex. "
        "A dim collector's study, leather-bound books in background.",
        "Patina brass rings, crimson cabochon, dark velvet, amber lamplight.",
    ),
    (
        "riichi_stick",
        "Riichi Stick",
        "A single ivory baton with a red enamel inlay stripe, resting on a "
        "felt-lined presentation case. Brass clasps on the case. Museum "
        "artifact lighting — single spot from above.",
        "Ivory baton, crimson enamel, dark green felt, brass clasps.",
    ),
    (
        "tenpai_talisman",
        "Tenpai Talisman",
        "A tall narrow signal semaphore arm at a rail junction, locked in the "
        "'clear' position. A single red lantern glows at the pivot. Iron "
        "lattice post, gravel rail bed, dusk sky.",
        "Iron post, crimson lantern, rust semaphore arm, amber dusk.",
    ),
    (
        "river_eraser",
        "River Eraser",
        "A canal lock with its gates half-open, water draining rapidly from "
        "the chamber. The receding waterline leaves dark wet marks on the "
        "stone walls. A small control house sits atop.",
        "Dark stone walls, teal draining water, rust gates, grey sky.",
    ),
    (
        "furiten_ward",
        "Furiten Ward",
        "A medieval pavise shield — tall and rectangular — planted upright in "
        "muddy ground. Its face bears an abstract ward sigil in faded gold "
        "paint. Two crossbow bolts are embedded in it, deflected.",
        "Weathered wood shield, faded gold sigil, rust bolts, ochre mud.",
    ),
    (
        "round_compass",
        "Round Compass",
        "A large ship's binnacle compass on a teak pedestal, its brass gimbal "
        "housing a compass card with ornate wind-rose points. The needle "
        "points firmly east. Teak deck planking beneath.",
        "Polished brass housing, cream compass card, warm teak wood.",
    ),
    (
        "zodiac_pouch",
        "Zodiac Pouch",
        "A leather map case — cylindrical, with a brass cap and shoulder "
        "strap — standing upright with a rolled star chart peeking from the "
        "top. A brass zodiac dial is embossed on the side.",
        "Brown leather, brass cap and dial, cream chart edge, olive strap.",
    ),
    (
        "lunar_almanac",
        "Lunar Almanac",
        "A thick leather-bound nautical almanac lying open on a chart table, "
        "its pages showing printed lunar phase tables and tide charts. A "
        "brass divider compass rests across the gutter.",
        "Dark leather cover, cream pages, brass dividers, amber lamplight.",
    ),
    (
        "yaku_scholar",
        "Yaku Scholar",
        "A small portable field desk — folding mahogany box opened to reveal "
        "ink wells, nibs, and a half-written report. A pair of wire-rimmed "
        "spectacles rests on the paper. Campaign tent backdrop.",
        "Mahogany desk, brass ink wells, cream paper, olive canvas tent.",
    ),
    (
        "eight_treasures",
        "Eight Treasures",
        "An ornate reliquary chest — dark wood with gold filigree — cracked "
        "open to reveal a warm amber glow from within. Eight small objects "
        "are barely visible inside. Cathedral crypt setting.",
        "Dark wood chest, gold filigree, warm amber interior glow, stone.",
    ),
    (
        "kongs_blessing",
        "Kong's Blessing",
        "Four identical artillery shells standing upright in a wooden crate, "
        "perfectly aligned, with a thin halo of light around the group. A "
        "quartermaster's storage room, shelves of supplies behind.",
        "Brass shells, raw wood crate, amber halo, olive-drab shelving.",
    ),
    (
        "codex_compass",
        "Codex Compass",
        "A field surveyor's theodolite on a wooden tripod, its brass telescope "
        "pointing at an angle, with a leather-bound logbook open at the base. "
        "Mountain pass landscape, low clouds.",
        "Brass theodolite, dark wood tripod, cream logbook, slate mountains.",
    ),
    # ── Flower-synergy relics ────────────────────────────────────────────
    (
        "garden_keeper",
        "Garden Keeper",
        "A squat cast-iron greenhouse heater with ornate legs, its chimney "
        "puffing gentle steam. Through the glass panes behind it, tropical "
        "plants press against the fogged glass. Victorian botanical garden.",
        "Dark iron heater, teal glass panes, green foliage, amber steam.",
    ),
    (
        "ikebana",
        "Ikebana",
        "A ceramic kiln — dome-shaped, brick-built — with its firing door "
        "slightly ajar, revealing a warm orange glow inside. Two finished "
        "vases cool on a rack beside it. Rural Japanese pottery workshop.",
        "Rust brick kiln, orange interior glow, cream vases, earth tones.",
    ),
    (
        "hanami",
        "Hanami",
        "A small wooden vendor's cart under a canopy of cherry blossom "
        "branches, petals drifting onto stacked wooden boxes of goods. "
        "Gold-painted price placards lean against the boxes. Spring market.",
        "Warm wood cart, pink petals, gold placards, soft daylight.",
    ),
    # ── 15 new relics ────────────────────────────────────────────────────
    (
        "jade_serpent",
        "Jade Serpent",
        "A narrow armored train car painted dark jade green, with a coiled "
        "serpent insignia on the side. It sits on overgrown tracks in dense "
        "bamboo forest. Vines wrap the undercarriage. Forgotten but intact.",
        "Jade green armor plating, dark bamboo, rust tracks, grey-green vines.",
    ),
    (
        "ink_brush",
        "Ink Brush",
        "A mechanical printing press — the hand-cranked flatbed type — with "
        "a sheet of paper mid-feed showing freshly stamped characters still "
        "glistening wet. Ink rollers gleam. Dim workshop lighting.",
        "Black iron press, dark ink rollers, cream paper, amber lamplight.",
    ),
    (
        "pearl_diver",
        "Pearl Diver",
        "A brass diving helmet — the classic round deep-sea type with small "
        "viewports — sitting on a dock piling, air hose coiled beside it. "
        "Harbour water below, diving barge in background.",
        "Patina brass helmet, dark rubber hose, teal harbour water.",
    ),
    (
        "low_tide",
        "Low Tide",
        "A small coastal survey boat resting on its keel on exposed tidal "
        "mud flats, the waterline far away. Measuring stakes driven into the "
        "mud at intervals. Flat grey estuary light.",
        "Dark hull on brown mud, white measuring stakes, grey flat light.",
    ),
    (
        "merchants_eye",
        "Merchant's Eye",
        "A jeweler's loupe mounted on a small brass articulating arm, clamped "
        "to a watchmaker's bench. Under the lens, the internal gears of a "
        "pocket watch are magnified. Tools scattered around.",
        "Brass loupe and arm, steel gears, dark wood bench, amber light.",
    ),
    (
        "edge_runner",
        "Edge Runner",
        "A narrow-gauge mining locomotive on a precarious cliff-side rail, "
        "the track barely wider than the wheels. Sheer rock face on one side, "
        "deep gorge on the other. Dramatic vertigo perspective.",
        "Dark iron locomotive, rust narrow rail, grey cliff, teal gorge.",
    ),
    (
        "lucky_seven",
        "Lucky Seven",
        "A vintage slot machine — the three-reel mechanical type — showing "
        "triple sevens in the window. A single brass lever on the side. "
        "Sitting alone on a green baize table in a dim room.",
        "Chrome and brass machine, cherry-red sevens, green baize, dim amber.",
    ),
    (
        "momentum",
        "Momentum",
        "A Newton's cradle — five steel balls on wire frames — captured at "
        "the moment of impact, the end ball swinging out with motion blur. "
        "Sits on a polished mahogany desk. Executive office setting.",
        "Chrome steel balls, dark wire frame, warm mahogany, amber light.",
    ),
    (
        "minimalist",
        "Minimalist",
        "A single-room concrete observation post on a flat empty plain — "
        "just a slit window and a steel door, nothing else. Perfectly "
        "geometric. Vast empty sky. Extreme negative space.",
        "Raw concrete, steel door, flat ochre plain, enormous pale sky.",
    ),
    (
        "turtle_shell",
        "Turtle Shell",
        "A compact armored personnel carrier with an unusually domed, "
        "turtle-like hull and small viewports. Parked behind a sandbag wall, "
        "hatches sealed. Defensive posture, not built for speed.",
        "Olive-drab domed hull, dark viewports, tan sandbags, grey sky.",
    ),
    (
        "closed_gate",
        "Closed Gate",
        "A heavy blast door in a concrete dam wall — circular, submarine-"
        "style, with a spoked locking wheel. Fully sealed. Water stains "
        "streak the concrete above. Industrial, impassable.",
        "Steel blast door, raw concrete, rust water stains, amber light.",
    ),
    (
        "gold_furnace",
        "Gold Furnace",
        "A small cupellation furnace — brick-built, dome-topped — with its "
        "front grate open showing a crucible of molten gold inside, glowing "
        "intensely. Tongs and ingot molds nearby. Assay office setting.",
        "Red brick furnace, bright gold molten glow, dark iron tools.",
    ),
    (
        "snowball",
        "Snowball",
        "A large steel ball-bearing rolling down a factory ramp, picking up "
        "smaller ball-bearings that stick to it magnetically as it goes. "
        "Growing noticeably larger toward the bottom. Assembly-line setting.",
        "Chrome steel ball, smaller bearings, grey ramp, industrial green.",
    ),
    (
        "second_wind",
        "Second Wind",
        "A wind-up tin toy soldier with its key being turned for a second "
        "time — hand visible on the key. The soldier is mid-march, one "
        "foot forward. Scuffed paint shows it's been wound before.",
        "Olive tin soldier, brass wind-up key, scuffed paint, wood floor.",
    ),
    (
        "glass_cannon",
        "Glass Cannon",
        "A field howitzer made entirely of blown glass — barrel, carriage, "
        "wheels — beautiful but visibly fragile, hairline cracks catching "
        "the light. An oversized brass shell sits beside it, ready to load.",
        "Translucent blue-tinted glass, brass shell, hairline crack glints.",
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
