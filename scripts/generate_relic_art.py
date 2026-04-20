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

Backgrounds on generated `*_object.png` are stripped in-place via `rembg`
(u2net) so the saved PNG has a clean transparent alpha. Disable with
`--skip-bg-removal`; rerun only the clean pass on existing files with
`--reclean-bg`.

Usage:
    pip install openai requests rembg pillow onnxruntime
    export OPENAI_API_KEY="sk-..."
    python scripts/generate_relic_art.py                       # all missing source assets
    python scripts/generate_relic_art.py --artifact object     # only object renders
    python scripts/generate_relic_art.py --artifact height     # only relief/height sources
    python scripts/generate_relic_art.py --artifact mask       # only rewrite masks from existing objects
    python scripts/generate_relic_art.py --artifact both --name overflow
    python scripts/generate_relic_art.py --force               # regenerate all
    python scripts/generate_relic_art.py --relic 17            # one relic by index
    python scripts/generate_relic_art.py --name kan_drum       # one relic by slug
    python scripts/generate_relic_art.py --list                # list all relics
    python scripts/generate_relic_art.py --dry-run             # print prompts only
    python scripts/generate_relic_art.py --reclean-bg          # strip bg + rewrite masks on existing objects
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


OUTPUT_DIR = (
    Path(__file__).resolve().parent.parent
    / "assets"
    / "textures"
    / "relics"
    / "source"
)

# Shared style prefix injected into every prompt. Tuned for isolated enamel-pin
# style relic renders that can be reviewed directly and fed into silhouette /
# relief derivation. Background handling is set per-artifact in the builder
# functions (transparent for object, black for height) — intentionally omitted here.
STYLE_PREFIX = (
    "A single isolated collectible enamel pin relic designed for a game asset "
    "pipeline. Front-facing near-orthographic presentation, centered "
    "composition, full object visible. Polished hard-enamel lapel pin look: "
    "crisp metal outlines, distinct flat color cells, strong silhouette "
    "readability, minimal perspective distortion."
)


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
]


def build_object_prompt(name: str, visual: str, palette: str) -> str:
    """Prompt for the transparent color render (`*_object.png` — albedo fallback for the loader)."""
    return (
        f"{STYLE_PREFIX}\n\n"
        f"Asset type: enamel pin relic color render (RGBA, transparent background).\n"
        f"Relic name: '{name}'.\n"
        f"Subject: use only the central subject from this description; ignore any environment, scene, or setting words: {visual}\n"
        f"Color palette: {palette}\n"
        "Style/medium: polished stylized enamel pin render, readable at game-camera scale.\n"
        "Composition/framing: centered square, badge fills most of the frame with a small uniform margin, front-facing, minimal tilt.\n"
        "Lighting/mood: neutral studio key plus soft rim light, faint contact shadow only.\n"
        "Materials: hard enamel color fills separated by raised polished metal borders; the outer rim is thick and legible.\n"
        "Background: fully transparent alpha outside the badge silhouette. The pin is a single continuous solid object — every region inside the outer silhouette is opaque enamel or metal, never a cutout or window.\n"
        "Relief guide usage: the accompanying grayscale relief guide defines SHAPE, SILHOUETTE, and internal divider layout. Match its outer silhouette, centered placement, divider structure, major shapes, and orientation exactly; add only color and material on top. Any gray region INSIDE the silhouette is a recessed enamel fill (paint it as solid opaque enamel), not a hole. Only the pure-black area OUTSIDE the silhouette becomes transparent.\n"
        "Keep proportions, parts, and framing of the relief guide intact."
    )


def build_height_prompt(name: str, visual: str) -> str:
    """Prompt for `*_height.png` — matches object silhouette; bound as linear GPU relief."""
    return (
        f"Grayscale relief guide for the enamel pin relic '{name}'.\n"
        f"Subject: use only the central subject from this description; ignore any environment, scene, or setting words: {visual}\n"
        "Output: centered square, pure black background, front-facing near-orthographic enamel pin silhouette with clean internal partitions.\n"
        "Tonal key (strict, flat regions with hard edges — no gradients, no texture, no noise within a region):\n"
        "  - White: highest polished metal rim or raised divider.\n"
        "  - Mid-gray: recessed enamel fill surface inside the silhouette.\n"
        "  - Black: only the area OUTSIDE the outer silhouette.\n"
        "Every area inside the outer silhouette must be gray or white — never black — so the later color pass treats it as a solid opaque enamel fill, not a cutout.\n"
        "Monochrome only, no color, no text, no border, no frame."
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


def generate_from_reference(
    client: OpenAI,
    prompt: str,
    output_path: Path,
    model: str,
    reference_path: Path,
    input_fidelity: str,
) -> None:
    """Use an existing source image as the structural reference for a new output."""
    with reference_path.open("rb") as image_file:
        response = client.images.edit(
            model=model,
            image=image_file,
            prompt=prompt,
            input_fidelity=input_fidelity,
        )
    save_response_image(response.data[0], output_path)


def artifact_targets(base_dir: Path, slug: str, artifact: str) -> list[tuple[str, Path]]:
    if artifact == "object":
        return [("object", base_dir / f"{slug}_object.png")]
    if artifact == "height":
        return [("height", base_dir / f"{slug}_height.png")]
    if artifact == "mask":
        return [("mask", base_dir / f"{slug}_mask.png")]
    return [
        ("height", base_dir / f"{slug}_height.png"),
        ("object", base_dir / f"{slug}_object.png"),
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
        default=0.0,
        help="Seconds to sleep between API calls (default: 0.0).",
    )
    parser.add_argument(
        "--height-mode",
        choices=("reference", "generate"),
        default="generate",
        help=(
            "How to make *_height.png assets: generate from text only "
            "(default) or use an existing object render as an image edit reference."
        ),
    )
    parser.add_argument(
        "--height-input-fidelity",
        choices=("low", "high"),
        default="high",
        help="Input fidelity for reference-based height generation (default: high).",
    )
    parser.add_argument(
        "--object-mode",
        choices=("reference", "generate"),
        default="reference",
        help=(
            "How to make *_object.png assets: use the generated height guide as "
            "an image edit reference (default) or generate from text only."
        ),
    )
    parser.add_argument(
        "--object-input-fidelity",
        choices=("low", "high"),
        default="high",
        help="Input fidelity for reference-based object generation (default: high).",
    )
    parser.add_argument(
        "--skip-bg-removal",
        action="store_true",
        help="Do not run rembg on generated *_object.png files.",
    )
    parser.add_argument(
        "--reclean-bg",
        action="store_true",
        help=(
            "Skip generation; just run rembg on existing *_object.png for the "
            "selected relics. Useful for cleaning old assets."
        ),
    )
    args = parser.parse_args()

    if args.list:
        for i, (slug, name, _, _) in enumerate(RELICS, 1):
            print(
                f"  {i:2d}. {name:<22s}  "
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

    if args.reclean_bg:
        cleaned = 0
        missing = 0
        for idx, (slug, name, _, _) in targets:
            path = out_dir / f"{slug}_object.png"
            if not path.exists():
                print(f"[{idx + 1}] {name}: no {path.name} — skipping")
                missing += 1
                continue
            print(f"[{idx + 1}] {name}: cleaning {path.name}")
            remove_background(path)
            cleaned += 1
            if not args.skip_mask:
                mask_path = out_dir / f"{slug}_mask.png"
                if write_mask_from_object(path, mask_path):
                    print(f"  Wrote mask: {mask_path.name}")
                else:
                    print(f"  Skipped mask: {path.name} is fully opaque")
        print(f"\nDone. cleaned={cleaned} missing={missing} → {out_dir}")
        return

    if args.artifact == "mask":
        wrote = 0
        missing = 0
        for idx, (slug, name, _, _) in targets:
            obj = out_dir / f"{slug}_object.png"
            mask_path = out_dir / f"{slug}_mask.png"
            if mask_path.exists() and not args.force:
                print(
                    f"[{idx + 1}] {name}: mask exists — use --force to regenerate"
                )
                continue
            if not obj.exists():
                print(f"[{idx + 1}] {name}: no {obj.name} — skipping")
                missing += 1
                continue
            if not write_mask_from_object(obj, mask_path):
                print(
                    f"[{idx + 1}] {name}: {obj.name} is fully opaque "
                    "(run --reclean-bg first) — skipping"
                )
                missing += 1
                continue
            print(f"[{idx + 1}] {name}: wrote {mask_path.name}")
            wrote += 1
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
        print(f"\n[{idx + 1}/{len(RELICS)}] {name}")

        object_output_path = out_dir / f"{slug}_object.png"
        height_output_path = out_dir / f"{slug}_height.png"
        for artifact_name, output_path in artifact_targets(out_dir, slug, args.artifact):
            prompt = (
                build_object_prompt(name, visual, palette)
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
                            build_object_prompt(name, visual, palette),
                            object_output_path,
                            args.model,
                            args.size,
                        )
                        generated += 1
                        if not args.skip_bg_removal:
                            remove_background(object_output_path)
                            print(f"  Cleaned bg: {object_output_path.name}")
                        if not args.skip_mask:
                            mask_path = out_dir / f"{slug}_mask.png"
                            if write_mask_from_object(object_output_path, mask_path):
                                print(f"  Wrote mask: {mask_path.name}")
                    generate_from_reference(
                        client,
                        prompt,
                        output_path,
                        args.model,
                        object_output_path,
                        args.height_input_fidelity,
                    )
                elif artifact_name == "object" and args.object_mode == "reference":
                    if not height_output_path.exists():
                        print(
                            "  Object needs a height reference first; generating height pass."
                        )
                        if args.height_mode == "reference" and object_output_path.exists():
                            generate_from_reference(
                                client,
                                build_height_prompt(name, visual),
                                height_output_path,
                                args.model,
                                object_output_path,
                                args.height_input_fidelity,
                            )
                        else:
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
                generated += 1
                if artifact_name == "object" and not args.skip_bg_removal:
                    remove_background(output_path)
                    print(f"  Cleaned bg: {output_path.name}")
                if artifact_name == "object" and not args.skip_mask:
                    mask_path = out_dir / f"{slug}_mask.png"
                    if write_mask_from_object(output_path, mask_path):
                        print(f"  Wrote mask: {mask_path.name}")
                if artifact_name == "height":
                    flatten_height_to_black_bg(output_path)
                    print(f"  Black bg: {output_path.name}")
            except Exception as e:
                print(f"  Error generating {name} [{artifact_name}]: {e}")
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
