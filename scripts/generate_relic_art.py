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
    python scripts/generate_relic_art.py --artifact both --name strength_in_numbers
    python scripts/generate_relic_art.py --force               # regenerate all
    python scripts/generate_relic_art.py --relic 17            # one relic by index
    python scripts/generate_relic_art.py --name kan_drum       # one relic by slug
    python scripts/generate_relic_art.py --list                # list all relics
    python scripts/generate_relic_art.py --dry-run             # print prompts only
"""

import argparse
import base64
import json
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


REPO_ROOT = Path(__file__).resolve().parent.parent
RELIC_RS_PATH = REPO_ROOT / "src" / "core" / "relic.rs"
RELIC_JSON_PATH = REPO_ROOT / "assets" / "data" / "relics.json"


def load_slug_to_rarity() -> dict:
    """Build the slug → rarity mapping from the game's source-of-truth files.

    Slugs come from `asset_filename` arms in src/core/relic.rs (e.g.
    `RelicId::TripletBoost => "triplet_boost.png"`). Rarities come from
    assets/data/relics.json, which `all_relic_defs` deserializes at runtime.
    The two are joined on the snake_case slug.

    Returns `{ "triplet_boost": "Common", ... }`. Fails loud if either file is
    missing or unparseable — drift between the script and the game must be
    surfaced, not silently defaulted.
    """
    if not RELIC_RS_PATH.exists():
        raise SystemExit(
            f"Cannot read slug list: {RELIC_RS_PATH} does not exist."
        )
    if not RELIC_JSON_PATH.exists():
        raise SystemExit(
            f"Cannot read rarity map: {RELIC_JSON_PATH} does not exist."
        )

    text = RELIC_RS_PATH.read_text()
    slugs = {
        m.group(1)
        for m in re.finditer(
            r'RelicId::\w+\s*=>\s*"([a-z0-9_]+)\.png"', text
        )
    }
    if not slugs:
        raise SystemExit(
            "Failed to parse relic.rs — asset_filename arms did not match "
            "expected shape."
        )

    try:
        defs = json.loads(RELIC_JSON_PATH.read_text())
    except json.JSONDecodeError as e:
        raise SystemExit(f"Failed to parse {RELIC_JSON_PATH}: {e}")

    slug_to_rarity = {}
    for entry in defs:
        slug = entry.get("id")
        rarity = entry.get("rarity")
        if not slug or not rarity:
            raise SystemExit(
                f"relics.json entry missing id/rarity: {entry!r}"
            )
        if slug not in slugs:
            # Defined in JSON but no asset_filename arm — harmless for art gen.
            continue
        slug_to_rarity[slug] = rarity.capitalize()
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
        "Three mahjong tiles standing upright in a tight row at the center "
        "of an emerald felt table — a 4, 5, and 6 of bamboo — with a "
        "trailing streak of warm light arcing through them as if caught "
        "mid-rush. Faint motion blur on the felt behind the row, gold "
        "sparks flicking off the trailing edge. A few stray face-down "
        "wall tiles lie blurred in the background.",
        "Ivory tile faces, deep emerald felt, bamboo green marks, warm gold light streak, subtle amber sparks.",
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
        "A triplet of honor tiles standing upright in a tight row at the "
        "center of an emerald felt table — a wind tile flanked by a red "
        "dragon and a green dragon — their faces glowing hot from within "
        "as if forged. Embers and faint heat shimmer rise from the seams "
        "between them, and concentric shockwave rings ripple outward across "
        "the felt. A few stray face-down wall tiles lie blurred in the "
        "background.",
        "Ivory tile faces, deep emerald felt, glowing crimson and jade honor marks, warm ember sparks, faint gold shockwaves.",
    ),
    (
        "red_dragon_rage",
        "Red Dragon Rage",
        "Inked scroll illustration of three dragon mahjong tiles standing "
        "upright in a tight triplet on an emerald felt table — a red dragon "
        "flanked by a green dragon and a white dragon. The center tile's "
        "crimson 'chun' character is alive: its brushstrokes uncoil into a "
        "serpentine eastern dragon that bursts forward off the tile face, "
        "scaled crimson body lashing through the air with its jaws thrown "
        "open in a roar. A plume of brushwork flame trails from its mouth "
        "and scorches the faces of the flanking tiles, leaving sumi-e "
        "smoke curls drifting upward. The dragon's tail still loops back "
        "into unfinished crimson calligraphy strokes on the tile face. A "
        "few stray face-down wall tiles lie blurred in the background.",
        "Painterly inked scroll illustration, sumi-e brushwork, ivory tile faces, deep emerald felt, crimson chun calligraphy uncoiling into a coiled eastern dragon, jade-green and blue-edged white dragon glyphs, brushstroke flame and smoke curls, warm amber rim light.",
    ),
    (
        "green_luck",
        "Green Luck",
        "A single mahjong tile standing upright at the center of an emerald "
        "felt table, its face carved with the green bamboo 'one bam' — a "
        "stylized peacock with jade plumage. Small gold coins are stacked "
        "in a neat pile beside the tile, with a few more spilling toward "
        "the foreground catching warm light. A scatter of numbered suit "
        "tiles — bamboos and circles, no honors — lies blurred in the "
        "background.",
        "Ivory tile face, deep emerald felt, jade-green peacock mark, warm gold coins, soft amber rim light.",
    ),
    (
        "white_dragons_hush",
        "White Dragon's Hush",
        "A pair of blank-faced white dragon mahjong tiles standing upright "
        "side by side at the center of an emerald felt table, their ivory "
        "faces unmarked except for the carved blue-edged border of the "
        "haku tile. A faint cool moonlit halo rings the pair, and the "
        "felt around them is utterly still — even the stray face-down "
        "wall tiles in the blurred background seem to hush. A single "
        "small zodiac tile lies face-up just beside the pair, as if "
        "drawn from quiet.",
        "Ivory tile faces, deep emerald felt, blue-edged white dragon borders, cool pale moonlit rim light, hushed muted shadows.",
    ),
    (
        "joker_tile",
        "Joker Tile",
        "A single mahjong tile standing upright at the center of an emerald "
        "felt table, its face split into four uneven quadrants showing "
        "ghostly impressions of different tile faces — a bamboo stick, a "
        "circle dot, a character glyph, and a wind arrow — overlapping like "
        "shifting reflections. A faint prismatic shimmer plays across the "
        "ivory surface as if the tile cannot decide what it is. A few stray "
        "face-down wall tiles lie blurred in the background.",
        "Ivory tile face, deep emerald felt, prismatic shimmer, muted bamboo green / circle blue / character crimson marks.",
    ),
    (
        "strength_in_numbers",
        "Strength in Numbers",
        "A double-stacked mahjong wall sitting at the center of an emerald "
        "felt table, far taller than a normal four-row wall — six rows "
        "high, face-down tiles packed tight. The top is bowing outward "
        "and tiles are spilling off the back edges in slow cascades, "
        "tumbling down onto the felt and pooling at the base of the wall. "
        "A few loose tiles have rolled forward into the foreground. Soft "
        "amber rim light catches the lacquered tile backs.",
        "Lacquered face-down mahjong tiles, deep emerald felt, towering double-stacked wall, tumbling cascade of tiles spilling off the edges, soft amber rim light, dark warm shadows.",
    ),
    (
        "quick_draw",
        "Quick Draw",
        "A neat row of face-down mahjong wall tiles along the back of an "
        "emerald felt table, with one tile mid-flight in a freeze-frame "
        "arc as it lifts from the wall toward the foreground — a streak "
        "of warm light trailing behind it. The tile is just starting to "
        "rotate face-up, hinting at the glyph beneath. A few stray "
        "face-up tiles rest blurred in the foreground.",
        "Ivory tile faces, deep emerald felt, motion-blurred warm gold light streak, dark lacquered tile backs, soft amber rim light.",
    ),
    (
        "chain_reaction",
        "Chain Reaction",
        "Two short rows of mahjong tiles laid face-up on an emerald felt "
        "table, one row behind the other. The back row glows softly as if "
        "just scored, with a single arc of warm light leaping forward "
        "from its last tile to ignite the first tile of the front row, "
        "kindling that one in turn. Faint gold sparks trail along the arc. "
        "A few stray face-down wall tiles lie blurred in the background.",
        "Ivory tile faces, deep emerald felt, warm gold arc of light, soft amber afterglow on the back row, drifting gold sparks.",
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
        "A chunk of raw magnetite ore resting on a rough cool-grey stone slab, with one "
        "mahjong tile lifting free from a row of face-down wall tiles and "
        "floating toward three matching tiles already gathered beside the "
        "stone. Faint iron filings cling to the ore's crystalline facets.",
        "Blue-black magnetite, silver-grey filings, ivory tile faces, cool grey stone slab.",
    ),
    (
        "wild_winds",
        "Wild Winds",
        "Three mahjong tiles standing upright in a tight row at the center "
        "of an emerald felt table — a 4 of bamboo on the left, a 6 of "
        "bamboo on the right, and a wind tile in the middle slot. The "
        "wind tile's carved glyph is heavily motion-blurred, smeared "
        "sideways into ghostly streaks as if shifting between forms "
        "faster than the eye can fix. Strong horizontal motion-blur lines "
        "rake across the wind tile and trail off both edges of the row, "
        "with petals and dust caught in the streaking blur. The 4 and 6 "
        "stand sharp and still in contrast. A few stray face-down wall "
        "tiles lie blurred in the background.",
        "Ivory tile faces, deep emerald felt, bamboo green marks, deep blue wind glyph smeared in heavy horizontal motion blur, pale wind-streak light, drifting motion-blur petals and dust, sharp focus on the flanking number tiles.",
    ),
    (
        "dragon_echo",
        "Dragon Echo",
        "A triplet of red dragon mahjong tiles standing upright at the "
        "center of an emerald felt table, their crimson 'chun' glyphs "
        "burning with inner light. Ghostly translucent echoes of the same "
        "triplet recede behind it in three progressively fainter arcs, as "
        "if a dragon's roar reverberating across the felt. Faint embers "
        "drift between the layers. A few stray face-down wall tiles lie "
        "blurred in the background.",
        "Ivory tile faces, deep emerald felt, glowing crimson dragon glyphs, ghostly amber echo arcs, drifting ember sparks.",
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
        "A single mahjong tile lying face-up at the center of an emerald "
        "felt table — a dora indicator with its glyph carved in red — "
        "wearing a small Chinese imperial mianguan crown hovering just "
        "above its face: a flat rectangular gold-and-lacquer board with "
        "rows of beaded jade tassels hanging from its front and back "
        "edges, accented by ornate carved dragon and cloud motifs. A "
        "second dora indicator tile lies face-up just behind it, slightly "
        "offset. Soft warm light glows from beneath both tiles, and a few "
        "flecks of gold drift across the felt. A few stray face-down "
        "wall tiles lie blurred in the background.",
        "Ivory tile faces, deep emerald felt, crimson dora glyphs, gold-and-black-lacquer mianguan crown, jade beaded tassels, carved dragon and cloud motifs, warm amber underglow, drifting gold flecks.",
    ),
    (
        "round_compass",
        "Round Compass",
        "Four wind mahjong tiles — East, South, West, North — laid face-up "
        "in a ring at the cardinal points of an emerald felt table, their "
        "glyphs aligned outward like a compass rose. A faint golden "
        "wind-rose is etched into the felt between them, and a glowing "
        "needle of warm light arcs from the center toward the East tile. "
        "A few stray face-down wall tiles lie blurred in the background.",
        "Ivory tile faces, deep emerald felt, deep blue wind glyphs, etched gold wind-rose, warm amber needle of light.",
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
        "garden_keeper",
        "Garden Keeper",
        "A small bell-shaped glass cloche dome resting on an emerald felt "
        "table, sealing in a tiny private greenhouse. Inside, a single "
        "flower mahjong tile stands upright with two real living blossoms "
        "— a delicate plum and an orchid — growing out of the carved "
        "glyph on its face, leaves and tendrils curling around the tile. "
        "Soft warm grow-light glows from beneath, and faint condensation "
        "beads the inside of the glass. A few stray face-down wall tiles "
        "lie blurred on the felt outside the dome.",
        "Clear glass cloche dome with beaded condensation, ivory tile face, deep emerald felt, soft pastel pink plum and jade orchid blossoms, curling green leaves, warm amber grow-light underglow.",
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
        "A miniature cherry tree growing from a small lacquered pot at "
        "the center of an emerald felt table — gnarled dark trunk, "
        "delicate spreading branches — where every blossom on the tree "
        "is actually a small upright mahjong flower tile clipped to the "
        "branch like a pink bloom, ivory faces painted with plum, orchid, "
        "bamboo, and chrysanthemum motifs. A few flower tiles have fallen "
        "and lie scattered face-up on the felt below, with stacks of warm "
        "gold coins gathered beneath the tree where the tiles have "
        "dropped. Loose pink petals drift through the air. A few stray "
        "face-down wall tiles lie blurred in the background.",
        "Dark gnarled cherry trunk and branches, ivory mahjong flower tiles clipped on as blossoms, soft pastel pink and jade flower glyphs, deep emerald felt, lacquered black pot, warm gold coin stacks beneath fallen tiles, drifting pink petals, soft amber rim light.",
    ),
    (
        "jade_serpent",
        "Jade Serpent",
        "A glazed porcelain serpent figurine coiled around a bundle of "
        "bamboo stalks, its body sculpted from creamy white china with "
        "fine crackle glaze and inlaid scales of polished jade cabochons. "
        "Its eyes are tiny faceted emeralds catching the light. The "
        "figurine rests on a dark lacquer plinth against a soft, neutral "
        "studio backdrop.",
        "Cream porcelain body, polished jade scales, faceted emerald eyes, dark lacquer plinth, neutral studio backdrop.",
    ),
    (
        "red_serpent",
        "Red Serpent",
        "A glazed porcelain serpent figurine coiled around a single mahjong "
        "character tile, its body sculpted from creamy white china with "
        "fine crackle glaze and inlaid scales of polished carved ruby and "
        "carnelian. Its eyes are tiny faceted rubies catching the light. "
        "The figurine rests on a dark lacquer plinth against a soft, "
        "neutral studio backdrop.",
        "Cream porcelain body, polished ruby and carnelian scales, faceted ruby eyes, ivory tile with dark ink character, dark lacquer plinth, neutral studio backdrop.",
    ),
    (
        "blue_serpent",
        "Blue Serpent",
        "A glazed porcelain serpent figurine coiled around a single mahjong "
        "circles/dots tile, its body sculpted from creamy white china with "
        "fine crackle glaze and inlaid scales of polished lapis lazuli and "
        "sapphire. Its eyes are tiny faceted sapphires catching the light. "
        "The figurine rests on a dark lacquer plinth against a soft, "
        "neutral studio backdrop.",
        "Cream porcelain body, polished lapis and sapphire scales, faceted sapphire eyes, ivory tile with blue dot pips, dark lacquer plinth, neutral studio backdrop.",
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
        "high_tide",
        "High Tide",
        "A small coastal survey boat afloat on the same tidal estuary, the "
        "waterline risen to swallow the mud flats. Measuring stakes driven into "
        "the bed at intervals, only their tops still showing above the water. "
        "Flat grey estuary light.",
        "Dark hull on grey water, white measuring stakes barely showing above the surface, grey flat light.",
    ),
    (
        "merchants_eye",
        "Merchant's Eye",
        "A brass jeweler's loupe hovering over a single mahjong tile lying "
        "face-up on an emerald felt table, magnifying its carved glyph "
        "into sharp focus. A few scattered gold coins and stray face-down wall "
        "tiles lie blurred in the background.",
        "Polished brass loupe, ivory tile face, deep emerald felt, paper price tag with red string, warm gold coin accents, soft amber lamplight.",
    ),
    (
        "i_got_a_guy",
        "I Got A Guy",
        "A creased paper business card and a scrap with a phone number scribbled "
        "in ink, clipped together with a bent paperclip, resting on emerald "
        "felt beside a mahjong wall tile edge. Slightly comedic noir mood, "
        "small prop readable at icon scale.",
        "Cream paper, black ink scrawl, steel paperclip, emerald felt, warm counter light.",
    ),
    (
        "edge_runner",
        "Edge Runner",
        "Two mahjong tiles standing upright at opposite ends of an emerald "
        "felt table — a '1 of circles' on the left and a '9 of circles' on "
        "the right — with a taut line of light arcing between them across "
        "the felt like a tightrope. The middle of the table is empty save "
        "for soft shadow; faint gold sparks trail along the light. A few "
        "stray face-down wall tiles lie blurred in the background.",
        "Ivory tile faces, deep emerald felt, deep blue circle marks, warm gold light arc, subtle amber sparks.",
    ),
    (
        "lucky_seven",
        "Lucky Seven",
        "A vintage slot machine — the three-reel mechanical type — showing "
        "triple sevens in the window. A single brass lever on the side. "
        "Sitting alone on a dark wooden counter in a dim room.",
        "Chrome and brass machine, cherry-red sevens, deep walnut wood, dim amber.",
    ),
    (
        "momentum",
        "Momentum",
        "A stack of mahjong tiles tipping forward like falling dominoes, "
        "with the last few tiles in the chain already airborne and trailing "
        "bright motion streaks. Energy visibly accumulates toward the leading "
        "edge of the cascade.",
        "Ivory tile faces, deep ink markings, warm gold motion streaks, dark neutral shadow plane.",
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
        "A short row of mahjong tiles standing upright at the center of an "
        "emerald felt table — only terminals and honors: a 1 of bamboo, a "
        "9 of circles, a red dragon, an East wind. Between them a heavy "
        "ornamental gate is etched into the felt in faint gold lines, its "
        "doors drawn shut behind the row as if barring the middle ranks "
        "from entry. A few stray face-down wall tiles lie blurred in the "
        "background.",
        "Ivory tile faces, deep emerald felt, deep blue circle and crimson dragon marks, etched gold gate lines, warm amber rim light.",
    ),
    (
        "golden_engine",
        "Golden Engine",
        "A small ornate brass machine sitting at the center of an emerald "
        "felt table — part steam engine, part music box — with polished "
        "brass pipes, a glass dome, and a coin-slotted hopper on top. "
        "Stacks of gold coins are feeding into the hopper, while a tiny "
        "pressure gauge on its face has its needle pinned to the right. "
        "Warm golden exhaust vapor curls upward from the pipes, glittering "
        "with flecks of gold. A few stray face-down wall tiles lie blurred "
        "in the background.",
        "Polished brass machine body, glass dome, deep emerald felt, warm gold coin stacks feeding the hopper, pinned brass pressure gauge, golden exhaust vapor with drifting gold flecks, soft amber rim light.",
    ),
    (
        "snowball",
        "Snowball",
        "A massive sphere built entirely from packed mahjong tiles, "
        "rolling across an emerald felt table. Tiles of every suit — "
        "bamboos, circles, characters, winds, dragons — are jammed "
        "together at every angle, ivory faces and lacquered backs "
        "pressed into the curve, with a few loose tiles tumbling along "
        "to be absorbed at the leading edge. A widening trail of fallen "
        "tiles curves across the felt behind it. A few stray face-down "
        "wall tiles lie blurred in the background.",
        "Ivory tile faces, lacquered tile backs, deep emerald felt, multicolored suit marks, warm amber rim light, drifting motion blur on the trail.",
    ),
    (
        "second_wind",
        "Second Wind",
        "A small pull-back wind-up toy car mid-action on an emerald felt "
        "table — chunky cartoon proportions, painted tin body with chrome "
        "bumpers and rubber tires. A hand has just released it: the car "
        "lurches forward in a freeze-frame burst, tiny gold sparks "
        "kicking from the rear wheels and a streak of warm motion-blur "
        "light trailing behind. A single mahjong tile is pinned face-up "
        "under one wheel as it launches off. A few stray face-down wall "
        "tiles lie blurred in the background.",
        "Painted tin toy car, chrome bumpers, rubber tires, deep emerald felt, ivory tile face, warm gold spark trail, motion-blur streak, soft amber rim light.",
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
        "A single hero mahjong tile stands front and center on deep "
        "emerald felt, its carved glyph blazing molten gold as if lit from "
        "within. Behind it, the rest of the final hand is fanned outward "
        "in a theatrical kabuki-pose arc, each tile angled to catch the "
        "light — deep blue circles, jade-green bamboo, crimson characters. "
        "Sharp gold rays radiate outward from the hero tile across the "
        "felt, and a translucent ghost echo of each tile rises just beside "
        "it, scoring a second time. Drifting gold sparks hang in the air "
        "around the pose.",
        "Ivory tile faces, deep emerald felt, molten gold glyph, sharp radiating gold rays, fanned kabuki arc, ghostly translucent echo tiles, drifting gold sparks.",
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
        "silver_filigree_lantern",
        "Silver Filigree Lantern",
        "An ornate silver lantern wrapped in delicate filigree scrollwork — "
        "fine pierced silver vinework forms the cage around a steady inner "
        "flame. Heirloom craftsmanship, like a temple votive elevated to "
        "treasure. The successor to a humble paper lantern.",
        "Polished silver filigree, cool argent highlights, warm gold flame, "
        "soft pearl-white inner glow, faint indigo shadow in the recesses.",
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
        "geese",
        "Geese",
        "Five geese rising in a tight V-formation across a misty dawn sky "
        "above a reedy marsh, wings spread mid-beat. The lead bird at the apex "
        "is haloed warm gold, the four followers trailing in receding pairs "
        "behind it. A ghostly translucent echo lifts off each goose, drifting "
        "a wingbeat behind its source, telegraphing each of the five firing "
        "twice. Trails of gold sparks streak through the air in their wake. "
        "Below, dark water and silhouetted reeds catch faint amber light from "
        "the rising sun.",
        "Warm amber dawn sky, gold V-formation of geese, ghostly translucent echoes, drifting gold sparks, dark marsh water.",
    ),
    (
        "voice_of_the_people",
        "Voice of the People",
        "Four ivory mahjong character-suit tiles standing in a row on deep "
        "emerald felt, their faces carrying the traditional crimson "
        "man-zu numerals one, two, three, and four — 一, 二, 三, 四 — "
        "painted as bold brushed characters. Behind each tile, two or three "
        "ghostly translucent echo copies of the same tile fan outward in "
        "fading teal and warm gold, as if the common ranks are answering "
        "back in chorus. Soft concentric gold ripples spread across the "
        "felt at the tiles' feet, and warm amber rim light catches their "
        "edges. A few stray face-down wall tiles lie blurred in the "
        "background.",
        "Ivory tile faces, deep emerald felt, traditional crimson man-zu numerals, pale teal echo silhouettes, soft gold ripples, warm amber rim light.",
    ),
    (
        "voice_of_the_elite",
        "Voice of the Elite",
        "Four ivory mahjong character-suit tiles standing in a row on deep "
        "emerald felt, their faces carrying the traditional crimson "
        "man-zu numerals six, seven, eight, and nine — 六, 七, 八, 九 — "
        "painted as bold brushed characters. Behind each tile, two or three "
        "ghostly translucent echo copies of the same tile fan outward in "
        "fading crimson and warm gold, as if the high ranks are repeating "
        "their decree down the line. Soft concentric gold ripples spread "
        "across the felt at the tiles' feet, and warm amber rim light "
        "catches their edges. A few stray face-down wall tiles lie blurred "
        "in the background.",
        "Ivory tile faces, deep emerald felt, traditional crimson man-zu numerals, pale crimson echo silhouettes, soft gold ripples, warm amber rim light.",
    ),
    (
        "xxxl_egg",
        "XXXL Egg",
        "A comically oversized goose egg — smooth matte white shell — teetering in a "
        "realistic ground-nest bowl far too small for it: matted dry grasses and reeds "
        "with patches of moss and lichen, the cup lined with soft gray-white down and "
        "body feathers the way geese line their scrape before incubation. The nest "
        "reads shallow and bowl-shaped, not a tidy woven basket. The shell has a "
        "hairline crack with a sliver of down showing through. Something inside shifts — "
        "faint motion blur on the shell; a loose pale feather or two beside the rim. "
        "Warm amber rim light; compact enamel-badge composition.",
        "Matte white goose egg, grassy bowl nest with moss and lichen, down-lined cup, "
        "straw and olive tones, warm amber rim light, subtle motion hint.",
    ),
    (
        "tea_ceremony",
        "Tea Ceremony",
        "Paired chanoyu set (1 of 2 — the other pin is the Rakuware relic): use "
        "the **same rounded-square badge crop, bowl scale, and three-quarter "
        "view from slightly above** as the companion pin so they line up side "
        "by side as one story. Subject: a **smooth refined chawan** (porcelain "
        "or soft celadon), centered — the bowl that **precedes** the rustic "
        "raku piece in the pair. **Four** delicate rising steam wisps (not "
        "three), each whisper-tinted a different muted hue — sage green, pale "
        "shell pink, cool mist blue, warm ivory — suggesting the four guiding "
        "spirits of the ceremony without text or symbols. No wooden rest, no "
        "kiln drama; calm, luminous, **still steaming**. Formal ceremonial "
        "enamel badge — subject only, no tabletop scene.",
        "Cream porcelain, muted celadon glaze, four subtly tinted steam wisps, "
        "soft gold lip line, pale warm neutral void — **must match the void "
        "tone and framing of the Rakuware pin** in the same matched pair.",
    ),
    (
        "rakuware",
        "Rakuware",
        "Paired chanoyu set (2 of 2 — the other pin is the Tea Ceremony relic): "
        "use the **same rounded-square badge crop, bowl scale, and three-quarter "
        "view from slightly above** as the companion pin so they line up side "
        "by side as one story. Subject: a **hand-built raku chawan** — the bowl "
        "that **follows** the refined steaming bowl in the pair: bold crackle "
        "glaze, warm charcoal reduction, soft coppery flash at the rim. **No "
        "steam** — the tea is finished; the clay has cooled. A single hairline "
        "crack traced in fine metallic gold — kintsugi hint, not a full repair. "
        "Optional **small dark wooden rest** only if it stays inside the badge "
        "silhouette without clutter. Compact enamel composition; same calm "
        "negative space as the Tea Ceremony pin.",
        "Matte black raku glaze, gold kintsugi line in one crack, warm copper "
        "rim flash, deep walnut rest if present, **same pale warm neutral void "
        "as Tea Ceremony** — second chapter of the same two-pin tea story.",
    ),
    (
        "ghost_hand",
        "Ghost Hand",
        "A spectral translucent hand reaching around a scored tile from "
        "behind, eerie but graphic and clean like a talisman emblem.",
        "Pale cyan ghost hand, ivory tile, cool indigo shadows, silver outline.",
    ),
    (
        "humility",
        "Humility",
        "A hooded pilgrim in plain travel robes walking forward along a "
        "narrow road, staff in hand, head bowed. Behind them in the dusk "
        "an ornate golden crown set with a crimson dragon glyph lies "
        "abandoned in the dirt, half-forgotten. Ahead of the pilgrim the "
        "path is lit by a row of small humble lanterns receding into the "
        "distance, each one burning a little brighter than the last, "
        "drawing the eye toward the horizon. Painterly illustration with "
        "soft brushwork and warm amber lighting against a deep twilight sky.",
        "Painterly illustration, hooded pilgrim in muted travel robes, abandoned golden crown with crimson glyph in shadow behind, row of lanterns ahead with intensifying warm amber glow, deep twilight sky, soft brushwork.",
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
        "Abstract pin emblem, not a scene: a **stylized faceted ice shard or "
        "hex crystal** built from a few bold cloisonné planes — sharp geometry "
        "only, no realistic cube or plinth. The lower edge **breaks into "
        "graphic drip and bead shapes** (enamel teardrops) reading as melt, "
        "not water simulation. At the heart, **suggest** something sleeping "
        "through **symbolic negative space** — twin shallow crescent horn "
        "curves and two small bronze enamel dots for eyes inside one pale "
        "recessed cell; no body, no muzzle, no illustration of a beast. "
        "Hairline wire cracks divide facets like a badge crest. Flat graphic "
        "silhouette; readable at icon scale.",
        "Pale arctic blue and frost-white enamel facets, cool grey crack lines "
        "in wire, copper-bronze horn curves and eye dots as minimal glyphs, "
        "translucent aqua drip beads along the lower rim, soft amber pin "
        "rim light — **graphic enamel**, not painterly ice.",
    ),
    (
        "taotie",
        "Taotie",
        "A green-patinated bronze taotie ritual mask centered on a cool grey "
        "stone plinth — broad bulging eyes, twin curling horns "
        "rising over the brow, jaws cracked open in a hungry grin baring "
        "blunt bronze teeth. A single mahjong tile is captured mid-flight "
        "just above the open mouth, tilted and being drawn down into it; "
        "faint motion-blur lines trail behind the tile to read the pull "
        "as gravitational, not gentle. A shallow pool of meltwater still "
        "glistens beneath the mask's chin. Soft dark out-of-focus backdrop.",
        "Green-patinated bronze, deep jade highlights, warm bronze "
        "underglow in the recessed eye sockets, ivory tile face with "
        "crimson dragon glyph being devoured, cool grey stone plinth, "
        "glistening meltwater puddle, soft amber rim light.",
    ),
    (
        "silk_thread",
        "Silk Thread",
        "A plump silkworm cocooning a single upright mahjong tile — fresh ivory silk fans out "
        "from the worm's mouth and partially wraps the tile in a glossy, "
        "translucent skein, the still-bare lower half of the tile face "
        "showing through. The worm is mid-motion, its segmented body "
        "curled along the tile's edge, head lifted as it spins another "
        "loop. A single mulberry leaf rests on dark lacquered wood beside the "
        "in-progress cocoon. Soft dark out-of-focus backdrop.",
        "Pale cream silkworm, glossy ivory silk, dark lacquered wood surface, "
        "muted jade mulberry leaf, soft cream tile face with faint amber "
        "warmth from within the half-cocoon.",
    ),
    (
        "silk_moth",
        "Silk Moth",
        "A pale silk moth with broad ivory wings emerging from a split "
        "cocoon on dark lacquered wood — the cocoon is "
        "torn open along its seam, the discarded silk shell still threaded "
        "around the remains of a single mahjong tile beneath it. Fine "
        "silk strands trail from the moth's legs and the cocoon's torn "
        "rim, drifting in the air. The moth's wings are unfurled "
        "and dusted with a faint amber underglow as if freshly hatched. "
        "Soft dark out-of-focus backdrop.",
        "Ivory moth wings, soft pearl silk strands, dark lacquered wood surface, "
        "warm amber underglow, cream tile fragment beneath the cocoon, "
        "muted gold dust on the wings.",
    ),
    (
        "shadow_hand",
        "Shadow Hand",
        "A dark silhouetted hand mirroring another relic-like shape, hinting "
        "at imitation and duplication.",
        "Deep indigo shadow, soft silver edge light, muted ivory accents, charcoal background tones.",
    ),
    (
        "solitary_sage",
        "Solitary Sage",
        "A solitary robed mahjong sage seated cross-legged atop a single "
        "upright ivory tile on an emerald felt table, eyes closed in deep "
        "meditation, draped in faded crimson and umber silks that pool around "
        "the tile's base. The felt around him is utterly bare — no wall, no "
        "opponents, no scattered tiles — only the lone master and his lone "
        "tile. Warm amber rim light haloes his shoulders and drifts as faint "
        "gold sparks around him, and a single carved dragon glyph on the tile "
        "beneath glows softly in deep crimson. He plays alone because none "
        "remain worthy.",
        "Faded crimson and umber silks, ivory tile face, deep emerald felt, warm amber rim light, soft gold sparks, muted crimson glyph glow.",
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
        "A compact abacus with polished jade beads and a warm brass frame "
        "resting on an emerald felt table. Beside it, neat stacks of gold "
        "coins are grouped in fours, and a single mahjong tile carved with "
        "a jade-green character glyph leans against the frame. A few stray "
        "face-down wall tiles lie blurred in the background.",
        "Polished jade beads, warm brass frame, deep emerald felt, warm gold coin stacks, ivory tile with jade-green glyph, dark wood accents.",
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
        "hungry_ghost",
        "Hungry Ghost",
        "A translucent gaunt ghost-figure looming over an emerald felt "
        "table — distended belly, hollow sunken eyes, a tiny pinhole "
        "mouth — its grasping spectral hands cradling a single mahjong "
        "tile that is dissolving upward into wisps of pale smoke and "
        "drifting gold sparks as it is consumed. Faint red joss-paper "
        "embers smolder around the ghost's edges, and the air shimmers "
        "with cold draft. A few stray face-down wall tiles lie blurred "
        "in the background.",
        "Translucent pale ghost figure, distended belly, sunken hollow eyes, pinhole mouth, deep emerald felt, ivory tile dissolving into pale smoke, drifting gold sparks, smoldering red joss-paper embers, cool wraith-light, soft amber rim light.",
    ),
    (
        "disgust",
        "Disgust",
        "An East wind mahjong tile standing upright at the center of an "
        "emerald felt table, leaning slightly away with a sour curl of pale "
        "green wisp rising from its glyph. Beside it, three West wind tiles "
        "are clustered together as if uninvited but inseparable — one "
        "leaning in, two stacked just behind. A faint sickly sheen plays "
        "across all four tile faces. A few stray face-down wall tiles lie "
        "blurred in the background.",
        "Ivory tile faces, deep emerald felt, deep blue East and West wind glyphs, pale sickly green wisp, muted charcoal shadows.",
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
        "Two opposing hands meeting across an emerald felt table mid-trade, "
        "mirrored toward each other. From the right side of the frame, a "
        "RIGHT hand enters palm-up with the thumb on the LEFT side of "
        "the palm, cradling a small pile of warm gold coins, a few "
        "spilling onto the felt; this hand wears a dark embroidered "
        "sleeve with a subtle gold trim. From the left side of the frame, "
        "the OPPOSITE arm enters — a LEFT hand, palm-down, thumb on the "
        "RIGHT side of the palm, fingers spread to sweep a cluster of "
        "face-up honor mahjong tiles (winds and dragons) back toward "
        "itself; this arm wears a plain pale sleeve, distinctly different "
        "from the other. The two hands are clearly mirror-opposites of "
        "each other, not duplicates — anatomically correct opposing "
        "right and left hands. The transaction is frozen mid-motion. A "
        "few stray face-down wall tiles lie blurred in the background.",
        "Anatomically opposing right-hand and left-hand pair (mirror images, NOT two right hands), distinct dark embroidered sleeve versus plain pale sleeve, ivory honor tile faces with deep blue wind and crimson dragon glyphs, deep emerald felt, warm gold coins in an open palm-up right hand, palm-down left hand sweeping tiles, soft amber rim light.",
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
        "A single antique mahjong tile standing upright at the center of an "
        "emerald felt table, its ivory face yellowed with age and its "
        "carved glyph rubbed soft and shallow from generations of handling. "
        "A faint warm patina glow rims the edges, and tiny notches along "
        "the sides hint at decades of play. A few stray face-down wall "
        "tiles lie blurred in the background.",
        "Aged ivory tile face, deep emerald felt, soft worn glyph, warm amber patina rim light, muted dark wood accents.",
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
    (
        "eulers_number",
        "Euler's Number",
        "A small ivory plaque-style badge dominated by a single large engraved "
        "lowercase e in an upright classical serif, open counter, even stroke "
        "weight. Include a miniature visual proof in the same engraved draftsmanship: "
        "the curve y = 1/x from x = 1 toward e with cross-hatched or stippled area "
        "under that curve between 1 and e reading as exactly one unit square — the "
        "classic 'ln(e) = 1' picture. Along a margin or ribbon, tiny step marks can "
        "suggest (1 + 1/n)^n converging toward e. A fine spiral may still unwind "
        "from the letter as a secondary motif. Resting flat; letter plus proof read "
        "as one crest.",
        "Warm aged ivory, deep umber letter and diagram scoring, soft brass corner caps, muted ink shadows, subtle hatched enamel for the unit-area band under the hyperbola.",
    ),
    (
        "pi_constant",
        "Pi",
        "A scholar's ivory tablet with the Greek letter pi engraved large at "
        "the center. Include a clear visual proof motif: thin radial wedges sliced "
        "from a disk and rearranged into an alternating up-down stack that forms "
        "an almost rectangular strip — the classic dissection suggesting "
        "area = (half circumference) × radius = πr² without printing equations. "
        "A perfect compass-scribed circle and implied diameter remain visible as "
        "part of the construction. Fine radial tick marks at the rim. Flat, "
        "geometric, proof-like.",
        "Warm aged ivory, deep umber engraving, soft brass corner caps, muted ink shadows, soft contrast between adjacent wedge enamel fills.",
    ),
    (
        "big_hands",
        "Big Hands",
        "A pair of stylized open hands — palms forward, fingers spread wide — "
        "carved in raised relief on a single enamel pin crest, as if offering "
        "or catching an oversized fan of mahjong tiles fanned above them. "
        "Generous scale, broad gesture, readable silhouette.",
        "Warm copper cloisonné wires, cream and jade enamel fills, ivory tile hints, soft amber rim light.",
    ),
    (
        "tiny_hands",
        "Tiny Hands",
        "The same crest motif as Big Hands but at doll or Lilliputian scale: "
        "two minuscule delicate hands — short thin fingers, tiny palms — "
        "clearly dwarfed beneath a cluster of mahjong tiles that read large "
        "relative to the hands. Hands occupy only a small fraction of the "
        "badge height; gestures tight and nimble, fingers close together. "
        "Same framing and metal tier as its twin for a matched set, "
        "unmistakably tinier than Big Hands.",
        "Warm copper cloisonné wires, cream and jade enamel fills, ivory tile hints, soft amber rim light.",
    ),
    (
        "chrysalis",
        "Chrysalis",
        "A monarch butterfly chrysalis — jade-green and gold-speckled pupa "
        "hanging from a short silk pad stem, matte shell with subtle "
        "segment lines, contained and still; no wings yet. Cloisonné pin, "
        "centered, readable silhouette.",
        "Matte jade-green enamel, warm gold speckles, dark umber stem wrap, soft amber rim light.",
    ),
    (
        "monarch_butterfly",
        "Monarch Butterfly",
        "An open-wing monarch butterfly as a cloisonné pin — vivid orange "
        "wings with black veins and white dots at the wing edges, wings "
        "spread symmetric, body slim and black. Product-shot centered, "
        "wings filling the frame.",
        "Bright orange and black enamel wing bands, cream wing spots, copper cloisonné wires, soft amber rim light.",
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
