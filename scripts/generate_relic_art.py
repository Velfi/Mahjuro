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

**Not written here** — produced by `scripts/derive_relic_runtime_textures.py`:

  • `assets/textures/relics/{slug}.png` — derived runtime albedo (preferred in-game).
  • `source/{slug}_mask.png` — optional silhouette for mesh extrusion (`--emit-masks`).

The game’s load order and fallbacks live in `src/render/relic_pipeline.rs`.

Usage:
    pip install openai requests
    export OPENAI_API_KEY="sk-..."
    python scripts/generate_relic_art.py                       # all missing source assets
    python scripts/generate_relic_art.py --artifact object     # only object renders
    python scripts/generate_relic_art.py --artifact height     # only relief/height sources
    python scripts/generate_relic_art.py --artifact both --name overflow
    python scripts/generate_relic_art.py --force               # regenerate all
    python scripts/generate_relic_art.py --relic 17            # one relic by index
    python scripts/generate_relic_art.py --name kan_drum       # one relic by slug
    python scripts/generate_relic_art.py --list                # list all relics
    python scripts/generate_relic_art.py --dry-run             # print prompts only

After source PNGs exist, run `derive_relic_runtime_textures.py` (or
`pipeline_relic_ai.py`) so `relics/{slug}.png` and masks stay in sync.
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
# relief derivation.
STYLE_PREFIX = (
    "A single isolated collectible enamel pin relic designed for a game asset "
    "pipeline. Front-facing or near-orthographic presentation, centered "
    "composition, full object visible, no frame, no card backing, no "
    "typography, no logo, no extra props, no background scene clutter. Render "
    "it like a polished hard-enamel lapel pin with crisp metal outlines, "
    "distinct color cells, strong silhouette readability, and minimal "
    "perspective distortion."
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
    # ── Balatro-inspired relics (Patch F/G) ────────────────────────────
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
        "A compact polishing wheel pressing against a mahjong tile face, "
        "throwing off tiny bright sparks. Simple workshop-tool silhouette, "
        "clean and emblematic.",
        "Cream tile, brushed steel wheel, warm brass fittings, golden sparks.",
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
        "A ceremonial crest showing a single elegant suit symbol enclosed "
        "inside a pure circular border, minimal and disciplined.",
        "Soft ivory, pale jade green, warm gold edging, charcoal ink accents.",
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
        "A repeating wave motif made from small low-ranked tile symbols, "
        "stacked like resonant echoes radiating outward.",
        "Ivory symbols, teal shadow echoes, pale gold border, deep slate accents.",
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
        "A mysterious duplicate relic silhouette offset behind a primary one, "
        "like a spectral afterimage of another treasure.",
        "Cool silver body, pale cyan phantom glow, deep indigo shadows, muted gold rim.",
    ),
    (
        "ritual_blade",
        "Ritual Blade",
        "An ornate ceremonial blade pointed downward over a relic-like charm, "
        "sharp, sacrificial, and symbolic.",
        "Polished steel blade, crimson inlay, dark bronze hilt, warm gold accents.",
    ),
]


def build_object_prompt(name: str, visual: str, palette: str) -> str:
    """Prompt for the transparent color render (`*_object.png` — albedo fallback for the loader)."""
    return (
        f"{STYLE_PREFIX}\n\n"
        f"Asset type: 3D relic source object render\n"
        f"Primary request: create the relic '{name}' as a single enamel pin relic.\n"
        f"Subject: reinterpret this motif as an enamel pin badge rather than a full scene: {visual}\n"
        f"Color palette: {palette}\n"
        "Style/medium: polished stylized 3D enamel pin render, readable at game-camera scale.\n"
        "Composition/framing: centered, square, object fills about 75% of the frame, front-facing, minimal tilt.\n"
        "Lighting/mood: neutral studio key light plus soft rim light, very little cast shadow.\n"
        "Materials/textures: hard enamel color fills separated by raised polished metal borders; outer rim should be thick and readable.\n"
        "Constraints: the provided grayscale relief guide is the source of truth for SHAPE and SILHOUETTE only. Match it exactly: same outer silhouette, same centered placement, same internal divider layout, same major shapes, same front-facing orientation. Only add color/material information on top of that structure. "
        "Interpreting the relief guide: ONLY pure black pixels OUTSIDE the outer silhouette are empty background. Any gray or dark-gray region INSIDE the silhouette is a low-relief recessed enamel fill — it must be painted as solid opaque enamel in the color render, NOT left transparent, NOT rendered as a hole, cutout, gap, window, or opening. The pin is a single continuous solid object; do not punch holes through it and do not show anything behind it. "
        "Do not invent or remove parts, do not change proportions, do not rotate or recompose the badge. Transparent or plain isolated background OUTSIDE the silhouette only, no text, no border, no extra objects, no tabletop scene, no environment."
    )


def build_height_prompt(name: str, visual: str) -> str:
    """Prompt for `*_height.png` — matches object silhouette; bound as linear GPU relief."""
    return (
        f"Create a grayscale relief guide for the enamel pin relic '{name}'.\n"
        f"Subject: reinterpret this motif as the same enamel pin badge design: {visual}\n"
        "Output a centered square image on a black background with the same front-facing silhouette and internal partitions as the enamel pin object render.\n"
        "White = highest polished metal rim or raised divider, mid-gray = enamel fill surface, black = empty background or deepest recess.\n"
        "Design this as a clean blueprint for the later color object render. Do not invent a new composition, do not show perspective, do not add extra objects, no color, no text, no border."
    )


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
        choices=("object", "height", "both"),
        default="both",
        help="Which asset artifact to generate per relic (default: both).",
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
    args = parser.parse_args()

    if args.list:
        for i, (slug, name, _, _) in enumerate(RELICS, 1):
            print(
                f"  {i:2d}. {name:<22s}  "
                f"source/{slug}_object.png, source/{slug}_height.png"
            )
        print(
            "\n  Runtime: relics/{slug}.png (+ optional source/{slug}_mask.png) via derive_relic_runtime_textures.py"
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
