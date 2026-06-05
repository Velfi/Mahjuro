#!/usr/bin/env python3
"""
Generate relic **source** art for Mahjuro via Google's Nano Banana 2 image API.

Relic list matches `RelicId` / `asset_filename` in `src/core/relic.rs`. Art
direction: realistic soft-enamel lapel pins (custom die-cut silhouettes, raised metal lines, recessed fills).

**Writes (under `assets/textures/relics/` by default)**

  • `{slug}_object.png` — RGBA color render (transparent background). Fallback
    albedo if `derive` has not produced `relics/{slug}.png` yet; see
    `src/render/relic_pipeline.rs`.
  • `{slug}_height.png` — grayscale **relief guide**. At runtime this path is
    the linear height / `relief_tex` bind (same stem as `RelicId::source_heightmap_path`).
  • `{slug}_specular.png` — grayscale **specular mask** (L-mode). White = shiny
    raised metal; dark gray = matte soft-enamel wells. Loaded into the relief
    texture G channel at runtime (`RelicId::source_specular_path`). Derived from
    the height map by default; optional AI pass via `--specular-mode reference`.
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
    pip install google-genai pillow
    export GEMINI_API_KEY="..."
    python scripts/generate_relic_art.py                       # all missing source assets
    python scripts/generate_relic_art.py --artifact object     # only object renders
    python scripts/generate_relic_art.py --artifact height     # only relief/height sources
    python scripts/generate_relic_art.py --artifact mask       # only rewrite masks from existing heights
    python scripts/generate_relic_art.py --artifact specular   # only rewrite specular from existing heights
    python scripts/generate_relic_art.py --artifact both --name strength_in_numbers
    python scripts/generate_relic_art.py --force               # regenerate all
    python scripts/generate_relic_art.py --relic 17            # one relic by index
    python scripts/generate_relic_art.py --name kan_drum       # one relic by slug
    python scripts/generate_relic_art.py --list                # list all relics
    python scripts/generate_relic_art.py --dry-run             # print prompts only
"""

import argparse
import json
import os
import re
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _image_gen import (  # noqa: E402
    DEFAULT_MODEL,
    generate_image_bytes,
    init_client,
    parse_size,
)


OUTPUT_DIR = (
    Path(__file__).resolve().parent.parent / "assets" / "textures" / "relics"
)

# Shared style description injected into every prompt. Tuned for isolated
# soft-enamel-pin relic renders that can be reviewed directly and fed
# into silhouette / relief derivation.
#
# The core describes construction, material, and lighting in metal-agnostic
# terms. A per-rarity METAL_PROFILE is appended so Common/Uncommon/Rare/
# Legendary pins read as Iron/Copper/Silver/Gold — matching the canonical
# mapping in src/core/relic.rs `relic_visual`.
STYLE_CORE = (
    "A single isolated collectible soft-enamel lapel pin rendered as a "
    "realistic product photograph for a game asset pipeline. Front-facing "
    "near-orthographic presentation, pin plane parallel to the camera.\n\n"
    "Silhouette: the outer pin outline follows the natural shape of the "
    "subject — organic, asymmetric, circular, crest-shaped, or irregular as "
    "the motif demands. A continuous "
    "raised metal rim traces the entire outer edge of that individual "
    "silhouette.\n\n"
    "Construction: classic soft-enamel manufacture — colored enamel paint "
    "filled into recessed wells and air-dried, sitting noticeably below "
    "raised metal divider lines and the die-cut outer rim. Metal lines have "
    "visible height above the fill, catching crisp specular highlights along "
    "their top edge while casting a hairline shadow onto the enamel below. "
    "Enamel fills read matte to semi-gloss with slight meniscus pooling "
    "against the metal walls; the surface is textured and stepped, not "
    "polished flush like hard enamel.\n\n"
    "Material: opaque soft enamel with a slightly satin finish — colors stay "
    "rich but not glassy; subtle pooled depth in each well without vitreous "
    "shine or mirror-smooth polish.\n\n"
    "Composition: believable pin proportions — overlapping elements, subtle "
    "depth layering within the pin plane, and natural spacing rather than "
    "flat icon-badge layout. Photographic studio realism."
    "Strong silhouette readability."
)


# Per-rarity metal profile. Keys match src/core/relic.rs Rarity variants.
# The outer rim, raised metal lines, and any negative-space substrate all read
# as this metal; only the recessed soft-enamel fills vary per pin.
METAL_PROFILES = {
    "Common": (
        "Metal tier (Common — Iron): the raised divider lines and die-cut outer "
        "rim read as blackened wrought iron with a subtle hammered texture "
        "along the pin edge. Highlights are cool steely white; negative-space "
        "substrate shows as dark gunmetal with a soft brushed grain. Metal "
        "line tops catch a narrow hard specular."
    ),
    "Uncommon": (
        "Metal tier (Uncommon — Copper): the raised divider lines and die-cut "
        "outer rim read as polished rose copper with a warm amber patina "
        "settling into recesses. Highlights are warm peach-white; "
        "negative-space substrate shows as burnished copper with a soft radial "
        "brush. Metal line tops hold a long warm specular roll."
    ),
    "Rare": (
        "Metal tier (Rare — Silver): the raised divider lines and die-cut outer "
        "rim read as polished sterling silver with a cool white sheen. "
        "Highlights are bright cool-white; negative-space substrate shows as "
        "brushed silver with a faint radial burnish. Metal line tops catch a "
        "crisp cool specular, and recessed enamel wells pick up a subtle cool "
        "reflection near the walls."
    ),
    "Legendary": (
        "Metal tier (Legendary — Gold): the raised divider lines and die-cut "
        "outer rim read as polished jeweler's gold with a warm buttery tone. "
        "Highlights are warm ivory-gold; negative-space substrate shows as "
        "brushed gold with a soft radial burnish warming from the center "
        "outward. Metal line tops hold a long luminous specular, and recessed "
        "enamel wells carry a faint gold reflection along their inner edges."
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
RELIC_RS_PATH = REPO_ROOT / "crates" / "mahjuro-core" / "src" / "core" / "relic.rs"
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
        "Ivory tile faces, crackling gold lightning, muted crimson marks.",
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
        "Ivory tile faces, bamboo green marks, warm gold light streak, subtle amber sparks.",
    ),
    (
        "pair_power",
        "Pair Power",
        "A soft-enamel pin. Two mirror-twin magical girls mid-transformation: "
        "stylized enamel-glyph silhouettes in frilled battle dresses and long twin "
        "ponytails, arms raised as ribbon-light and sparkle halos burst from their "
        "shoulders. Wands cross at a vertical radiant seam; shockwave rings ripple "
        "outward in perfect symmetry. Crest-shaped die-cut with deep indigo void.",
        "gradient from deep indigo to hot magenta, lavender halos, gold shockwave rings, rose ribbon light, copper spark flares.",
    ),
    (
        "honor_fury",
        "Honor Fury",
        "A pin of three eastern dragons in a triple ouroboros — each biting "
        " the next dragon's tail in a closed ring with three-fold rotational "
        "symmetry. One mother-of-pearl white dragon, one carnelian dragon, "
        "one jade dragon.",
        "Mother-of-pearl iridescent white dragon, carnelian dragon, and "
        "jade dragon; orange-gold flame, spark accents.",
    ),
    (
        "dragon_rage",
        "Dragon Rage",
        "A soft-enamel pin shaped like three upright dragon honor "
        "tiles locked in a tight triplet — center chun (red dragon glyph) "
        "flanked by green and white dragon marks. Each tile reads as its own "
        "recessed enamel well bounded by raised metal lines; the center well "
        "reads hottest with saturated crimson soft enamel and tiny separate "
        "spark cells. A coiled eastern dragon is hinted in the routing of "
        "metal lines and negative metal space between the tiles. The pin outline "
        "follows the triplet's natural rectangular silhouette; realistic "
        "pin proportions, not a generic square frame.",
        "Crimson, jade, and cool white-blue honor enamels; soft amber rim light.",
    ),
    (
        "green_luck",
        "Green Luck",
        "A single upright mahjong tile with the green bamboo 1 tile: the peacock,"
        " beside a neat stack of gold coins and a few loose coins. "
        "Lucky green-and-gold charm. Crisp pin, everything sharp and in "
        "focus.",
        "Ivory tile face, jade-green peacock mark, warm gold coins, soft amber rim light.",
    ),
    (
        "plain_dealing",
        "Plain Dealing",
        "A soft-enamel pin of seven upright mahjong tiles in a neat row on "
        "emerald felt — every face a middle simple rank only (2 through 8): "
        "no winds, no dragons, no terminals 1 or 9. The tiles read as honest "
        "number-suit faces with crisp pips and character strokes, evenly "
        "spaced like a dealer's straight layout. Subtle warm copper rim light; "
        "Crisp pin, everything sharp and in focus.",
        "Ivory tile faces, jade and cobalt pip enamel, black character ink, warm copper rim light, deep emerald felt shadow.",
    ),
    (
        "white_dragons_hush",
        "White Dragon's Hush",
        "A soft-enamel pin of a giant eastern serpent asleep in a loose "
        "coil, its long body filling the die-cut silhouette from horned "
        "head to tapering tail. Scales are pale ivory and pearl white, "
        "each scale outlined in deep blue enamel like the carved border "
        "of a blank white-dragon (haku) mahjong tile. Eyes closed; no "
        "menace — only stillness. A faint cool moonlit halo traces the "
        "coil. Two small upright blank-faced white dragon tiles nestle "
        "in the curve of its neck. A single face-up zodiac tile rests "
        "beside the lowest loop, as if drawn from quiet. The pin outline "
        "follows the serpent's resting silhouette — organic, elongated, "
        "asymmetric.",
        "Pale ivory and pearl-white scale fills, deep blue scale-edge enamel, cool moonlit rim highlights, muted jade shadow in coil recesses, ivory white-dragon tile wells.",
    ),
    (
        "joker_tile",
        "Joker Tile",
        "A single mahjong tile standing upright at the center of an emerald "
        "felt table, its face split into four uneven quadrants showing "
        "ghostly impressions of different tile faces — a bamboo stick, a "
        "dot pips, a character glyph, and a wind arrow — overlapping like "
        "shifting reflections. A faint prismatic shimmer plays across the "
        "ivory surface as if the tile cannot decide what it is. A few stray "
        "face-down wall tiles lie blurred in the background.",
        "Ivory tile face, prismatic shimmer, muted bamboo green / dots blue / character crimson marks.",
    ),
    (
        "strength_in_numbers",
        "Strength in Numbers",
        "A beetle phalanx rendered as a tight military formation — no table, "
        "no felt, no ground plane. Dozens of glossy scarab beetles "
        "lock shields in overlapping ranks — a tall front wall of upright "
        "beetles, side files angled outward like hoplite wings, and a "
        "curving carapace roof of beetles on top in testudo formation. "
        "Each beetle is a separate recessed enamel cell with raised metal "
        "dividers; iridescent green-gold elytra catch crisp specular "
        "highlights. A few stragglers crawl toward the formation at the "
        "base. Strong silhouette readability at pin scale.",
        "Iridescent jade-green and warm gold beetle elytra, dark umber legs and antennae, soft amber rim light, cool pale shadows between ranks.",
    ),
    (
        "quick_draw",
        "Quick Draw",
        "A vintage editorial caricature pin — bold pen-and-ink exaggeration "
        "in the spirit of nineteenth-century political cartoons and Wild "
        "West broadsheets. A lanky gunslinger figure frozen mid-draw: "
        "impossibly long spurred legs, a tiny tilted hat, a beak-like nose, "
        "and enormous gloved hands. One hand blurs toward a low row of "
        "face-down mahjong wall tiles treated like a holstered belt; a single "
        "tile rockets out in a speed-line arc, spinning face-up with its "
        "glyph just visible — the tile is the 'pistol.' Squiggly motion "
        "hatching and starburst speed marks fill the negative space. "
        "Grotesque, funny, readable silhouette; not realistic photography.",
        "Ink-black linework enamel, warm cream and ivory tile, deep emerald "
        "felt band, copper-rim speed lines, muted sepia shadows, small crimson "
        "accent on the spur or hat band.",
    ),
    (
        "chain_reaction",
        "Chain Reaction",
        "A vintage atomic-age nuclear chain-reaction schematic as a soft-enamel "
        "pin: a large Bohr-model atom at center — dense copper-red nucleus, "
        "three crisp electron orbit rings in cool teal enamel — with neutron "
        "dots racing along branching dotted trajectories to two smaller "
        "satellite atoms at the corners. Each impact triggers amber-gold "
        "fission sparks and concentric blast rings that link the atoms in a "
        "cascading cascade. Clean mid-century science-poster diagram fills, "
        "strong silhouette readability, everything sharp and in focus.",
        "Copper-red nucleus, cool teal orbital rings, amber-gold fission sparks "
        "and blast rings, ivory neutron paths, muted slate diagram background.",
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
        "Two kabuki wind-spirits in perfect left-right mirror symmetry on a "
        "vertical axis — identical mie poses and proportions, each the mirror "
        "twin of the other. White oshiroi base faces with bold red kumadori "
        "paint, arched brows, and exaggerated puffed cheeks; long black stage "
        "wigs and flowing indigo happi coats whip outward. They face inward, "
        "blowing fierce opposing gales that meet in a turbulent center swirl. "
        "The colliding gusts and crest silhouette read with 180-degree "
        "rotational symmetry. Ukiyo-e poster boldness translated into soft enamel.",
        "Pearl white oshiroi, crimson kumadori, deep indigo coats and wind "
        "streaks, jet-black wig enamel, warm gold gust highlights, drifting petals.",
    ),
    (
        "dragon_echo",
        "Dragon Echo",
        "A triplet of upright red dragon mahjong tiles with glowing "
        "crimson chun glyphs. Ghostly echo copies stagger behind in "
        "fading arcs, like a reverberating roar. Crisp pin, everything "
        "sharp and in focus.",
        "Ivory tile faces, glowing crimson dragon glyphs, ghostly amber echo arcs, ember sparks.",
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
        "A mahjong-tile princess doll: a face-up dora indicator tile is her "
        "dress, with a small painted porcelain head, tiny arms, and a gold "
        "crown. Cute 1920s figurine, crisp and fully in focus.",
        "Ivory tile dress, crimson dora glyph, porcelain skin, gold crown, pastel pink accents.",
    ),
    (
        "wind_reader",
        "Windreader",
        "A woman in a flowing indigo and ivory robe floats above a low "
        "lacquered plinth, one arm extended upward, index finger pointing "
        "into layered storm clouds. Two distinct wind currents part the "
        "clouds in different directions — one bearing a faint East wind "
        "mahjong glyph, the other a South wind glyph — as if she reads "
        "two round winds at once. Four tiny wind tiles ring the plinth "
        "below. Macro product shot with infinite depth of field — figure, "
        "clouds, glyphs, and fabric folds equally tack-sharp edge to edge.",
        "Porcelain skin, indigo robe enamel, pearl cloud layers, deep blue wind glyphs, warm amber sky glow.",
    ),
    (
        "eight_treasures",
        "Eight Treasures",
        "An ornate open treasure chest ringed with the eight ba-bao auspicious "
        "emblems — pearl, double coins, coral, stone chime, knot, vase, fan, "
        "and cloud — each a tiny distinct charm on the rim. Warm light spills "
        "from inside; curling zodiac ribbons spill over the edge. Crisp pin, "
        "everything sharp and in focus.",
        "Dark lacquer chest, gold filigree, eight colored treasure charms, warm amber glow, curling ribbon accents.",
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
        "Clear glass cloche dome with beaded condensation, ivory tile face, soft pastel pink plum and jade orchid blossoms, curling green leaves, warm amber grow-light underglow.",
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
        "A soft-enamel pin shaped like a miniature cherry tree growing "
        "from a small lacquered black pot — gnarled dark trunk, delicate "
        "spreading branches. Every blossom is a small upright mahjong flower "
        "tile clipped to the branch, ivory enamel faces painted with plum, "
        "orchid, bamboo, and chrysanthemum motifs in soft pastel pink and "
        "jade. A few fallen flower tiles and neat stacks of warm gold coins "
        "rest at the tree base. Loose pink petals drift around the canopy. "
        "The pin outline follows the tree's organic canopy silhouette; "
        "realistic depth and layering within the pin plane.",
        "Dark gnarled cherry trunk and branches, lacquered black pot, ivory mahjong flower-tile blossoms with pastel pink and jade glyphs, warm gold coin stacks, drifting pink petal accents, soft amber rim light.",
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
        "ruby_serpent",
        "Ruby Serpent",
        "A glazed porcelain serpent figurine coiled around a single mahjong "
        "character tile, its body sculpted from creamy white china with "
        "fine crackle glaze and inlaid scales of polished carved ruby and "
        "carnelian. Its eyes are tiny faceted rubies catching the light. "
        "The figurine rests on a dark lacquer plinth against a soft, "
        "neutral studio backdrop.",
        "Cream porcelain body, polished ruby and carnelian scales, faceted ruby eyes, ivory tile with dark ink character, dark lacquer plinth, neutral studio backdrop.",
    ),
    (
        "lapis_serpent",
        "Lapis Serpent",
        "A glazed porcelain serpent figurine coiled around a single mahjong "
        "dots tile, its body sculpted from creamy white china with "
        "fine crackle glaze and inlaid scales of polished lapis lazuli and "
        "sapphire. Its eyes are tiny faceted sapphires catching the light. "
        "The figurine rests on a dark lacquer plinth against a soft, "
        "neutral studio backdrop.",
        "Cream porcelain body, polished lapis and sapphire scales, faceted sapphire eyes, ivory tile with blue dot pips, dark lacquer plinth, neutral studio backdrop.",
    ),
    (
        "low_tide",
        "Low Tide",
        "Landscape-oriented wide horizontal pin badge: a small coastal "
        "survey boat in profile on exposed tidal mud flats, spanning left "
        "to right. Squat workboat hull, cabin roof with a compact radar "
        "dome and short survey antenna — no tall sail mast. Measuring "
        "seasons in the mud under a pale open sky. Flat grey estuary light. "
        "Crisp pin, everything sharp and in focus.",
        "Dark hull, brown mud, white measuring stakes, small radar dome, short antenna, pale sky, grey flat light.",
    ),
    (
        "high_tide",
        "High Tide",
        "Landscape-oriented wide horizontal pin badge: a small coastal "
        "survey boat in profile afloat on a risen tidal estuary, spanning "
        "left to right. Squat workboat hull, cabin roof with a compact "
        "radar dome and short survey antenna — no tall sail mast. "
        "Measuring-stake tops barely show above the water under a pale "
        "open sky. Flat grey estuary light. Crisp pin, everything sharp "
        "and in focus.",
        "Dark hull, grey water, white stake tips, small radar dome, short antenna, pale sky, grey flat light.",
    ),
    (
        "even_keel",
        "Even Keel",
        "Landscape-oriented wide horizontal pin badge — companion to Low "
        "and High Tide: the same squat coastal survey boat in profile on "
        "calm mid-tide water, perfectly level on the surface with no list "
        "or roll. Measuring stakes half-submerged in a neat row; the waterline "
        "sits evenly between mud and crest. Three small ivory mahjong tiles "
        "with middle ranks (4, 5, 6 pips) float as tiny buoys along the "
        "wake. Flat grey estuary light, crisp pin, everything sharp.",
        "Dark hull, grey-green calm water, half-submerged white stakes, ivory tile buoys with blue dot pips, pale sky, grey flat light.",
    ),
    (
        "merchants_eye",
        "Merchant's Eye",
        "A Victorian pawn-shop curio: a glossy blown-glass prosthetic eye "
        "with a vivid painted amber-hazel iris and pinprick pupil, resting "
        "on emerald felt beside a single face-up mahjong tile. The glass "
        "orb catches a sharp studio highlight as if appraising the tile. "
        "A tiny cream price tag on red string dangles from a short brass "
        "display hook. A few scattered gold coins and face-down wall tiles "
        "around the edges. Crisp pin, everything sharp and in focus.",
        "Milky glass sclera, amber-hazel iris, jet pupil highlight, ivory tile face, warm gold coins, cream price tag, red string, brass hook, soft amber lamplight on the glass.",
    ),
    (
        "i_got_a_guy",
        "I Got A Guy",
        "A creased paper business card for Otto's Pawn Shop and a scrap "
        "with strange symbols scribbled in ink, clipped together with a bent paperclip."
        "Phone number is not readable.",
        "Cream paper, black ink scrawl, steel paperclip, emerald felt, warm counter light.",
    ),
    (
        "edge_runner",
        "Edge Runner",
        "Two upright mahjong circle-suit tiles at opposite ends, linked by a "
        "taut gold light arc like a tightrope with sparks along the span. "
        "Left tile: exactly one large centered blue dot (pinzu 1). Right "
        "tile: exactly nine blue dots in a perfect 3×3 grid — three rows of "
        "three, no extra dots (pinzu 9). Small corner numerals 1 and 9. "
        "Crisp pin, everything sharp and in focus.",
        "Ivory tile faces, one large center dot, nine dots in 3×3 grid, deep blue pips, warm gold light arc, amber sparks.",
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
        "A soft-enamel pin of a colossal ancient swamp turtle half-sunk in "
        "misty bog water — the Morla-style great old turtle from classic "
        "fantasy cinema. Its domed carapace reads as a weathered earthen "
        "hill: layered moss, ferns, twisted roots, and pale driftwood "
        "caught along the rim. One massive wrinkled head breaks the "
        "surface beside the shell, deep-set patient eyes half-lidded with "
        "indifference. Reeds and lily pads ring the waterline; grey-green "
        "fog softens the background. The pin's die-cut silhouette follows "
        "the turtle's organic mound-and-head outline.",
        "Mossy olive and peat-brown shell hill, jade fern and root enamel, "
        "bleached bone ivory accents, muddy grey-green swamp water, reedy "
        "chartreuse highlights, wrinkled grey-olive skin, soft amber rim light.",
    ),
    (
        "closed_gate",
        "Closed Gate",
        "A soft-enamel pin: four upright mahjong tiles in a tight row — "
        "1 of bamboo, 9 of dots, red dragon (chun), East wind — each tile "
        "its own recessed enamel well bounded by raised metal lines. Behind "
        "the row rises a heavy ornamental Chinese gate with a curved roof "
        "and lattice panels; its two doors are fully closed and flush in "
        "the center with no gap or opening, barred shut as if blocking "
        "middle ranks from passing. The gate frame and tile row define the "
        "pin's crest-shaped silhouette; worn smooth at the edges from "
        "countless late-night rounds.",
        "Ivory tile faces, deep blue dot and wind glyphs, crimson dragon enamel, muted bamboo-green 1-bam, antique gold gate filigree, soft amber rim light.",
    ),
    (
        "open_gate",
        "Open Gate",
        "A soft-enamel pin paired with Closed Gate: four upright mahjong "
        "tiles in a tight row — 4, 5, and 6 of dots plus one more middle "
        "simple rank — each tile its own recessed enamel well bounded by "
        "raised metal lines. Behind the row rises the same heavy ornamental "
        "Chinese gate with curved roof and lattice panels, but both doors "
        "stand fully open with a clear passage through the center, welcoming "
        "simple tiles through. Warm light spills from the opening onto the "
        "felt. Crest-shaped silhouette, worn smooth at the edges.",
        "Ivory tile faces, deep blue dot pips only (no honors), antique gold gate filigree, warm amber light from the open passage, soft rim highlights.",
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
        "Polished brass machine body, glass dome, warm gold coin stacks feeding the hopper, pinned brass pressure gauge, golden exhaust vapor with drifting gold flecks, soft amber rim light.",
    ),
    (
        "snowball",
        "Snowball",
        "A massive sphere built entirely from packed mahjong tiles, "
        "rolling forward in a freeze-frame burst. Tiles of every suit — "
        "bamboos, dots, characters, winds, dragons — are jammed "
        "together at every angle, ivory faces and lacquered backs "
        "pressed into the curve, with a few loose tiles tumbling along "
        "to be absorbed at the leading edge. A widening trail of fallen "
        "tiles curves behind it in the dark background.",
        "Ivory tile faces, lacquered tile backs, multicolored suit marks, warm amber rim light, cool pale shadows.",
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
        "Painted tin toy car, chrome bumpers, rubber tires, ivory tile face, warm gold spark trail, motion-blur streak, soft amber rim light.",
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
        "An abstract soft-enamel pin — no felt table, no fanned hand. At the "
        "center, the traditional Chinese character 氣 (qi — breath, vital "
        "energy) rendered as a bold molten-gold brushstroke sigil, not inside "
        "a tile rectangle. Three concentric echo-rings orbit it: each ring "
        "bears a simplified ghost-outline of the same 氣 character, "
        "progressively fainter toward the outside, telegraphing one final "
        "exhale that repeats. Pale jade and warm-gold vapor ribbons curl "
        "counterclockwise through the rings like breath on cold air. The "
        "die-cut silhouette follows the spiral and rings organically — "
        "asymmetric, crest-shaped, with deep midnight-teal void between the "
        "curls. A few drifting gold sparks catch the silver rim.",
        "Midnight-teal void, molten-gold 氣 character, pale jade breath ribbons, fading gold echo rings, translucent ghost 氣 strata, drifting gold sparks, cool silver rim light.",
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
        "stone_lantern",
        "Stone Lantern",
        "A traditional Japanese ishidōrō stone lantern — stacked carved "
        "stone roof and pillar, hollow light chamber with openings, weathered "
        "granite texture."
        "readable as a single centered badge.",
        "Weathered gray granite and mossy stone, warm amber and soft gold "
        "candlelight glowing from the chamber interior, muted moonlit sage "
        "and jade moss with small fern fronds clustered at the stone base "
        "as part of the pin composition, cool blue-gray rim light on stone edges.",
    ),
    (
        "mirror_tile",
        "Mirror Tile",
        "One oversized blank mahjong tile as a soft-enamel pin — a tall "
        "rounded rectangle, ivory face completely empty (no suit mark on "
        "the giant tile). Its entire front is skinned over by a tight "
        "mosaic of miniature mirror tiles: hundreds of tiny square "
        "mirror-facet cells with thin gold grout, each cell a bright "
        "silver reflective square catching sharp diagonal glints — like "
        "a wall of tiny mirrored mahjong chips laid flat on the blank "
        "carrier. No dots, bamboo, or characters on the mosaic cells "
        "themselves — only mirror shine. A few pale blue reflections and "
        "white star sparkles where the mirrors catch the light. Raised "
        "gold die-cut rim around the giant tile. Upright, symmetrical, "
        "readable at pin scale.",
        "Pearl ivory blank giant carrier, bright silver mirror mosaic "
        "cells, warm gold grout and outer rim, pale blue glints, soft "
        "amber rim light.",
    ),
    (
        "way_of_purity",
        "Way of Purity",
        "A soft-enamel pin in the Funerary Row set — match the sculptural "
        "punch of Way of Pairs: late-Ming woodblock engraving translated into "
        "raised metal lines, Holbein Dance of Death austerity. A tall narrow "
        "tomb-crest silhouette of four identical stone funeral masks stacked "
        "vertically, each mask carved with the same calm closed-eye expression "
        "and the same deep cinnabar-red lacquer shroud draped over brow and "
        "cheek — one funeral hue only, no mixed colors. Forehead name cartouches "
        "are chiseled blank ivory voids. A single silver burial cord threads "
        "through all four masks. Rain-slick blue-grey stone plinth; a black "
        "enamel puddle at the foot. Deep undercut shadows in eye sockets and "
        "mouth; moss in carved creases. No living figure, no flat drawer-stack "
        "geometry, no English text.",
        "Sculpted blue-grey stone masks, deep cinnabar shroud enamel, blank "
        "ivory forehead wells, hollow charcoal eye sockets, cool silver cord and "
        "rim, black rain pool, moss-green crevice accents.",
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
        "Ivory tile faces, traditional crimson man-zu numerals, pale teal echo silhouettes, soft gold ripples, warm amber rim light.",
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
        "Ivory tile faces, traditional crimson man-zu numerals, pale crimson echo silhouettes, soft gold ripples, warm amber rim light.",
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
        "Warm amber rim light; natural pin composition with the nest "
        "defining the outer silhouette.",
        "Matte white goose egg, grassy bowl nest with moss and lichen, down-lined cup, "
        "straw and olive tones, warm amber rim light, subtle motion hint.",
    ),
    (
        "tea_ceremony",
        "Tea Ceremony",
        "Paired chanoyu set (1 of 2 — the other pin is the Rakuware relic): use "
        "the **same bowl scale and three-quarter view from slightly above** as "
        "the companion pin so they line up side by side as one story. Subject: "
        "a **smooth refined chawan** (porcelain or soft celadon), centered — "
        "the bowl that **precedes** the rustic raku piece in the pair. **Four** "
        "delicate rising steam wisps (not three), each whisper-tinted a "
        "different muted hue — sage green, pale shell pink, cool mist blue, "
        "warm ivory — suggesting the four guiding spirits of the ceremony "
        "without text or symbols. No wooden rest, no kiln drama; calm, "
        "luminous, **still steaming**. The pin outline follows the bowl and "
        "steam plume; subject only, no tabletop scene.",
        "Cream porcelain, muted celadon glaze, four subtly tinted steam wisps, "
        "soft gold lip line, pale warm neutral void — **must match the void "
        "tone and framing of the Rakuware pin** in the same matched pair.",
    ),
    (
        "rakuware",
        "Rakuware",
        "Paired chanoyu set (2 of 2 — the other pin is the Tea Ceremony relic): "
        "use the **same bowl scale and three-quarter view from slightly above** "
        "as the companion pin so they line up side by side as one story. "
        "Subject: a **hand-built raku chawan** — the bowl that **follows** "
        "the refined steaming bowl in the pair: bold crackle glaze, warm "
        "charcoal reduction, soft coppery flash at the rim. **No steam** — "
        "the tea is finished; the clay has cooled. A single hairline crack "
        "traced in fine metallic gold — kintsugi hint, not a full repair. "
        "Optional **small dark wooden rest** tucked beneath the bowl. The pin "
        "outline follows the bowl shape; same calm negative space as the "
        "Tea Ceremony pin.",
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
        "A Northern Renaissance allegory pin after Hans Sebald Beham's Humilitas "
        "engraving tradition: a hooded pilgrim in three-quarter view turned "
        "**away** from honor tiles — winds and dragons pushed face-down behind "
        "the figure, not worn. An ornate crown with a crimson dragon glyph lies "
        "trampled in mud behind. The path ahead shows numbered bamboo, character, "
        "and dot tiles only — no honors on the road. Compact vertical "
        "cameo silhouette like the original print (~10×7 cm proportions), with fine "
        "cross-hatched shading suggested in recessed enamel wells rather than flat "
        "cartoon fills. Black enamel background. "
        "At the feet, one honor tile deliberately left unplayed, face-down. "
        "Moralizing, sculptural, legible at pin scale.",
        "Warm aged ivory robes, deep umber cross-hatch enamel, muted slate honor "
        "tile backs, trampled gold crown recesses, plain ivory numbered tiles on "
        "the path, cool iron rim highlights.",
    ),
    (
        "obsession",
        "Obsession",
        "An eye motif locked onto a single ornate yaku sigil, intense and "
        "focused, rendered as a realistic soft-enamel pin with natural depth.",
        "Muted ivory eye, crimson focal ring, dark navy outlines, warm brass accents.",
    ),
    (
        "bonfire",
        "Bonfire",
        "A stacked pyre of tiles and wood burning upward, flame shape defining "
        "the pin's outer silhouette.",
        "Orange flame, charcoal black embers, warm wood browns, gold highlights.",
    ),
    (
        "river_runner",
        "River Runner",
        "A swift river current curling around a sequence of tiles, showing "
        "flow and forward motion; the meandering stream defines the pin outline.",
        "Teal water ribbons, ivory tiles, deep blue shadows, silver highlights.",
    ),
    (
        "melting_ice",
        "Melting Ice",
        "A faceted ice crystal or hex shard as a soft-enamel pin — sharp "
        "geometry with believable ice translucency and depth. The lower edge "
        "breaks into melting drip and bead shapes (enamel teardrops). At the "
        "heart, suggest something sleeping through negative space — twin "
        "shallow crescent horn curves and two small bronze enamel dots for "
        "eyes inside one pale recessed cell. Hairline wire cracks divide "
        "facets. The crystal's irregular silhouette defines the pin outline.",
        "Pale arctic blue and frost-white enamel facets, cool grey crack lines "
        "in wire, copper-bronze horn curves and eye dots, translucent aqua "
        "drip beads along the lower edge, soft amber rim light.",
    ),
    (
        "taotie",
        "Taotie",
        "A soft-enamel pin of a broad taotie mask with curling horns and a "
        "wide malicious grin. Three honor mahjong tiles — red dragon, green "
        "dragon, white dragon — sit in its open jaws only, each a small "
        "recessed enamel well with raised metal outlines. The "
        "mask defines the die-cut pin silhouette.",
        "Warm jade-green and bronze enamel fills, ivory tile faces, crimson "
        "and emerald dragon glyphs, copper raised metal lines, soft amber rim "
        "light.",
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
        "Faded crimson and umber silks, ivory tile face, warm amber rim light, soft gold sparks, muted crimson glyph glow.",
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
        "Polished jade beads, warm brass frame, warm gold coin stacks, ivory tile with jade-green glyph, dark wood accents.",
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
        "A Northern Renaissance allegory pin after Hans Sebald Beham's 1540 "
        "engraving Pacientia (Patience): a calm winged woman seated in three-quarter "
        "view, gently cradling a small lamb — the traditional attribute of gentleness "
        "and docile endurance. Behind her shoulder, a small horned imp or demon "
        "taunts with a grimace but cannot break her composure; above, two tiny putti "
        "hover as one lowers a laurel victory wreath toward her head. Compact vertical "
        "cameo silhouette like the original print (~10×7 cm proportions), with fine "
        "cross-hatched shading suggested in recessed enamel wells rather than flat "
        "cartoon fills. Black enamel background. At her feet, a short neat stack of unused face-down mahjong "
        "tiles nods to waiting out the round. Moralizing, sculptural, legible at "
        "pin scale — engraved draftsmanship translated into raised metal lines.",
        "Warm aged ivory flesh and lamb wool, deep umber cross-hatch enamel, muted "
        "slate-blue demon, pale laurel-green wreath, soft cream wing feathers, cool "
        "iron rim highlights on the figures.",
    ),
    (
        "kindling",
        "Kindling",
        "A small bundle of twigs tied with twine, a single ember glowing at "
        "the base where a tiny flame is just catching. Each cash-in adds a "
        "new twig to the growing pile — the pin's silhouette widens slightly "
        "at the base like an accumulating stack. Cozy, organic, pre-fire "
        "energy rather than full blaze.",
        "Warm amber ember glow, tan-brown twig bundles, rust-red twine, "
        "faint orange flame, dark ashy shadow base.",
    ),
    (
        "kindness",
        "Kindness",
        "A Northern Renaissance allegory pin after Hans Sebald Beham's Misericordia "
        "tradition: a calm figure with open palm offering a mahjong tile toward an "
        "empty seat or unseen companion — gentle sheltering posture, Compact vertical "
        "cameo silhouette like the original print (~10×7 cm proportions), with fine "
        "cross-hatched shading suggested in recessed enamel wells rather than flat "
        "cartoon fills. Black enamel background. At the feet, **five** face-down wall "
        "tiles in a tidy discard row: four base tiles plus one extra clearly separated "
        "or highlighted — the spare discard granted by the relic. Moralizing, "
        "sculptural, legible at pin scale.",
        "Warm aged ivory flesh, deep umber cross-hatch, muted teal spare tile "
        "highlight, ivory tile faces, cool iron rim highlights.",
    ),
    (
        "temperance",
        "Temperance",
        "A Northern Renaissance allegory pin after Hans Sebald Beham's Temperantia "
        "tradition: a winged figure bridling a small imp or restraining a cup with "
        "one hand stayed against rash haste — measured composure, compact vertical "
        "cameo silhouette like the original print (~10×7 cm proportions), with fine "
        "cross-hatched shading suggested in recessed enamel wells rather than flat "
        "cartoon fills. Black enamel background. At the feet, unused play tally: stone "
        "counters or face-down tiles still on a tray, not spent, beside a scroll or "
        "laurel ring with etched hash marks suggesting growth across blinds. Mirror "
        "Patience's saved-resource layout but for plays, not discards.",
        "Warm aged ivory flesh, deep umber cross-hatch, muted slate-blue imp, pale "
        "laurel-green growth marks, cool iron rim highlights.",
    ),
    (
        "chastity",
        "Chastity",
        "A Northern Renaissance allegory pin after Hans Sebald Beham's Castitas "
        "tradition: a veiled woman holding an upright lily — purity without "
        "ornament. Compact vertical cameo silhouette like the original print (~10×7 cm proportions), with fine "
        "cross-hatched shading suggested in recessed enamel wells rather than flat "
        "cartoon fills. Black enamel background. Scored props "
        "are plain ivory mahjong tiles only: **no** pearl, gold leaf, talisman seal, "
        "or polychrome enhancement in any well. At the feet, a pair of unadorned "
        "simple tiles and one rejected gilded tile lying aside, unused. Matte fills "
        "only on tiles.",
        "Warm aged ivory flesh and veil, deep umber cross-hatch, pale lily-green, "
        "plain ivory tile wells, dull rejected gold tile aside, cool iron rim.",
    ),
    (
        "chow_line",
        "Chow Line",
        "A soft-enamel pin: ornate three-tier bronze display stand with "
        "scalloped copper frame, red enamel recesses, and fan motifs at the "
        "base. Twelve cream mahjong tiles on three curved shelves — each tile "
        "shows a sequence chow as sushi icons (1, 2, then 3 pieces): top row "
        "salmon nigiri; middle row cucumber slices and shrimp nigiri; bottom "
        "row ikura gunkan. Glowing orange paper lanterns with red tassels "
        "flank the middle tier. Warm lantern light on dark background.",
        "Aged bronze and copper frame, deep red enamel wells, cream ivory "
        "tile faces, orange salmon and shrimp, green cucumber, red-orange roe, "
        "warm lantern glow, soft amber rim light.",
    ),
    (
        "charity",
        "Charity",
        "A Northern Renaissance allegory pin after Hans Sebald Beham's Caritas "
        "tradition: a figure pouring coins from a **nearly empty purse** into a "
        "beggar's bowl at dawn — thin sunrise rim along the cameo edge. Only a few "
        "coins remain in the purse; a stream of **five** new coins falls into the "
        "bowl. Compact vertical cameo silhouette like the original print (~10×7 cm proportions), with fine "
        "cross-hatched shading suggested in recessed enamel wells rather than flat "
        "cartoon fills. Black enamel background. At the feet, an "
        "empty coin stack beside a small fresh pile of five — broke before the gift.",
        "Warm aged ivory flesh, deep umber cross-hatch, thin gold glints on the new "
        "five coins only, muted rose dawn rim, cool iron highlights.",
    ),
    (
        "diligence",
        "Diligence",
        "A Northern Renaissance allegory pin after Hans Sebald Beham's Industria "
        "tradition: a steady worker at a loom or beehive — traditional diligence "
        "attributes, compact vertical cameo silhouette like the original print (~10×7 cm proportions), with fine "
        "cross-hatched shading suggested in recessed enamel wells rather than flat "
        "cartoon fills. Black enamel background. At the feet, "
        "**six** tick marks on a work ledger or six ready tiles in a play line — base "
        "five plays plus one extra — or a hand reaching to place a sixth tile while "
        "five already sit committed. The bonus play waiting to be used.",
        "Warm aged ivory flesh, deep umber cross-hatch, muted honey-amber beehive or "
        "walnut loom wood, ivory tile faces, cool iron rim highlights.",
    ),
    (
        "way_of_pairs",
        "Way of Pairs",
        "A soft-enamel pin in the Funerary Row set, late-Ming woodblock "
        "staging with Holbein Dance of Death symmetry: a crest-shaped "
        "silhouette of two weathered stone guardian lions seated in perfect "
        "mirror symmetry on rain-slick flagstones, flanking a narrow sealed "
        "tomb door with no gap — no face, no figure, only the paired beasts "
        "and the shut lintel. Each lion bears the same worn collar crest; "
        "moss in paw creases. Fine drizzle streaks on stone; a hairline crack "
        "on one base only, not breaking the symmetry of pose.",
        "Blue-grey wet stone, muted moss-green recesses, pale ivory worn "
        "carving highlights, dark charcoal door well, cool silver outer rim, "
        "soft amber rim light on rain.",
    ),
    (
        "way_of_triplets",
        "Way of Triplets",
        "A soft-enamel pin in the Funerary Row set — match Way of Pairs "
        "exactly: same rain-slick flagstone base, same architectural tomb "
        "staging, late-Ming woodblock engraving, Holbein austerity. EXACTLY "
        "THREE closed bronze funeral urns and nothing else — no fourth urn, "
        "no human face, no bust, no commemorative plaque frame. The three "
        "urns sit in a tight equilateral triangle on wet stone, each urn "
        "identical: squat round body, domed lid, and the same cast taotie "
        "beast knocker ring on the front. A single silver burial wire threads "
        "through all three rings as one offering. A small black enamel void "
        "gapes in the center between the three urns. Identical cinnabar seal "
        "stamps on each lid. Moss in stone cracks, fine drizzle, worn metal "
        "patina. Triangular silhouette comes from the three round urn bodies "
        "only — not a pyramid of heads.",
        "Weathered bronze urn bodies, deep cinnabar lid seals, charcoal "
        "center void, blue-grey wet flagstone, cool silver wire and rim, "
        "moss-green stone cracks, soft amber rim light on rain.",
    ),
    (
        "way_of_sequences",
        "Way of Sequences",
        "A soft-enamel pin in the Funerary Row set — match the sculptural "
        "punch of Way of Pairs: late-Ming woodblock diagonal staging, Holbein "
        "Dance of Death descent. A wedge-shaped silhouette of three steep "
        "rain-slick tomb steps descending left to right, each tread carved "
        "with a single Chinese grave numeral (一, 二, 三) in deep ivory inlay "
        "— NOT English words. A long skeletal bone-white handprint stain "
        "trails down the center of all three treads as if something was "
        "dragged downward into a black enamel pool at the bottom step. Worn "
        "stone undercut, moss in tread corners, cool silver step lips. No "
        "figure visible — only the descending numerals, the handprint trail, "
        "and the dark pool. No flat infographic stairs, no English text.",
        "Wet blue-grey stone treads, deep teal moss wells, pale ivory "
        "Chinese numeral inlays, bone-white handprint streak, black pool at "
        "base, cool silver step edging and outer rim.",
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
        "together but visibly unstable. Realistic tile proportions and wear.",
        "Aged ivory tile, charcoal crack, dusty ochre debris, faded ink.",
    ),
    (
        "star_tile",
        "Star Tile",
        "A mahjong tile transformed into a lucky celestial enamel pin, with "
        "a bold five-pointed star centered on the face and small radiant "
        "accent marks around it. The tile's natural rectangular silhouette "
        "defines the pin outline.",
        "Warm ivory tile body, gold star accents, deep navy details, soft amber highlights.",
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
        "Translucent pale ghost figure, distended belly, sunken hollow eyes, pinhole mouth, ivory tile dissolving into pale smoke, drifting gold sparks, smoldering red joss-paper embers, cool wraith-light, soft amber rim light.",
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
        "Ivory tile faces, deep blue East and West wind glyphs, pale sickly green wisp, muted charcoal shadows.",
    ),
    (
        "curio_cabinet",
        "Curio Cabinet",
        "A miniature glass-fronted display cabinet crammed with tiny assorted "
        "relics on stepped shelves, each visible through the door pane. A small "
        "brass keyhole sits at the center. The cabinet's rectangular glass "
        "front defines the pin outline.",
        "Warm mahogany frame, pale amber glass, brass fittings, muted multicolor shelf contents.",
    ),
    (
        "lotus_bloom",
        "Lotus Bloom",
        "A single stylized lotus flower in full bloom, layered petals radiating "
        "outward from a gold seedpod center, with a trailing stem curling below. "
        "The flower and stem define the organic pin silhouette; symmetrical.",
        "Soft pink petals, cream inner tones, gold seedpod, deep jade leaf accents.",
    ),
    (
        "wall_weaver",
        "Wall Weaver",
        "A loom frame weaving a tight lattice of tiny mahjong tiles together like "
        "fabric, with a shuttle paused mid-pass. The loom frame defines the "
        "pin's outer silhouette.",
        "Warm wood loom, ivory woven tiles, dark ink grid lines, muted gold shuttle.",
    ),
    (
        "kong_collector",
        "Kong Collector",
        "Four matching mahjong tiles stacked in a perfect square bundle, bound "
        "together by a tight gold cord with a hanging coin tassel. Trophy-like "
        "with the square bundle defining the pin outline.",
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
        "few stray face-down wall tiles rest in the background. Crisp pin, "
        "everything sharp and in focus.",
        "Anatomically opposing right-hand and left-hand pair (mirror images, NOT two right hands), distinct dark embroidered sleeve versus plain pale sleeve, ivory honor tile faces with deep blue wind and crimson dragon glyphs, warm gold coins in an open palm-up right hand, palm-down left hand sweeping tiles, soft amber rim light.",
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
        "passport stickers, a pair of leather straps crossing the lid. Worldly "
        "and well-traveled; the trunk shape defines the pin silhouette.",
        "Warm tan leather, dark brass corners, muted multicolor stamps, ivory highlights.",
    ),
    (
        "heirloom",
        "Heirloom",
        "A single antique upright mahjong tile: yellowed ivory face, worn "
        "shallow glyph, warm patina along the edges, tiny play notches on "
        "the sides. Crisp pin, everything sharp and in focus.",
        "Aged ivory tile face, soft worn glyph, warm amber patina, muted dark wood accents.",
    ),
    (
        "tourist",
        "Tourist",
        "A small brass compass lying atop a folded paper map, with a tiny camera "
        "and a luggage tag beside it. Travelogue motif with the compass-and-map "
        "cluster defining the pin outline.",
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
        "A small ivory plaque pin dominated by a single large engraved "
        "lowercase e in an upright classical serif, open counter, even stroke "
        "weight. Include a miniature visual proof in the same engraved draftsmanship: "
        "the curve y = 1/x from x = 1 toward e with cross-hatched or stippled area "
        "under that curve between 1 and e reading as exactly one unit square — the "
        "classic 'ln(e) = 1' picture. Along a margin or ribbon, tiny step marks can "
        "suggest (1 + 1/n)^n converging toward e. A fine spiral may still unwind "
        "from the letter as a secondary motif. The plaque's natural rectangular "
        "silhouette defines the pin outline.",
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
        "A pair of open hands — palms forward, fingers spread wide — "
        "carved in raised relief, as if offering or catching an oversized fan "
        "of mahjong tiles fanned above them. Generous scale, broad gesture; "
        "the hands and tile fan define the pin's outer silhouette.",
        "Warm copper raised metal lines, cream and jade soft-enamel fills, ivory tile hints, soft amber rim light.",
    ),
    (
        "tiny_hands",
        "Tiny Hands",
        "The same motif as Big Hands but at doll or Lilliputian scale: "
        "two minuscule delicate hands — short thin fingers, tiny palms — "
        "clearly dwarfed beneath a cluster of mahjong tiles that read large "
        "relative to the hands. Gestures tight and nimble, fingers close "
        "together. Same scale and metal tier as its twin for a matched set, "
        "unmistakably tinier than Big Hands.",
        "Warm copper raised metal lines, cream and jade soft-enamel fills, ivory tile hints, soft amber rim light.",
    ),
    (
        "chrysalis",
        "Chrysalis",
        "A monarch butterfly chrysalis — jade-green and gold-speckled pupa "
        "hanging from a short silk pad stem, matte shell with subtle "
        "segment lines, contained and still; no wings yet. Soft-enamel pin, "
        "centered, readable silhouette.",
        "Matte jade-green enamel, warm gold speckles, dark umber stem wrap, soft amber rim light.",
    ),
    (
        "monarch_butterfly",
        "Monarch Butterfly",
        "An open-wing monarch butterfly as a soft-enamel pin — vivid orange "
        "wings with black veins and white dots at the wing edges, wings "
        "spread symmetric, body slim and black. Product-shot centered, "
        "wings filling the frame.",
        "Bright orange and black soft-enamel wing bands, cream wing spots, copper raised metal lines, soft amber rim light.",
    ),
    (
        "ancestor_echo",
        "Ancestor Echo",
        "A ceremonial bronze temple bell suspended in profile, struck by a "
        "small carved wooden mallet. Three translucent afterimages of the "
        "same bell trail behind it in diminishing arcs, each echo ring offset "
        "from the last to suggest one powerful strike repeating through time. "
        "Fine engraved cloud motifs and knotwork wrap the bell body; the pin "
        "silhouette follows the bell-and-wave composition.",
        "Aged bronze and dark umber enamel for the bell, pale jade echo rings, warm gold strike spark, deep indigo shadow wells.",
    ),
    (
        "crown_of_patterns",
        "Crown of Patterns",
        "An ornate imperial crown formed from interlocking mahjong motifs: "
        "miniature triplets, sequence runs, and pair crests woven into one "
        "tiered diadem. At the center sits a radiant circular medallion with "
        "concentric geometry, while side filigree branches into repeating "
        "tile-like tessellations. Rich, symmetrical silhouette with layered "
        "depth and strong readability at pin scale.",
        "Royal crimson and deep indigo enamel wells, ivory tile motifs, luminous amber-gold highlights, cool jade accent inlays.",
    ),
]


def _relic_rs_asset_slugs() -> set[str]:
    text = RELIC_RS_PATH.read_text()
    return {
        m.group(1)
        for m in re.finditer(r'RelicId::\w+\s*=>\s*"([a-z0-9_]+)\.png"', text)
    }


def _validate_relics_list() -> None:
    expected = _relic_rs_asset_slugs()
    listed = {slug for slug, *_ in RELICS}
    missing = sorted(expected - listed)
    extra = sorted(listed - expected)
    if missing or extra:
        msg = ["RELICS list drift vs crates/mahjuro-core/src/core/relic.rs asset_filename():"]
        if missing:
            msg.append(f"  missing: {', '.join(missing)}")
        if extra:
            msg.append(f"  extra: {', '.join(extra)}")
        raise SystemExit("\n".join(msg))


_validate_relics_list()


def build_object_prompt(
    name: str,
    visual: str,
    palette: str,
    rarity: str,
    *,
    from_reference: bool = False,
    extra: str = "",
) -> str:
    """Prompt for the transparent color render (`*_object.png` — albedo fallback for the loader).

    When `from_reference=True`, assumes the call is an image edit against a
    grayscale relief guide and appends instructions to honor that guide's
    silhouette and divider structure. Text-prompted runs omit those lines so
    the model isn't told to match a reference that doesn't exist.
    """
    base = (
        f"{style_prefix(rarity)}\n\n"
        f"Asset type: soft-enamel lapel pin relic color render, product-shot framing.\n"
        f"Relic name: '{name}'.\n"
        f"Subject: isolated soft-enamel lapel pin depicting: {visual}\n"
        f"Enamel palette (colors apply to the recessed soft-enamel wells only; the raised metal lines, die-cut outer rim, and negative-space substrate follow the metal tier above): {palette}\n"
        "Composition/framing: pin centered on a square canvas with a small uniform margin; the pin's outer silhouette may be any natural shape (not forced square). Pin plane parallel to the camera.\n"
        "Lighting/mood: neutral studio key plus a soft warm rim, gentle radial burnish on the substrate.\n"
        "Materials: matte-to-satin soft enamel fills sitting below raised metal divider lines in the metal tier above; a continuous raised outer rim traces the pin's individual die-cut outline in that same metal. Not hard enamel — no flush polish, no glassy vitreous surface.\n"
        "Background: a perfectly flat, uniform, pure black archival backdrop — the solid matte black of a museum archival photography plate. Every region inside the outer silhouette resolves as solid opaque material — either enamel fill or raised metal line."
    )
    if extra:
        base += f"\nAdditional subject direction: {extra}"
    if from_reference:
        base += (
            "\nRelief guide usage: the accompanying grayscale relief guide defines SHAPE, SILHOUETTE, and internal divider layout. Match its outer silhouette, centered placement, divider structure, major shapes, and orientation exactly; add color and material on top. Any gray region INSIDE the silhouette resolves as a solid opaque enamel fill. Areas outside the outer silhouette resolve as pure flat black (#000000).\n"
            "Keep the proportions, parts, and framing of the relief guide intact."
        )
    return base


def build_height_prompt(name: str, visual: str, addendum: str = "") -> str:
    """Prompt for `*_height.png` — matches input silhouette; bound as linear GPU relief."""
    return (
        f"Grayscale relief guide for the soft-enamel lapel pin relic '{name}'.\n"
        f"Subject: {visual}\n"
        "Composition/framing: pin centered on a square canvas with a small uniform margin; outer silhouette may be any natural shape (not forced square). Pin plane parallel to the camera.\n"
        "Output: pure black background, front-facing near-orthographic soft-enamel pin silhouette with clean internal partitions.\n"
        "Tonal key (each region is a single flat tone with a hard edge to its neighbor):\n"
        "  - White: highest raised metal — die-cut outer rim tracing the pin's custom silhouette and raised metal divider lines.\n"
        "  - Mid-grays: recessed soft-enamel fill surface inside the silhouette (below the metal lines).\n"
        "  - Black: the area outside the outer silhouette.\n"
        "Every area inside the outer silhouette resolves to gray or white, so the later color pass treats it as a solid opaque enamel fill.\n"
        "A clean monochrome grayscale relief, matching the input in proportion."
    )
    if addendum:
        base += f"\n\nSubject-specific relief:\n{addendum}"
    return base


def build_specular_prompt(name: str, visual: str) -> str:
    """Prompt for `*_specular.png` — specular mask aligned to the color render."""
    return (
        f"Grayscale specular reflectivity map for the soft-enamel lapel pin relic '{name}'.\n"
        f"Subject: {visual}\n"
        "Composition/framing: pin centered on a square canvas with a small uniform margin; outer silhouette may be any natural shape (not forced square). Pin plane parallel to the camera.\n"
        "Output: pure black background, front-facing near-orthographic pin silhouette with clean internal partitions.\n"
        "Tonal key (each region is a single flat tone with a hard edge to its neighbor):\n"
        "  - White: highest specularity — die-cut outer rim and raised metal divider lines (polished metal catching sharp highlights).\n"
        "  - Dark grays: low specularity — matte-to-satin soft-enamel wells (rougher, diffuse fills).\n"
        "  - Black: the area outside the outer silhouette.\n"
        "This is a reflectivity map, not a height map or color render. Match the input silhouette and internal partitions exactly.\n"
        "A clean monochrome grayscale specular mask, matching the input in proportion."
    )


# Soft-enamel specular defaults (0–255). Raised metal ≈ height white; fills ≈ matte.
SPECULAR_ENAMEL = 36
SPECULAR_METAL = 255
SPECULAR_METAL_THRESHOLD = 200
SPECULAR_RAMP_START = 168


def specular_from_height_luma(luma: int) -> int:
    """Map relief height luma → specular mask luma for soft-enamel pins."""
    if luma < HEIGHT_ALPHA_LO:
        return 0
    if luma >= SPECULAR_METAL_THRESHOLD:
        return SPECULAR_METAL
    if luma >= SPECULAR_RAMP_START:
        t = (luma - SPECULAR_RAMP_START) / max(
            1, SPECULAR_METAL_THRESHOLD - SPECULAR_RAMP_START
        )
        return int(SPECULAR_ENAMEL + t * (SPECULAR_METAL - SPECULAR_ENAMEL))
    if luma >= HEIGHT_ALPHA_HI:
        t = (luma - HEIGHT_ALPHA_HI) / max(1, SPECULAR_RAMP_START - HEIGHT_ALPHA_HI)
        return int(SPECULAR_ENAMEL * (0.65 + 0.35 * t))
    return SPECULAR_ENAMEL


def write_specular_from_height(height_path: Path, specular_path: Path) -> bool:
    """Derive a specular mask PNG from the height relief guide.

    Returns False if the height map does not exist.
    """
    from PIL import Image

    if not height_path.exists():
        return False
    with Image.open(height_path) as im:
        height = im.convert("L")
    spec = height.point(specular_from_height_luma, mode="L")
    spec.save(specular_path, format="PNG")
    return True


def flatten_specular_to_black_bg(path: Path) -> None:
    """Force outside-silhouette pixels to true black on a specular mask."""
    flatten_height_to_black_bg(path)


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
    where salient-object matting reads the outer rim as background and shreds it).
    The height-derived path sidesteps that failure entirely.
    """
    if alpha_from_height(object_path, height_path):
        print(f"  Alpha from height map: {object_path.name}")
        return
    remove_background(object_path)
    print(f"  Cleaned bg (rembg fallback): {object_path.name}")


def alpha_from_height(object_path: Path, height_path: Path) -> bool:
    """Use the height map's silhouette as the alpha channel for the object render.

    Image-gen models tend to bake in some background tone, and u2net's
    salient-object matte fails on dark-on-dark subjects (e.g. iron-tier
    pins rendered against
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
    client,
    prompt: str,
    output_path: Path,
    model: str,
    size: str,
) -> None:
    """Call Gemini Nano Banana 2 and save the resulting PNG."""
    aspect_ratio, image_size = parse_size(size)
    img_bytes = generate_image_bytes(
        client,
        prompt,
        model=model,
        aspect_ratio=aspect_ratio,
        image_size=image_size,
    )
    output_path.write_bytes(img_bytes)
    print(f"  Saved: {output_path}")


# OpenAI's DALL-E 2 edits endpoint used alpha to mark editable regions. Gemini
# has no first-class mask parameter; we instead pass the silhouette mask as a
# second reference image and describe the convention in the prompt.
# `MASK_ALPHA_THRESHOLD` (defined elsewhere in this file) still drives where the
# binary cut between silhouette and background lands when we build the mask.


def build_edit_mask(reference_path: Path, mask_out: Path) -> bool:
    """Build a binary L-mode silhouette mask for `reference_path` (white = subject).

    Returns False if the reference is fully opaque (nothing to derive a
    silhouette from), since a full-white mask would convey no information.
    """
    from PIL import Image

    if not reference_path.exists():
        return False
    with Image.open(reference_path) as im:
        ref = im.convert("RGBA")
    alpha = ref.split()[-1]
    lo, hi = alpha.getextrema()
    if lo == 255 and hi == 255:
        return False

    # Silhouette → white (subject region), surround → black (background).
    mask_alpha = alpha.point(
        lambda a: 255 if a >= MASK_ALPHA_THRESHOLD else 0, mode="L"
    )
    mask_alpha.save(mask_out, format="PNG")
    return True


_MASK_INSTRUCTION = (
    "\n\nReference inputs: image #1 is the structural reference to edit; "
    "image #2 is a binary silhouette mask where WHITE marks the subject "
    "region and BLACK marks the background. Keep the output silhouette "
    "aligned to the white region of the mask."
)


def generate_from_reference(
    client,
    prompt: str,
    output_path: Path,
    model: str,
    reference_path: Path,
    try_lock_silhouette: bool = False,
) -> None:
    """Edit an existing source image into the requested artifact via Gemini.

    When `try_lock_silhouette=True` and the reference has a usable alpha, a
    binary mask is built and passed as a second reference image, with prompt
    instructions describing the convention (white = subject region).
    """
    refs: list[Path] = [reference_path]
    full_prompt = prompt
    mask_path: Path | None = None
    if try_lock_silhouette:
        fd, mask_str = tempfile.mkstemp(prefix="editmask_", suffix=".png")
        os.close(fd)
        candidate = Path(mask_str)
        if build_edit_mask(reference_path, candidate):
            mask_path = candidate
            refs.append(candidate)
            full_prompt = prompt + _MASK_INSTRUCTION
        else:
            candidate.unlink(missing_ok=True)
            print(
                f"  (silhouette lock requested but {reference_path.name} has "
                "no usable alpha; falling back to unmasked edit)"
            )

    try:
        img_bytes = generate_image_bytes(
            client,
            full_prompt,
            model=model,
            aspect_ratio="1:1",
            image_size="1K",
            refs=refs,
        )
        output_path.write_bytes(img_bytes)
        print(f"  Saved: {output_path}")
    finally:
        if mask_path is not None:
            mask_path.unlink(missing_ok=True)


def artifact_targets(
    base_dir: Path, slug: str, artifact: str, *, specular_mode: str
) -> list[tuple[str, Path]]:
    if artifact == "object":
        return [("object", base_dir / f"{slug}_object.png")]
    if artifact == "height":
        return [("height", base_dir / f"{slug}_height.png")]
    if artifact == "mask":
        return [("mask", base_dir / f"{slug}_mask.png")]
    if artifact == "specular":
        return [("specular", base_dir / f"{slug}_specular.png")]
    # Object-first ordering: the object render is the authoritative pass
    # (text-prompted, most context), and the height pass edits it into a
    # relief guide with the object's silhouette as a hard constraint.
    targets = [
        ("object", base_dir / f"{slug}_object.png"),
        ("height", base_dir / f"{slug}_height.png"),
    ]
    if specular_mode == "reference":
        targets.append(("specular", base_dir / f"{slug}_specular.png"))
    return targets


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate Mahjuro relic 3D source art via Google Nano Banana 2"
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
        choices=("object", "height", "specular", "mask", "both"),
        default="both",
        help=(
            "Which asset artifact to generate per relic (default: both → "
            "object+height+specular+mask). 'mask' / 'specular' only re-derive "
            "from existing heights."
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
        default=DEFAULT_MODEL,
        help=f"Gemini image model (default: {DEFAULT_MODEL}).",
    )
    parser.add_argument(
        "--size",
        type=str,
        default="1:1@1K",
        help=(
            "Generation size — Gemini ASPECT@TIER (default: 1:1@1K). "
            "Legacy WxH like '1024x1024' is auto-translated to the closest "
            "Gemini aspect/size."
        ),
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
        "--specular-mode",
        choices=("derive", "reference"),
        default="derive",
        help=(
            "How to make *_specular.png assets: derive from the height map "
            "(default — matte enamel vs shiny metal) or edit the object render "
            "into a specular mask via the API."
        ),
    )
    args = parser.parse_args()

    if args.list:
        for i, (slug, name, _, _) in enumerate(RELICS, 1):
            rarity = SLUG_TO_RARITY.get(slug, "?")
            print(
                f"  {i:2d}. {name:<22s}  [{rarity:<9s}]  "
                f"{slug}_object.png, {slug}_height.png, "
                f"{slug}_specular.png, {slug}_mask.png"
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

    if args.artifact == "specular":
        wrote = 0
        missing = 0
        for idx, (slug, name, _, _) in targets:
            height_path = out_dir / f"{slug}_height.png"
            specular_path = out_dir / f"{slug}_specular.png"
            if specular_path.exists() and not args.force:
                print(
                    f"[{idx + 1}] {name}: specular exists — use --force to regenerate"
                )
                continue
            if args.specular_mode == "reference":
                print(
                    f"[{idx + 1}] {name}: --specular-mode reference requires the "
                    "main generation loop; run without --artifact specular."
                )
                missing += 1
                continue
            if write_specular_from_height(height_path, specular_path):
                print(f"[{idx + 1}] {name}: wrote {specular_path.name}")
                wrote += 1
            else:
                print(f"[{idx + 1}] {name}: no height map — skipping")
                missing += 1
        print(f"\nDone. wrote={wrote} missing={missing} → {out_dir}")
        return

    client = None
    if not args.dry_run:
        client = init_client()

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

        for artifact_name, output_path in artifact_targets(
            out_dir, slug, args.artifact, specular_mode=args.specular_mode
        ):
            object_ref_prompt = build_object_prompt(
                name,
                visual,
                palette,
                rarity,
                from_reference=(args.object_mode == "reference"),
            )
            if artifact_name == "object":
                prompt = object_ref_prompt
            elif artifact_name == "height":
                prompt = build_height_prompt(name, visual)
            elif artifact_name == "specular":
                prompt = build_specular_prompt(name, visual)
            else:
                prompt = object_ref_prompt

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
                    )
                    flatten_height_to_black_bg(output_path)
                    print(f"  Black bg: {output_path.name}")
                elif artifact_name == "specular" and args.specular_mode == "reference":
                    if not object_output_path.exists():
                        print(
                            "  Specular needs an object reference first; generating object pass."
                        )
                        generate_image(
                            client,
                            build_object_prompt(name, visual, palette, rarity),
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
                    )
                    flatten_specular_to_black_bg(output_path)
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
                    )
                else:
                    generate_image(
                        client, prompt, output_path, args.model, args.size
                    )
                    if artifact_name == "height":
                        flatten_height_to_black_bg(output_path)
                        print(f"  Black bg: {output_path.name}")
                    elif artifact_name == "specular":
                        flatten_specular_to_black_bg(output_path)
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

        if (
            height_ready
            and args.specular_mode == "derive"
            and args.artifact in ("both", "height", "specular")
        ):
            specular_path = out_dir / f"{slug}_specular.png"
            if args.force or not specular_path.exists():
                if write_specular_from_height(height_output_path, specular_path):
                    print(f"  Wrote specular: {specular_path.name}")

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
