#!/usr/bin/env python3
"""
Generate relic **source** art for Mahjuro via the OpenAI image API.

Relic list matches `RelicId` / `asset_filename` in `src/core/relic.rs`. Art
direction: enamel-pin badges (readable silhouette, metal rims, enamel fills).

**Writes (under `assets/textures/relics/source/` by default)**

  • `{slug}_object.png` — RGBA color render (transparent background). Fallback
    albedo if `derive` has not produced `relics/{slug}.png` yet; see
    `src/render/relic_pipeline.rs`.
  • `{slug}_height.png` — grayscale **relief guide**. At runtime this path is
    the linear height / `relief_tex` bind (same stem as `RelicId::source_heightmap_path`).
  • `{slug}_mask.png` — binary silhouette (L-mode PNG) derived from the cleaned
    object alpha. Matches `RelicId::source_mask_path` and feeds the mesh
    extrusion path in `src/render/relic_pipeline.rs`. Rewritten whenever the
    object is (re)generated or re-cleaned. Disable with `--skip-mask`.

**Not written here**:

  • `assets/textures/relics/{slug}.png` — derived runtime albedo (preferred in-game).

The game’s load order and fallbacks live in `src/render/relic_pipeline.rs`.

The pipeline is non-destructive: each file on disk is a single-write
artifact. Objects ship with the seamless-paper backdrop the model rendered.
Runtime alpha cutting is done by the Rust loader, which composites the mask
onto the object's alpha channel at decode time. Re-running `--artifact mask`
with `--force` regenerates the mask from the height map without ever
touching the object.

Usage:
    pip install openai requests pillow
    export OPENAI_API_KEY="sk-..."
    python scripts/generate_relic_art.py                       # all missing source assets
    python scripts/generate_relic_art.py --artifact object     # only object renders
    python scripts/generate_relic_art.py --artifact height     # only relief/height sources
    python scripts/generate_relic_art.py --artifact mask       # only rewrite masks from existing heights
    python scripts/generate_relic_art.py --artifact both --name overflow
    python scripts/generate_relic_art.py --force               # regenerate all
    python scripts/generate_relic_art.py --relic 17            # one relic by index
    python scripts/generate_relic_art.py --name kan_drum       # one relic by slug
    python scripts/generate_relic_art.py --list                # list all relics
    python scripts/generate_relic_art.py --dry-run             # print prompts only
"""

import argparse
import base64
import os
import re
import sys
import tempfile
import time
from pathlib import Path

try:
    from openai import OpenAI
except ImportError:
    print("Error: openai package not installed. Run: pip install openai")
    sys.exit(1)


OUTPUT_DIR = (
    Path(__file__).resolve().parent.parent
    / "assets"
    / "textures"
    / "relics"
    / "source"
)

# Shared style description injected into every prompt. Tuned for isolated
# cloisonné-enamel-pin relic renders that can be reviewed directly and fed
# into silhouette / relief derivation.
#
# The core describes construction, material, and lighting in metal-agnostic
# terms. A per-rarity METAL_PROFILE is appended so Common/Uncommon/Rare/
# Legendary pins read as Iron/Copper/Silver/Gold — matching the canonical
# mapping in src/core/relic.rs (see material_for_rarity).
STYLE_CORE = (
    "A single isolated collectible cloisonné enamel pin relic rendered as a "
    "hero badge for a game asset pipeline. Front-facing near-orthographic "
    "presentation, pin plane parallel to the camera, centered inside a "
    "rounded-square bezel with a raised polished outer frame and a stepped "
    "inner lip.\n\n"
    "Construction: every color region is a champlevé cell recessed slightly "
    "below raised cloisonné wires. The wires have visible cross-section, "
    "catch a crisp specular highlight along their top edge, and cast a "
    "hairline shadow down into the enamel below. Enamel fills sit at a "
    "consistent recessed depth across the whole pin.\n\n"
    "Material: vitreous glass enamel with faint subsurface depth — light "
    "enters each fill, bounces off the polished metal substrate beneath, and "
    "returns slightly desaturated toward the center of the cell, giving a "
    "jewel-like inner glow. Strong silhouette readability at game-camera "
    "scale; proportions stay clean under the near-orthographic camera."
)


# Per-rarity metal profile. Keys match src/core/relic.rs Rarity variants.
# The bezel, cloisonné wires, and any negative-space substrate all read as
# this metal; only the enamel fills vary per badge.
METAL_PROFILES = {
    "Common": (
        "Metal tier (Common — Iron): the wire inlay and bezel read as "
        "blackened wrought iron with a subtle hammered texture along the "
        "outer frame. Highlights are cool steely white; negative-space "
        "substrate shows as dark gunmetal with a soft brushed grain. Wire "
        "tops catch a narrow hard specular line."
    ),
    "Uncommon": (
        "Metal tier (Uncommon — Copper): the wire inlay and bezel read as "
        "polished rose copper with a warm amber patina settling into "
        "recesses. Highlights are warm peach-white; negative-space substrate "
        "shows as burnished copper with a soft radial brush. Wire tops hold "
        "a long warm specular roll."
    ),
    "Rare": (
        "Metal tier (Rare — Silver): the wire inlay and bezel read as "
        "polished sterling silver with a cool white sheen. Highlights are "
        "bright cool-white; negative-space substrate shows as brushed silver "
        "with a faint radial burnish. Wire tops catch a crisp cool specular, "
        "and enamel cells pick up a subtle cool reflection near the wire "
        "edges."
    ),
    "Legendary": (
        "Metal tier (Legendary — Gold): the wire inlay and bezel read as "
        "polished jeweler's gold with a warm buttery tone. Highlights are "
        "warm ivory-gold; negative-space substrate shows as brushed gold "
        "with a soft radial burnish warming from the center outward. Wire "
        "tops hold a long luminous specular, and enamel cells carry a faint "
        "gold reflection along their inner edges."
    ),
}


def style_prefix(rarity: str) -> str:
    """Compose the full style description for a given rarity tier."""
    profile = METAL_PROFILES.get(rarity)
    if profile is None:
        raise SystemExit(
            f"Unknown rarity '{rarity}'. Expected one of: "
            f"{', '.join(METAL_PROFILES)}."
        )
    return f"{STYLE_CORE}\n\n{profile}"


RELIC_RS_PATH = (
    Path(__file__).resolve().parent.parent / "src" / "core" / "relic.rs"
)


def load_slug_to_rarity() -> dict:
    """Parse src/core/relic.rs for the authoritative slug → rarity mapping.

    Joins two match tables from the Rust source:
      - `asset_filename` arms: `RelicId::TripletBoost => "triplet_boost.png"`
      - `all_relic_defs` entries: `id: RelicId::TripletBoost, ... rarity: Rarity::Common`

    Returns `{ "triplet_boost": "Common", ... }`. Fails loud if relic.rs is
    missing or a RelicId appears in one table but not the other — drift between
    the script and the game must be surfaced, not silently defaulted.
    """
    if not RELIC_RS_PATH.exists():
        raise SystemExit(
            f"Cannot read rarity map: {RELIC_RS_PATH} does not exist."
        )
    text = RELIC_RS_PATH.read_text()

    id_to_slug = {
        m.group(1): m.group(2)
        for m in re.finditer(
            r'RelicId::(\w+)\s*=>\s*"([a-z0-9_]+)\.png"', text
        )
    }

    id_to_rarity = {}
    for m in re.finditer(
        r"id:\s*RelicId::(\w+)\s*,[^}]*?rarity:\s*Rarity::(\w+)",
        text,
        flags=re.DOTALL,
    ):
        id_to_rarity[m.group(1)] = m.group(2)

    if not id_to_slug or not id_to_rarity:
        raise SystemExit(
            "Failed to parse relic.rs — asset_filename arms or "
            "all_relic_defs entries did not match expected shape."
        )

    slug_to_rarity = {}
    for relic_id, slug in id_to_slug.items():
        rarity = id_to_rarity.get(relic_id)
        if rarity is None:
            # RelicId has an asset filename but no RelicDef — harmless (the
            # script only generates for slugs in RELICS), so skip silently.
            continue
        slug_to_rarity[slug] = rarity
    return slug_to_rarity


SLUG_TO_RARITY = load_slug_to_rarity()


def rarity_for(slug: str) -> str:
    """Look up the rarity tier for a relic slug, failing loud on drift."""
    rarity = SLUG_TO_RARITY.get(slug)
    if rarity is None:
        raise SystemExit(
            f"Relic slug '{slug}' has no rarity entry in relic.rs. "
            "Add a RelicDef or remove it from the RELICS list."
        )
    return rarity


# Each tuple: (filename_slug, display_name, visual_description, palette_hint)
# Order and slugs MUST match RelicId::asset_filename in src/core/relic.rs.
RELICS = [
    (
        "triplet_boost",
        "Triplet Boost",
        "Three identical mahjong tiles standing upright in a straight "
        "horizontal row on an emerald felt table, with crackling gold "
        "lightning arcing tile-to-tile between their faces in chained bolts. "
        "A few stray tiles lie blurred in the background.",
        "Ivory tile faces, deep emerald felt, crackling gold lightning, muted crimson marks.",
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
        "Two identical mahjong tiles standing upright side by side on a felt "
        "table, touching at the edge, with a subtle shockwave ring rippling "
        "outward from the seam between them. A few stray tiles lie blurred "
        "in the background.",
        "Ivory tile faces, deep emerald felt, warm gold shockwave, muted bamboo green marks.",
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
        "A lone revolver mid-unholster from a worn leather gun belt, barrel "
        "already clearing the lip of the holster in a freeze-frame blur. A "
        "single brass cartridge glints in the foreground. Dusty saloon plank "
        "floor, low raking sunlight through slats.",
        "Blued steel revolver, tan leather belt, brass cartridge, warm dusty amber light.",
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
        "A chunk of raw magnetite ore resting on a felt table, with one "
        "mahjong tile lifting free from a row of face-down wall tiles and "
        "floating toward three matching tiles already gathered beside the "
        "stone. Faint iron filings cling to the ore's crystalline facets.",
        "Blue-black magnetite, silver-grey filings, ivory tile faces, deep emerald felt.",
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
    (
        "shanten_shove",
        "Shanten Shove",
        "A hand of mahjong tiles standing in a neat row on a felt table, "
        "with one extra tile being nudged forward into the last open slot "
        "by an unseen force — a faint push-line trailing behind it. The "
        "arrangement reads as one tile away from complete.",
        "Ivory tile faces, deep emerald felt, warm gold push line, muted ink markings.",
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
        "A folded paper omamori charm with a silk tassel and drawstring, its "
        "face stamped with a single mahjong tile glyph surrounded by a ring "
        "of small waiting-tile icons, as if the final tile is about to arrive.",
        "Deep indigo silk, gold ink stamp, ivory tile glyph, crimson tassel cord.",
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
        "lunar_almanac",
        "Lunar Almanac",
        "A thick leather-bound nautical almanac lying open on a chart table, "
        "its pages showing printed lunar phase tables and tide charts. A "
        "brass divider compass rests across the gutter.",
        "Dark leather cover, cream pages, brass dividers, amber lamplight.",
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
        "Four identical mahjong tiles stood upright side by side on an altar "
        "cloth, perfectly aligned, with a bright ring of golden light arcing "
        "overhead and thin incense smoke curling up past the tiles.",
        "Ivory tile faces, deep vermilion altar cloth, bright gold halo, soft incense haze.",
    ),
    (
        "codex_compass",
        "Codex Compass",
        "A field surveyor's theodolite on a wooden tripod, its brass telescope "
        "pointing at an angle, with a leather-bound logbook open at the base. "
        "Mountain pass landscape, low clouds.",
        "Brass theodolite, dark wood tripod, cream logbook, slate mountains.",
    ),
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
        "A shallow ceramic suiban vase holding a minimalist flower "
        "arrangement — a single upright branch of pine, a curved iris stem, "
        "and one white chrysanthemum — balanced on a tokonoma alcove shelf.",
        "Pale celadon vase, dark pine, violet iris, white bloom, muted tatami backdrop.",
    ),
    (
        "hanami",
        "Hanami",
        "A small wooden vendor's cart under a canopy of cherry blossom "
        "branches, petals drifting onto stacked wooden boxes of goods. "
        "Gold-painted price placards lean against the boxes. Spring market.",
        "Warm wood cart, pink petals, gold placards, soft daylight.",
    ),
    (
        "jade_serpent",
        "Jade Serpent",
        "A jade-green snake coiled around a bundle of bamboo stalks, its "
        "scales formed from tiny mahjong tile faces. A simple terrarium "
        "display with moss and pebbles. Soft diffused daylight. Cheerful, "
        "slightly off — the snake has too-large friendly eyes.",
        "Cream background, jade green snake, bamboo greens, muted moss and pebble accents.",
    ),
    (
        "red_serpent",
        "Red Serpent",
        "A red snake coiled around a single Chinese mahjong character tile, "
        "its scales formed from tiny mahjong tile faces. A simple terrarium "
        "display with moss and pebbles. Soft diffused daylight. Cheerful, "
        "slightly off — the snake has too-large friendly eyes.",
        "Cream background, crimson snake, ivory tile with dark ink character, muted moss and pebble accents.",
    ),
    (
        "blue_serpent",
        "Blue Serpent",
        "A blue snake coiled around a single blue mahjong circles/dots tile, "
        "its scales formed from tiny mahjong tile faces. A simple terrarium "
        "display with moss and pebbles. Soft diffused daylight. Cheerful, "
        "slightly off — the snake has too-large friendly eyes.",
        "Cream background, cobalt blue snake, ivory tile with blue dot pips, muted moss and pebble accents.",
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
        "A stack of mahjong tiles tipping forward like falling dominoes, "
        "with the last few tiles in the chain already airborne and trailing "
        "bright motion streaks. Energy visibly accumulates toward the leading "
        "edge of the cascade.",
        "Ivory tile faces, deep ink markings, warm gold motion streaks, dark felt underneath.",
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
        "A hefty snowball mid-roll down a pine-shadowed hillside, its surface "
        "crusted with bark flecks and pebbles gathered on the way. A widening "
        "track carves through deep powder behind it.",
        "Bright packed snow, blue shadows, dark pines, scattered debris, crisp winter light.",
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
    (
        "last_breath",
        "Last Breath",
        "A fading candle flame enclosed in a narrow metal lantern, with a "
        "single bright final flare bursting from the wick. The shape should "
        "feel like a final-chance omen rendered as a crisp enamel badge.",
        "Warm brass body, ivory flame core, ember orange highlights, deep soot accents.",
    ),
    (
        "tile_polisher",
        "Tile Polisher",
        "A mahjong tile held upright on a soft cloth, its face buffed to a "
        "mirror gloss with a bright crescent highlight and a small cluster "
        "of sparkle glints, flanked by a folded polishing rag and a tiny "
        "bottle of wax.",
        "Pearl ivory tile, warm beige cloth, soft gold sparkles, deep amber wax bottle.",
    ),
    (
        "paper_lantern",
        "Paper Lantern",
        "A round ribbed paper lantern glowing softly from within, suspended "
        "from a small metal hook. Delicate but lucky, like a festival charm.",
        "Warm cream paper, amber inner glow, muted red tassel, dark bronze hook.",
    ),
    (
        "iron_lantern",
        "Iron Lantern",
        "A sturdy iron lantern with a protected inner flame and thick cage "
        "bars, shaped like a tougher successor to a paper lantern.",
        "Dark iron frame, warm gold flame, soot black accents, dim amber glow.",
    ),
    (
        "mirror_tile",
        "Mirror Tile",
        "A mahjong tile paired with a polished circular mirror inset, the "
        "reflective face framed by a crisp geometric border. Symmetrical, "
        "iconic, and badge-readable.",
        "Ivory tile body, bright silver mirror, muted jade details, pale blue reflections.",
    ),
    (
        "way_of_purity",
        "Way of Purity",
        "Three identical suited mahjong tiles arranged in a tight vertical "
        "stack, each bearing the same clean numeric mark, haloed by a thin "
        "luminous ring that signals a flawless single-suit hand.",
        "Crisp ivory tile faces, deep bamboo green marks, pale jade halo, warm gold rim light.",
    ),
    (
        "leading_tile",
        "Leading Tile",
        "A prominent first mahjong tile at the head of a short formation, "
        "visually emphasized like a leader's banner badge.",
        "Ivory tiles, dark ink markings, warm gold trim, subtle navy shadows.",
    ),
    (
        "low_echo",
        "Low Echo",
        "Four low-numbered mahjong tiles (1, 2, 3, 4) standing in a row on "
        "a dark felt table, with translucent duplicate silhouettes of each "
        "tile radiating outward behind them like acoustic echoes. Faint "
        "concentric sound rings ripple from the group.",
        "Ivory tile faces, deep ink numerals, pale teal echo silhouettes, dark felt, soft gold rings.",
    ),
    (
        "tea_ceremony",
        "Tea Ceremony",
        "A refined tea bowl with three drifting steam curls above it, rendered "
        "as a formal ceremonial enamel badge.",
        "Cream porcelain, muted celadon glaze, warm brown tea tones, soft gold rim.",
    ),
    (
        "ghost_hand",
        "Ghost Hand",
        "A spectral translucent hand reaching around a scored tile from "
        "behind, eerie but graphic and clean like a talisman emblem.",
        "Pale cyan ghost hand, ivory tile, cool indigo shadows, silver outline.",
    ),
    (
        "clean_streak",
        "Clean Streak",
        "A neatly aligned series of polished tiles with a shining streak "
        "running across them like a reward for consistency.",
        "Ivory tiles, bright white gleam, pale gold edging, cool slate accents.",
    ),
    (
        "obsession",
        "Obsession",
        "An eye motif locked onto a single ornate yaku sigil, intense and "
        "focused, but simplified into badge language.",
        "Muted ivory eye, crimson focal ring, dark navy outlines, warm brass accents.",
    ),
    (
        "bonfire",
        "Bonfire",
        "A stacked pyre of tiles and wood burning upward in a stylized flame "
        "shape, compact and iconic.",
        "Orange flame, charcoal black embers, warm wood browns, gold highlights.",
    ),
    (
        "river_runner",
        "River Runner",
        "A swift river current curling around a sequence of tiles, showing "
        "flow and forward motion in one bold emblem.",
        "Teal water ribbons, ivory tiles, deep blue shadows, silver highlights.",
    ),
    (
        "melting_ice",
        "Melting Ice",
        "A cracked ice shard dripping away around a trapped tile, fragile and "
        "temporary, rendered as a clean enamel pin.",
        "Pale blue ice, white frost, cool grey cracks, faint cyan glow.",
    ),
    (
        "silk_thread",
        "Silk Thread",
        "A taut silk thread winding around a tile and fraying at the ends, "
        "thin, elegant, and precarious.",
        "Soft ivory thread, muted rose accents, cream tile, pale gold trim.",
    ),
    (
        "shadow_hand",
        "Shadow Hand",
        "A dark silhouetted hand mirroring another relic-like shape, hinting "
        "at imitation and duplication.",
        "Deep indigo shadow, soft silver edge light, muted ivory accents, charcoal background tones.",
    ),
    (
        "empty_frame",
        "Empty Frame",
        "An ornate empty frame with a dramatic hollow center, meant to read "
        "clearly as a missing-slot emblem.",
        "Antique gold frame, dark hollow center, warm brass shine, muted umber details.",
    ),
    (
        "gold_idol",
        "Gold Idol",
        "A small squat golden idol with a serene face and stacked coin-like "
        "base, compact and lucky.",
        "Rich gold body, amber highlights, dark brown recesses, pale ivory glints.",
    ),
    (
        "jade_abacus",
        "Jade Abacus",
        "A compact abacus with jade beads and a sturdy brass frame, rendered "
        "as a crisp economy-themed badge.",
        "Green jade beads, warm brass frame, ivory highlights, dark wood accents.",
    ),
    (
        "nest_egg",
        "Nest Egg",
        "A single gleaming egg nestled in a tidy woven nest, stylized and "
        "readable as a savings symbol.",
        "Warm gold egg, tan nest fibers, soft amber light, muted brown shadows.",
    ),
    (
        "patience",
        "Patience",
        "An hourglass resting beside a tidy stack of unused tiles, communicating "
        "calm restraint and waiting.",
        "Warm sand gold, ivory glass frame, pale blue highlights, dark bronze accents.",
    ),
    (
        "way_of_pairs",
        "Way of Pairs",
        "Two matched mahjong tiles mirrored side by side inside a formal crest, "
        "simple and symmetrical.",
        "Ivory tiles, warm gold crest border, deep navy accents, soft amber highlights.",
    ),
    (
        "way_of_triplets",
        "Way of Triplets",
        "Three matching mahjong tiles arranged in a tight triangular crest, "
        "bold and balanced.",
        "Ivory tiles, gold border, dark ink details, muted crimson accents.",
    ),
    (
        "way_of_sequences",
        "Way of Sequences",
        "Three consecutive mahjong tiles stepping upward in order, designed "
        "as a clean progression emblem.",
        "Ivory tiles, pale gold frame, deep teal accents, soft cream highlights.",
    ),
    (
        "fortunes_favor",
        "Fortune's Favor",
        "A lucky charm medallion with a swirling fortune motif and tiny spark "
        "stars around it, energetic but tidy.",
        "Warm gold medallion, crimson charm knot, ivory spark accents, dark navy shadows.",
    ),
    (
        "cracked_tile",
        "Cracked Tile",
        "A weathered mahjong tile split by a jagged crack, still holding "
        "together but visibly unstable. Keep it iconic rather than scenic.",
        "Aged ivory tile, charcoal crack, dusty ochre debris, faded ink.",
    ),
    (
        "star_tile",
        "Star Tile",
        "A mahjong tile transformed into a lucky celestial enamel pin, with "
        "a bold five-pointed star emblem centered on the face and small "
        "radiant accent marks around it. Clean, iconic badge language rather "
        "than a full environmental scene.",
        "Warm ivory tile body, gold star accents, deep navy details, soft amber highlights.",
    ),
    (
        "smoke_bomb",
        "Smoke Bomb",
        "A compact old-style smoke bomb with a pull ring and a curling cloud "
        "bursting from one side, simplified into a clear emblem.",
        "Dark iron shell, pale grey smoke, warm brass ring, charcoal accents.",
    ),
    (
        "phantom_relic",
        "Phantom Relic",
        "An ornate jeweled treasure chest resting on a dark stone pedestal, "
        "with a translucent ghostly duplicate of the same chest drifting out "
        "of it like vapor, slightly offset and hovering. Faint wisps of mist "
        "curl from the seam.",
        "Aged gold chest, deep mahogany wood, pale cyan phantom glow, cool indigo shadows.",
    ),
    (
        "ritual_blade",
        "Ritual Blade",
        "An ornate ceremonial blade pointed downward over a relic-like charm, "
        "sharp, sacrificial, and symbolic.",
        "Polished steel blade, crimson inlay, dark bronze hilt, warm gold accents.",
    ),
    (
        "disgust",
        "Disgust",
        "A weather vane whose East and West cardinal arms have curled inward "
        "and fused at the tips into a single grimacing mouth, the North and "
        "South arms hanging slack. A pale sour wisp curls from the joined mouth.",
        "Tarnished bronze body, muted teal patina, pale sickly green wisp, charcoal accents.",
    ),
    (
        "curio_cabinet",
        "Curio Cabinet",
        "A miniature glass-fronted display cabinet crammed with tiny assorted "
        "relics on stepped shelves, each visible through the door pane. A small "
        "brass keyhole sits at the center. Collector-shelf vibe compressed to a "
        "single badge silhouette.",
        "Warm mahogany frame, pale amber glass, brass fittings, muted multicolor shelf contents.",
    ),
    (
        "lotus_bloom",
        "Lotus Bloom",
        "A single stylized lotus flower in full bloom, layered petals radiating "
        "outward from a gold seedpod center, with a trailing stem curling below. "
        "Iconic badge framing, symmetrical.",
        "Soft pink petals, cream inner tones, gold seedpod, deep jade leaf accents.",
    ),
    (
        "wall_weaver",
        "Wall Weaver",
        "A loom frame weaving a tight lattice of tiny mahjong tiles together like "
        "fabric, with a shuttle paused mid-pass. Render as a clean crest showing "
        "a densely packed tile-wall weave.",
        "Warm wood loom, ivory woven tiles, dark ink grid lines, muted gold shuttle.",
    ),
    (
        "kong_collector",
        "Kong Collector",
        "Four matching mahjong tiles stacked in a perfect square bundle, bound "
        "together by a tight gold cord with a hanging coin tassel. Trophy-like, "
        "compact, iconic.",
        "Ivory tiles, dark ink faces, warm gold cord, polished coin tassel.",
    ),
    (
        "no_honor_but_wealth",
        "No Honor But Wealth",
        "A toppled honor tile lying face-down at the base of a neat stack of gold "
        "coins, a single coin balanced on edge atop the stack. Greedy, irreverent, "
        "badge-clean.",
        "Ivory honor tile, warm gold coins, deep crimson accents, charcoal shadow.",
    ),
    (
        "sweepstakes",
        "Sweepstakes",
        "A small spinning prize drum on a stand, its handle mid-turn, with a "
        "single golden ticket half-ejected from the slot. A couple of stray "
        "paper slips drift nearby. Carnival-lucky energy.",
        "Polished brass drum, cream tickets, muted crimson stand, soft gold highlights.",
    ),
    (
        "beggars_cup",
        "Beggar's Cup",
        "A dented tin alms cup resting on worn cobblestones, a single gold coin "
        "inside and a second coin balanced on the rim. Humble but slowly filling.",
        "Battered pewter cup, warm gold coins, muted slate cobbles, soft amber highlight.",
    ),
    (
        "cosmopolitan",
        "Cosmopolitan",
        "A compact travel trunk plastered with small yaku-symbol stamps like "
        "passport stickers, a pair of leather straps crossing the lid. Worldly, "
        "well-traveled, badge-readable.",
        "Warm tan leather, dark brass corners, muted multicolor stamps, ivory highlights.",
    ),
    (
        "heirloom",
        "Heirloom",
        "An ornate antique pocket watch hanging from a fine chain, its engraved "
        "back faintly worn from generations of handling. A subtle patina gleam "
        "catches the rim. Timeless keepsake feel.",
        "Deep gold casing, warm amber patina, ivory watch face, dark brown chain accents.",
    ),
    (
        "tourist",
        "Tourist",
        "A small brass compass lying atop a folded paper map, with a tiny camera "
        "and a luggage tag beside it. Travelogue motif packed into a single crest.",
        "Warm brass compass, cream paper map, muted teal tag, soft tan luggage tones.",
    ),
    (
        "kintsugi",
        "Kintsugi",
        "A cracked ceramic tea bowl whose fracture lines have been mended with "
        "rivers of molten gold, the seams glowing faintly as if still warm. "
        "The bowl sits upright on a dark wooden stand, each gold vein tracing "
        "the break pattern in crisp lacquered relief. Repaired, not hidden.",
        "Pale celadon ceramic, bright molten gold veins, warm amber highlights, deep mahogany stand.",
    ),
    (
        "ant_trail",
        "Ant Trail",
        "A circular procession of tiny ants marching head-to-tail around the "
        "edge of a single mahjong tile, wrapping continuously so the line has "
        "no visible start or end. The tile face shows a low numeric mark; the "
        "trail reads as an unbroken loop from 9 back to 1.",
        "Ivory tile face, deep ink numeral, glossy black ant silhouettes, muted bamboo green underlayer.",
    ),
    (
        "brocade_pouch",
        "Brocade Pouch",
        "A small silk drawstring pouch with rich brocade patterning — "
        "embroidered clouds and knotwork in gold thread on deep indigo fabric, "
        "the cord loosely cinched with a jade bead dangling. A few faint "
        "glimmers of colored light leak from the mouth, hinting at charmed "
        "tokens inside. Resting on dark wood.",
        "Deep indigo silk, antique gold embroidery, pale jade bead, warm amber highlights.",
    ),
]


def build_object_prompt(
    name: str,
    visual: str,
    palette: str,
    rarity: str,
    *,
    from_reference: bool = False,
) -> str:
    """Prompt for the transparent color render (`*_object.png` — albedo fallback for the loader).

    When `from_reference=True`, assumes the call is an image edit against a
    grayscale relief guide and appends instructions to honor that guide's
    silhouette and divider structure. Text-prompted runs omit those lines so
    the model isn't told to match a reference that doesn't exist.
    """
    base = (
        f"{style_prefix(rarity)}\n\n"
        f"Asset type: cloisonné enamel pin relic color render, product-shot framing.\n"
        f"Relic name: '{name}'.\n"
        f"Subject: use only the central subject from this description; ignore any environment, scene, or setting words: {visual}\n"
        f"Enamel palette (colors apply to the recessed champlevé cells only; the wires and negative-space substrate follow the metal tier above): {palette}\n"
        "Composition/framing: centered square, badge fills most of the frame with a small uniform margin, pin plane parallel to the camera.\n"
        "Lighting/mood: neutral studio key plus a soft warm rim, gentle radial burnish on the substrate.\n"
        "Materials: vitreous glass enamel fills recessed below raised cloisonné wires in the metal tier above; the outer bezel reads as a thick legible frame in that same metal.\n"
        "Background: a perfectly flat, uniform, pure black archival backdrop — the solid matte black of a museum archival photography plate, with no gradient, no vignette, no texture, and no shadow cast onto the surface. Every region inside the outer silhouette resolves as solid opaque material — either enamel fill or metal wire."
    )
    if from_reference:
        base += (
            "\nRelief guide usage: the accompanying grayscale relief guide defines SHAPE, SILHOUETTE, and internal divider layout. Match its outer silhouette, centered placement, divider structure, major shapes, and orientation exactly; add color and material on top. Any gray region INSIDE the silhouette resolves as a solid opaque enamel fill. Transparency applies only to the area outside the outer silhouette.\n"
            "Keep the proportions, parts, and framing of the relief guide intact."
        )
    return base


def build_height_prompt(name: str, visual: str) -> str:
    """Prompt for `*_height.png` — matches input silhouette; bound as linear GPU relief."""
    return (
        f"Grayscale relief guide for the cloisonné enamel pin relic '{name}'.\n"
        f"Subject: {visual}\n"
        "Output: centered square, pure black background, front-facing near-orthographic enamel pin silhouette with clean internal partitions.\n"
        "Tonal key (each region is a single flat tone with a hard edge to its neighbor):\n"
        "  - White: highest raised metal — outer bezel rim and cloisonné wire dividers.\n"
        "  - Mid-grays: recessed champlevé enamel fill surface inside the silhouette.\n"
        "  - Black: the area outside the outer silhouette.\n"
        "Every area inside the outer silhouette resolves to gray or white, so the later color pass treats it as a solid opaque enamel fill.\n"
        "A clean monochrome grayscale relief, matching the input in proportion."
    )


_REMBG_SESSION = None


def remove_background(path: Path) -> None:
    """Replace `path` in-place with an alpha-matted RGBA PNG via rembg (u2net)."""
    global _REMBG_SESSION
    try:
        from rembg import remove, new_session
    except ImportError as e:
        raise SystemExit(
            "Error: rembg not installed. Run: pip install rembg pillow onnxruntime\n"
            f"(import failed: {e})"
        )
    from PIL import Image
    import io

    if _REMBG_SESSION is None:
        _REMBG_SESSION = new_session("u2net")

    out_bytes = remove(path.read_bytes(), session=_REMBG_SESSION)
    Image.open(io.BytesIO(out_bytes)).convert("RGBA").save(path, format="PNG")


# Alpha threshold for the binary mask. Anti-aliased edge pixels sit in the
# 1..254 range; anything at or above this becomes silhouette. Matches what a
# forward-rendered extrusion would accept without eating feathered edges.
MASK_ALPHA_THRESHOLD = 16


# Height-map luminance thresholds for alpha derivation. The height prompt asks
# for pure black (0) outside the silhouette and gray/white (>= mid) inside, so
# the alpha rule is: below `_LO` → transparent, at/above `_HI` → opaque, and a
# short anti-aliased ramp between. Keeping the ramp short (vs. remapping the
# whole 0..255 range) is critical: mid-gray enamel fills must stay fully
# opaque, not half-transparent.
HEIGHT_ALPHA_LO = 8
HEIGHT_ALPHA_HI = 24


def clean_object_background(object_path: Path, height_path: Path) -> None:
    """Strip the object render's background using the most reliable method available.

    Preference ladder:
      1. Derive alpha from the height map's silhouette (deterministic, free,
         already aligned — the object was generated from the height guide).
      2. Fall back to rembg (u2net) if the height map is missing or mismatched.

    rembg is fragile on dark-on-dark subjects (e.g. iron pins on dark vignettes
    where salient-object matting reads the bezel as background and shreds it).
    The height-derived path sidesteps that failure entirely.
    """
    if alpha_from_height(object_path, height_path):
        print(f"  Alpha from height map: {object_path.name}")
        return
    remove_background(object_path)
    print(f"  Cleaned bg (rembg fallback): {object_path.name}")


def alpha_from_height(object_path: Path, height_path: Path) -> bool:
    """Use the height map's silhouette as the alpha channel for the object render.

    gpt-image-2 refuses transparent backgrounds, and u2net's salient-object
    matter fails on dark-on-dark subjects (e.g. iron-tier pins rendered against
    a dark vignette). The height map already encodes the silhouette we want:
    pure black outside, gray/white inside. Threshold the height map and use it
    as alpha on the object render — no ML, deterministic, already aligned
    because the object was generated from the height reference.

    Returns False if either input is missing or shapes don't match after
    resizing; in that case the caller should fall back to rembg.
    """
    from PIL import Image

    if not object_path.exists() or not height_path.exists():
        return False
    with Image.open(object_path) as im:
        rgba = im.convert("RGBA")
    with Image.open(height_path) as hm:
        height = hm.convert("L")
    if height.size != rgba.size:
        height = height.resize(rgba.size, Image.LANCZOS)
    # Short ramp at the silhouette edge: transparent below LO, opaque at/above
    # HI, linear in between. Preserves an anti-aliased halo without eating the
    # opacity of interior enamel fills (which sit at mid-gray luminance).
    ramp = max(1, HEIGHT_ALPHA_HI - HEIGHT_ALPHA_LO)
    alpha = height.point(
        lambda v: 0 if v < HEIGHT_ALPHA_LO
        else 255 if v >= HEIGHT_ALPHA_HI
        else int((v - HEIGHT_ALPHA_LO) * 255 / ramp),
        mode="L",
    )
    r, g, b, _ = rgba.split()
    Image.merge("RGBA", (r, g, b, alpha)).save(object_path, format="PNG")
    return True


def flatten_height_to_black_bg(path: Path) -> None:
    """Composite `path` over pure black and save as grayscale L-mode PNG.

    The height prompt asks for a black background, but the model sometimes
    returns a transparent alpha or a near-black but non-zero background.
    The relief loader reads this as linear height, so a non-black background
    bleeds into the extrusion silhouette. Force it to true black here.
    """
    from PIL import Image

    with Image.open(path) as im:
        rgba = im.convert("RGBA")
    black = Image.new("RGB", rgba.size, (0, 0, 0))
    black.paste(rgba, mask=rgba.split()[-1])
    black.convert("L").save(path, format="PNG")


def write_mask_from_height(height_path: Path, mask_path: Path) -> bool:
    """Write `mask_path` as a binary L-mode silhouette of the height map.

    The height map is the canonical source of silhouette truth under the
    object-first pipeline: the object's alpha is itself derived from the
    height's luminance (via `alpha_from_height`), so deriving the mask from
    the height map directly skips a redundant threshold pass and guarantees
    the mask and object alpha agree pixel-for-pixel.

    Returns False if the height map does not exist.
    """
    from PIL import Image

    if not height_path.exists():
        return False
    with Image.open(height_path) as im:
        height = im.convert("L")
    mask = height.point(lambda v: 255 if v >= HEIGHT_ALPHA_LO else 0, mode="L")
    mask.save(mask_path, format="PNG")
    return True


def write_mask_from_object(object_path: Path, mask_path: Path) -> bool:
    """Write `mask_path` as a binary L-mode silhouette of `object_path`'s alpha.

    Returns False if `object_path` is missing or the source is fully opaque
    (raw API output before `rembg` — producing a full-rectangle mask would
    be useless, so we refuse rather than write garbage).
    """
    from PIL import Image

    if not object_path.exists():
        return False
    with Image.open(object_path) as im:
        alpha = im.convert("RGBA").split()[-1]
    if alpha.getextrema()[0] >= MASK_ALPHA_THRESHOLD:
        return False
    mask = alpha.point(lambda a: 255 if a >= MASK_ALPHA_THRESHOLD else 0, mode="L")
    mask.save(mask_path, format="PNG")
    return True


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

    save_response_image(response.data[0], output_path)


def save_response_image(data, output_path: Path) -> None:
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


# OpenAI's DALL-E 2 edits endpoint used the convention alpha=0 (transparent)
# → editable, alpha=255 (opaque) → preserved. gpt-image-2's documentation
# describes masks as "guidance... the model may not follow the exact shape
# with complete precision" but does not explicitly state which alpha value
# marks editable regions. Exposed as a module constant so this can be flipped
# from the CLI / env if the DALL-E 2 convention turns out to be wrong for
# gpt-image-2.
EDIT_MASK_TRANSPARENT_IS_EDITABLE = True


def build_edit_mask(reference_path: Path, mask_out: Path) -> bool:
    """Build an RGBA mask that asks the model to preserve the reference's silhouette.

    The mask marks the reference's silhouette as the editable region (so the
    model can repaint interior detail) and the surround as the preserved
    region (so the outer shape stays put). The alpha values used are driven
    by `EDIT_MASK_TRANSPARENT_IS_EDITABLE` since gpt-image-2's exact mask
    semantics are not documented.

    Requires the reference to already have a cutout alpha (i.e. the surround
    is transparent). Returns False if the reference is fully opaque — with no
    silhouette to lock onto there's nothing this mask can usefully convey.
    """
    from PIL import Image

    if not reference_path.exists():
        return False
    with Image.open(reference_path) as im:
        ref = im.convert("RGBA")
    alpha = ref.split()[-1]
    lo, hi = alpha.getextrema()
    if lo == 255 and hi == 255:
        # Reference has no transparent surround — no silhouette to lock.
        return False

    if EDIT_MASK_TRANSPARENT_IS_EDITABLE:
        # Silhouette → transparent (editable), surround → opaque (preserved).
        mask_alpha = alpha.point(
            lambda a: 0 if a >= MASK_ALPHA_THRESHOLD else 255, mode="L"
        )
    else:
        # Silhouette → opaque (editable), surround → transparent (preserved).
        mask_alpha = alpha.point(
            lambda a: 255 if a >= MASK_ALPHA_THRESHOLD else 0, mode="L"
        )

    # Fill RGB with mid-gray; edits API ignores RGB under an alpha mask but
    # some servers reject pure-black RGB.
    gray = Image.new("RGB", ref.size, (128, 128, 128))
    gray.putalpha(mask_alpha)
    gray.save(mask_out, format="PNG")
    return True


def generate_from_reference(
    client: OpenAI,
    prompt: str,
    output_path: Path,
    model: str,
    reference_path: Path,
    input_fidelity: str,
    try_lock_silhouette: bool = False,
) -> None:
    """Use an existing source image as the structural reference for a new output.

    When `try_lock_silhouette=True`, attempt to derive a mask from the
    reference's alpha and pass it to the edits endpoint. If the reference
    has no usable alpha (e.g. it hasn't been background-removed yet), the
    call silently proceeds without a mask — the name makes that fallback
    explicit. The mask is written to a process-local temp file and cleaned
    up even if the edit call raises.
    """
    # `input_fidelity` is a gpt-image-1-only knob; gpt-image-2 rejects it with
    # `invalid_input_fidelity_model`. Pass it only for models that accept it.
    edit_kwargs = {"model": model, "prompt": prompt}
    if model == "gpt-image-1":
        edit_kwargs["input_fidelity"] = input_fidelity

    mask_path: Path | None = None
    if try_lock_silhouette:
        fd, mask_str = tempfile.mkstemp(prefix="editmask_", suffix=".png")
        os.close(fd)
        candidate = Path(mask_str)
        if build_edit_mask(reference_path, candidate):
            mask_path = candidate
        else:
            candidate.unlink(missing_ok=True)
            print(
                f"  (silhouette lock requested but {reference_path.name} has "
                "no usable alpha; falling back to unmasked edit)"
            )

    try:
        with reference_path.open("rb") as image_file:
            if mask_path is not None:
                with mask_path.open("rb") as mask_file:
                    response = client.images.edit(
                        image=image_file, mask=mask_file, **edit_kwargs
                    )
            else:
                response = client.images.edit(image=image_file, **edit_kwargs)
        save_response_image(response.data[0], output_path)
    finally:
        if mask_path is not None:
            mask_path.unlink(missing_ok=True)


def artifact_targets(base_dir: Path, slug: str, artifact: str) -> list[tuple[str, Path]]:
    if artifact == "object":
        return [("object", base_dir / f"{slug}_object.png")]
    if artifact == "height":
        return [("height", base_dir / f"{slug}_height.png")]
    if artifact == "mask":
        return [("mask", base_dir / f"{slug}_mask.png")]
    # Object-first ordering: the object render is the authoritative pass
    # (text-prompted, most context), and the height pass edits it into a
    # relief guide with the object's silhouette as a hard constraint.
    return [
        ("object", base_dir / f"{slug}_object.png"),
        ("height", base_dir / f"{slug}_height.png"),
    ]


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate Mahjuro relic 3D source art via the OpenAI image API"
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
        "--artifact",
        choices=("object", "height", "mask", "both"),
        default="both",
        help=(
            "Which asset artifact to generate per relic (default: both → "
            "height+object+mask). 'mask' only re-derives masks from existing objects."
        ),
    )
    parser.add_argument(
        "--skip-mask",
        action="store_true",
        help="Do not rewrite *_mask.png when the object changes.",
    )
    parser.add_argument(
        "--model",
        type=str,
        default="gpt-image-2",
        help="Image model to use (default: gpt-image-2).",
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
        default=0.0,
        help="Seconds to sleep between API calls (default: 0.0).",
    )
    parser.add_argument(
        "--height-mode",
        choices=("reference", "generate"),
        default="reference",
        help=(
            "How to make *_height.png assets: edit the object render into a "
            "relief guide using the object's silhouette as a mask (default — "
            "locks height silhouette to the object) or generate from text only."
        ),
    )
    parser.add_argument(
        "--height-input-fidelity",
        choices=("low", "high"),
        default="high",
        help="Input fidelity for reference-based height generation (default: high, gpt-image-1 only).",
    )
    parser.add_argument(
        "--object-mode",
        choices=("reference", "generate"),
        default="generate",
        help=(
            "How to make *_object.png assets: generate from text (default — "
            "object is the authoritative pass) or edit a pre-existing height "
            "guide into color."
        ),
    )
    parser.add_argument(
        "--object-input-fidelity",
        choices=("low", "high"),
        default="high",
        help=(
            "Input fidelity for reference-based object generation "
            "(default: high, gpt-image-1 only, only used with --object-mode=reference)."
        ),
    )
    args = parser.parse_args()

    if args.list:
        for i, (slug, name, _, _) in enumerate(RELICS, 1):
            rarity = SLUG_TO_RARITY.get(slug, "?")
            print(
                f"  {i:2d}. {name:<22s}  [{rarity:<9s}]  "
                f"source/{slug}_object.png, source/{slug}_height.png, source/{slug}_mask.png"
            )
        print(
            "\n  Runtime albedo: relics/{slug}.png (derived separately)."
        )
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

    if args.artifact == "mask":
        wrote = 0
        missing = 0
        for idx, (slug, name, _, _) in targets:
            obj = out_dir / f"{slug}_object.png"
            height_path = out_dir / f"{slug}_height.png"
            mask_path = out_dir / f"{slug}_mask.png"
            if mask_path.exists() and not args.force:
                print(
                    f"[{idx + 1}] {name}: mask exists — use --force to regenerate"
                )
                continue
            if write_mask_from_height(height_path, mask_path) or write_mask_from_object(obj, mask_path):
                print(f"[{idx + 1}] {name}: wrote {mask_path.name}")
                wrote += 1
            else:
                print(
                    f"[{idx + 1}] {name}: no height map and {obj.name} is "
                    "missing or fully opaque — skipping"
                )
                missing += 1
        print(f"\nDone. wrote={wrote} missing={missing} → {out_dir}")
        return

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
        rarity = rarity_for(slug)
        print(f"\n[{idx + 1}/{len(RELICS)}] {name} [{rarity}]")

        object_output_path = out_dir / f"{slug}_object.png"
        height_output_path = out_dir / f"{slug}_height.png"

        # Mask derivation at the end of the iteration is gated on a
        # successful height pass — without a height map there's no silhouette
        # source. Starts from disk state so single-artifact runs (e.g.
        # --artifact object after a prior --artifact height) still finalize
        # the mask.
        height_ready = height_output_path.exists()

        for artifact_name, output_path in artifact_targets(out_dir, slug, args.artifact):
            object_ref_prompt = build_object_prompt(
                name, visual, palette, rarity,
                from_reference=(args.object_mode == "reference"),
            )
            prompt = (
                object_ref_prompt
                if artifact_name == "object"
                else build_height_prompt(name, visual)
            )

            if args.dry_run:
                print(f"  {artifact_name} prompt:\n    {prompt}\n")
                continue

            if output_path.exists() and not args.force:
                print(
                    f"  Skipping {artifact_name} (exists): {output_path.name}  "
                    "— use --force to regenerate"
                )
                skipped += 1
                if artifact_name == "height":
                    height_ready = True
                continue

            try:
                assert client is not None
                if artifact_name == "height" and args.height_mode == "reference":
                    if not object_output_path.exists():
                        print(
                            "  Height needs an object reference first; generating object pass."
                        )
                        generate_image(
                            client,
                            build_object_prompt(name, visual, palette, rarity),
                            object_output_path,
                            args.model,
                            args.size,
                        )
                        generated += 1
                    # The object ships with a full-alpha seamless backdrop
                    # (non-destructive pipeline), so there's no alpha-based
                    # silhouette the edit API could lock to. The height
                    # prompt's "pure black outside" tonal directive is what
                    # enforces silhouette alignment here.
                    generate_from_reference(
                        client,
                        prompt,
                        output_path,
                        args.model,
                        object_output_path,
                        args.height_input_fidelity,
                    )
                    flatten_height_to_black_bg(output_path)
                    print(f"  Black bg: {output_path.name}")
                elif artifact_name == "object" and args.object_mode == "reference":
                    if not height_output_path.exists():
                        print(
                            "  Object needs a height reference first; generating height pass."
                        )
                        generate_image(
                            client,
                            build_height_prompt(name, visual),
                            height_output_path,
                            args.model,
                            args.size,
                        )
                        flatten_height_to_black_bg(height_output_path)
                        print(f"  Black bg: {height_output_path.name}")
                        generated += 1
                        height_ready = True
                    generate_from_reference(
                        client,
                        prompt,
                        output_path,
                        args.model,
                        height_output_path,
                        args.object_input_fidelity,
                    )
                else:
                    generate_image(client, prompt, output_path, args.model, args.size)
                    if artifact_name == "height":
                        flatten_height_to_black_bg(output_path)
                        print(f"  Black bg: {output_path.name}")
                generated += 1
                if artifact_name == "height":
                    height_ready = True
            except Exception as e:
                print(f"  Error generating {name} [{artifact_name}]: {e}")
                failed += 1
                continue

        # Write the mask once from the height map. The object file is left
        # exactly as the model returned it (seamless-paper backdrop, full
        # alpha). The Rust loader cuts the object's alpha against this mask
        # at load time so re-running the pipeline is never destructive to
        # raw artifacts on disk.
        if not args.skip_mask and height_ready:
            mask_path = out_dir / f"{slug}_mask.png"
            if write_mask_from_height(height_output_path, mask_path):
                print(f"  Wrote mask: {mask_path.name}")

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
