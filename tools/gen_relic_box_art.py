#!/usr/bin/env python3
"""Generate relic icon PNGs via OpenAI image generation.

Each relic gets a full-color texture styled as vintage hobby model box art
from the fictional "Korvashi Model Co." — faded, slightly uncanny,
with nonsense-language text and dramatic oil-painting illustrations.

Run:
    OPENAI_API_KEY=sk-... python3 tools/gen_relic_box_art.py

    # Or generate a single relic:
    OPENAI_API_KEY=sk-... python3 tools/gen_relic_box_art.py triplet_boost

    # Preview at higher res without downscale:
    OPENAI_API_KEY=sk-... python3 tools/gen_relic_box_art.py --preview dragon_echo

Outputs RGBA PNGs into assets/textures/relics/ matching the filenames
from RelicId::asset_filename():
    triplet_boost.png
    sequence_surge.png
    ...

Requires:
    pip install openai Pillow
"""

import argparse
import base64
import io
import os
import sys

from openai import OpenAI
from PIL import Image, ImageFilter

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

SIZE = 256  # final output size (matches talisman pipeline)
PREVIEW_SIZE = 512  # --preview keeps this resolution
OUT_DIR = os.path.join(os.path.dirname(__file__), "..", "assets", "textures", "relics")

# ---------------------------------------------------------------------------
# Style system prompt — shared across all relics
# ---------------------------------------------------------------------------

STYLE_CORE = (
    "A square illustration in the style of a vintage 1970s–1980s hobby model "
    "kit box lid. The painting is dramatic, oil-painting-style with visible "
    "brushwork, depicting the subject in action. The image should look like "
    "it has been sitting in a dusty shop window for decades: slightly faded, "
    "yellowed at the edges, with subtle cardboard-print texture. "
    "\n\n"
    "IMPORTANT STYLE RULES:\n"
    "- The overall mood is creepy and old, but NOT gothic or horror. Think "
    "'uncanny hobbyist' — proportions slightly off, shadows falling wrong, "
    "skies an unusual hue. Eerie, not scary.\n"
    "- Any visible text/lettering MUST be in a made-up nonsense language "
    "(NOT real words in any real language). Include a fictional brand name "
    "in the top-left corner and a scale marking like '1/72' or '1:48' "
    "somewhere on the box face.\n"
    "- The box has a thin border and the brand logo area at top. The main "
    "painting fills most of the frame.\n"
    "- Do NOT include real brand names, real language, or recognizable IP.\n"
    "- Gunpla/mecha inspiration is welcome but not required.\n"
    "- The image must work as a small game icon — keep the composition "
    "readable at 256x256 with a clear focal point.\n"
)

# Each rarity tier has its own fictional manufacturer with a distinct
# cultural flavor, color palette, and box-art personality.
RARITY_STYLE: dict[str, str] = {
    "common": (
        "MANUFACTURER: 'KORVASHI' — a cheap, cheerful hobby brand. The "
        "nonsense language on the box looks vaguely Slavic-Japanese mashup "
        "(e.g. 'Korvashi Moderu', 'Postroika Seriya 4', 'Naboru-Kit'). "
        "Simple box design, friendly but slightly off. "
        "Color palette: warm tan, olive green, faded sky blue, cream. "
        "Feels like a beginner kit from a corner hobby shop."
    ),
    "uncommon": (
        "MANUFACTURER: 'DRAVUNA-KAI' — a mid-tier mecha and vehicle brand. "
        "The nonsense language looks vaguely Japanese-Thai mashup "
        "(e.g. 'Dravuna-Kai Senshaban', 'Kharudo Taipu-III', 'Mongara Saku'). "
        "Bolder colors, more dynamic compositions, box has a glossy feel. "
        "Color palette: deep teal, burnt orange, steel gray, jade. "
        "Feels like the brand an enthusiast upgrades to."
    ),
    "rare": (
        "MANUFACTURER: 'ZELKUBO WERKE' — a premium, austere brand. "
        "The nonsense language looks vaguely Germanic-technical "
        "(e.g. 'Zelkubo Werke Ausf. VII', 'Kampfgerät Drachen-Serie', "
        "'Präzisions-Bausatz'). Dramatic oil paintings, dark moody lighting. "
        "Color palette: deep navy, ochre, rust, muted gold accents. "
        "Feels like a serious collector's kit — military precision."
    ),
    "legendary": (
        "MANUFACTURER: 'MEKHARI ATELIER' — a luxury collector's brand. "
        "The nonsense language looks vaguely Arabic-French mashup "
        "(e.g. 'Mekhari Atelier Édh. Prestige', 'Qalbar al-Sariyya', "
        "'Trésillon Magnifar'). Ornate gold-foil border elements, the box "
        "art is the most dramatic and detailed. Rich, deep colors. "
        "Color palette: royal purple, deep gold, burgundy, midnight blue. "
        "Feels like an expensive limited-edition display piece."
    ),
}


def build_style_prefix(rarity: str) -> str:
    """Combine the core style prompt with the rarity-specific manufacturer flavor."""
    return STYLE_CORE + "\n" + RARITY_STYLE.get(rarity, RARITY_STYLE["common"]) + "\n"

# ---------------------------------------------------------------------------
# Per-relic prompts — the dramatic box-art painting description
# ---------------------------------------------------------------------------

RELIC_PROMPTS: dict[str, str] = {
    # ═══════════════════════════════════════════════════════════════════
    # COMMON — Korvashi (Slavic-Japanese mashup, friendly, cheap kits)
    # ═══════════════════════════════════════════════════════════════════
    "triplet_boost": (
        "A squat siege tower with three identical spires, painted mid-assault "
        "on a crumbling fortress wall. Tiny soldiers on the ramparts look up "
        "in exaggerated dread. The three spires glow faintly from within. "
        "Stormy sky with too-green clouds. Cheerful box layout, beginner-kit "
        "energy despite the ominous subject."
    ),
    "green_luck": (
        "A jade-green vintage automobile (beetle-shaped but with too many "
        "headlights and insect-like proportions) parked in an overgrown lot "
        "full of four-leaf clovers. Dusk lighting. No driver visible but the "
        "interior light is on. Fireflies around the car. Friendly, nostalgic."
    ),
    "wall_peek": (
        "A submarine periscope rising from murky, still water. The periscope "
        "lens is an enormous unblinking human eye. Reflected in the water's "
        "surface: a long wall of tiles stretching to the horizon. Eerie calm, "
        "low fog. Simple kit — just the periscope and a sea base."
    ),
    "zodiac_pouch": (
        "A weathered leather satchel sitting on a wooden table, slightly open, "
        "with constellation charts spilling out. The constellations on the "
        "papers seem to glow and shift. A hand reaches for the satchel from "
        "just off-frame. Candlelit tavern interior. Cozy but uncanny."
    ),

    # ═══════════════════════════════════════════════════════════════════
    # UNCOMMON — Dravuna-Kai (Japanese-Thai mashup, bold mecha/vehicles)
    # ═══════════════════════════════════════════════════════════════════
    "sequence_surge": (
        "A hydrofoil boat cutting through three sequential ocean waves at "
        "sunset, each wave taller than the last. The water is an unnatural "
        "turquoise. Birds in the sky form a sequential chevron pattern. "
        "Dynamic composition with bold teal and orange."
    ),
    "pair_power": (
        "Two identical construction mecha facing each other over an industrial "
        "dockyard, jointly lifting a single massive stone tile between them. "
        "Dramatic storm clouds and low golden light. The mecha are unnervingly "
        "symmetrical, perfect mirror images. Teal and steel palette."
    ),
    "quick_draw": (
        "A mechanical brass hand emerging from an ornate lacquered box, fingers "
        "poised to snatch a tile from a table. Lightning arcs between the "
        "fingertips. Neon-lit workshop background with specimen jars. "
        "The hand has one too many joints. Retrofuturist."
    ),
    "set_magnet": (
        "A retro-futuristic magnetic-levitation train pulling identical cargo "
        "containers toward itself. The containers slide along invisible rails, "
        "drawn by magnetism. Flat desert landscape under an ominous amber sky. "
        "Bold graphic composition, speed lines."
    ),
    "shanten_shove": (
        "A giant open palm made of jade and gold shoving a single mahjong tile "
        "across a polished stone table toward a waiting hand of tiles. Motion "
        "blur on the sliding tile. Dramatic side-lighting, dust motes in the air."
    ),
    "round_compass": (
        "An ornate compass the size of a dining table, sitting in a ship's "
        "bridge. The four cardinal directions are labeled in nonsense kanji-"
        "like characters. The needle spins wildly. Through the viewport: "
        "a violent storm with green lightning. Deep teal atmosphere."
    ),
    "yaku_scholar": (
        "A dollhouse-style cutaway diorama of a scholar's study, crammed with "
        "scrolls and leather-bound books. The scholar (seen from behind) has "
        "four arms, each holding a different scroll. A cat sits on the desk "
        "watching with too-knowing eyes. Warm amber and jade tones."
    ),

    # ═══════════════════════════════════════════════════════════════════
    # RARE — Zelkubo Werke (Germanic-technical, dark, austere, military)
    # ═══════════════════════════════════════════════════════════════════
    "honor_fury": (
        "A heavy tank covered in painted calligraphic symbols instead of "
        "camouflage, firing its main gun on a misty battlefield. The muzzle "
        "blast blooms into flowing brush-stroke characters. Low dramatic "
        "angle. Mud and smoke. Dark navy and ochre palette. Austere."
    ),
    "red_dragon_rage": (
        "A mecha in the shape of a coiling red dragon, mid-roar, standing on "
        "a rocky cliff over a burning city. Panel lines, verniers, beam sword "
        "in hand. The flames below are arranged in unnaturally orderly rows. "
        "Very Gunpla box-art. Deep reds over navy background."
    ),
    "white_silence": (
        "A pure white biplane flying through an impossibly quiet snowscape. "
        "No engine exhaust, no contrail, no sound implied. Below: a frozen "
        "lake with a single white tile shape visible beneath the ice. Muted "
        "blue-white palette, dead calm. Technical precision."
    ),
    "joker_tile": (
        "A mechanical puzzle box mid-transformation, each visible face showing "
        "a different tile symbol. Hands with slightly too many fingers "
        "manipulate the box. Workshop table cluttered with watchmaker's tools. "
        "A single spotlight from above. Dark and precise."
    ),
    "overflow": (
        "A grain silo bursting at the seams, tiles pouring out of cracks like "
        "grain in an avalanche. A lone figure stands below, dwarfed, looking "
        "up. Flat prairie under a deep golden sky that's almost too vivid. "
        "Industrial, foreboding."
    ),
    "chain_reaction": (
        "A Rube Goldberg machine made of dominoes, gears, and game tiles, "
        "mid-cascade. Each successive stage glows brighter than the last. "
        "Dark laboratory background with chalkboard equations barely visible. "
        "Technical diagram energy. Rust and muted gold."
    ),
    "multiplier_master": (
        "A radio transmission tower at night, concentric signal rings "
        "emanating outward into a star-filled desert sky. Each ring is "
        "labeled with multiplier numbers in nonsense script. A lone figure "
        "adjusts dials at the base. Navy and ochre."
    ),
    "wild_winds": (
        "A junk-rigged sailboat caught in four simultaneous winds, visible "
        "as four different-colored wind streams pulling from each cardinal "
        "direction. The sails billow impossibly in all directions at once. "
        "Churning sea, dramatic low lighting. Austere and moody."
    ),
    "kan_drum": (
        "A taiko drum the size of a building, struck by a colossal mechanical "
        "arm. The shockwave is visible, radiating outward and shattering "
        "tiles arranged in groups of four at its base. Festival ground at "
        "twilight with paper lanterns. Deep rust and navy."
    ),
    "dora_crown": (
        "A jeweler's workbench under a single dramatic lamp. A crown is being "
        "assembled — its central gemstone is a glowing tile with an indicator "
        "mark. Tiny tools, magnifying loupes, scattered metal filings. The "
        "crown casts sharp shadows. Dark, precise, muted gold highlights."
    ),
    "tenpai_talisman": (
        "A Cold War-era military radar installation. The circular screen "
        "shows a single blip dead-center, tile-shaped. Operators lean in with "
        "tense expressions, bathed in green CRT glow. Concrete bunker walls. "
        "Military-industrial austerity."
    ),
    "lunar_almanac": (
        "A massive orrery mounted on a cathedral wall, zodiac symbols orbiting "
        "a central moon face. Hooded monks observe from below. The moon has a "
        "face that is awake but expressionless. Candle-smoke and dust motes. "
        "Deep navy and aged gold."
    ),

    # ═══════════════════════════════════════════════════════════════════
    # LEGENDARY — Mekhari Atelier (Arabic-French, luxury, ornate, gold)
    # ═══════════════════════════════════════════════════════════════════
    "dragon_echo": (
        "Three dragon-shaped mecha (one red, one green, one white) standing "
        "in triangular formation, their roars visualized as overlapping sound "
        "waves that converge into a single beam of light. Ruined cityscape "
        "backdrop. Prestige collector's box — rich purples, gold filigree "
        "border elements, very dramatic."
    ),
    "eight_treasures": (
        "A treasure chest exploding open in an underwater temple, eight "
        "distinct glowing objects mid-flight: compass, scroll, bell, mirror, "
        "fan, sword, cup, pearl. Shafts of light pierce the water from above. "
        "Coral and ancient columns. Luxurious deep blues and gold."
    ),
    "kongs_blessing": (
        "A massive temple gate flanked by four identical stone guardian "
        "statues, each holding a glowing orb. Pilgrims approach in silhouette "
        "along a misty path. Dawn light filters through incense smoke. The "
        "statues' expressions are serene but watchful. Royal purple and gold."
    ),

    # ═══════════════════════════════════════════════════════════════════
    # FLOWER-SYNERGY
    # ═══════════════════════════════════════════════════════════════════
    "garden_keeper": (
        "A greenhouse made of stained glass panels, each panel depicting a "
        "different flower species in luminous detail. Inside, a mechanical "
        "gardener with brass arms tends two identical rows of blossoms — "
        "every flower is perfectly duplicated left-to-right, mirror image. "
        "Warm afternoon light refracts through the glass. Teal and sage."
    ),
    "ikebana": (
        "A stark alcove containing a single ikebana arrangement in a dark "
        "ceramic vase. Two blooming branches arch toward each other, their "
        "petals radiating concentric rings of light. A calligraphy scroll "
        "hangs behind the arrangement, its characters glowing faintly. The "
        "shadows are impossibly sharp. Deep navy and ochre. Austere and still."
    ),
    "hanami": (
        "A picnic blanket beneath a cherry tree in full bloom, petals falling "
        "like snow. On the blanket: neat stacks of gold coins among lacquered "
        "bento boxes and ceramic cups. No people — just the feast, the gold, "
        "and an impossible density of blossoms. Dappled pink light, nostalgic "
        "and slightly dreamy. Cheerful beginner-kit energy."
    ),

    # ═══════════════════════════════════════════════════════════════════
    # 15 NEW RELICS
    # ═══════════════════════════════════════════════════════════════════
    "jade_serpent": (
        "A jade-green snake coiled around a bundle of bamboo stalks, its "
        "scales formed from tiny mahjong tile faces. A simple terrarium "
        "display with moss and pebbles. Soft diffused daylight. Cheerful, "
        "slightly off — the snake has too-large friendly eyes. Cream and "
        "olive green palette."
    ),
    "ink_brush": (
        "A calligraphy brush standing upright in an ink stone, mid-stroke — "
        "a splatter of ink radiates outward, forming tiny character-suit tile "
        "shapes. Wooden desk, rice paper, and a window with gray daylight. "
        "Simple, nostalgic. Warm tan and black ink tones."
    ),
    "pearl_diver": (
        "A diver in a vintage brass helmet descending into turquoise water, "
        "reaching for a cluster of luminous pearls arranged in the pattern of "
        "dots-suit tile pips. Bubbles trail upward. Coral and sand below. "
        "Simple, cheerful composition with faded blues and cream."
    ),
    "low_tide": (
        "A beach at low tide revealing a mosaic of numbered tiles (1, 2, 3) "
        "embedded in the wet sand. Tide pools reflect a pale sky. A child's "
        "bucket and shovel sit nearby, slightly too large. Peaceful, warm, "
        "beginner-kit energy. Tan, soft blue, cream."
    ),
    "merchants_eye": (
        "A jeweler's monocle on a velvet cushion, its lens showing a tiny "
        "reflected price tag with the number crossed out and replaced by a "
        "smaller one. A cluttered shop counter with coins and trinkets. Warm "
        "lamplight, cozy but slightly uncanny. Cream, olive, faded gold."
    ),
    "edge_runner": (
        "A futuristic motorcycle tearing along the razor edge of a cliff road, "
        "one wheel off the edge. The cliff face is tiled with enormous 1s and "
        "9s. Sunset behind, deep ravine below. Dynamic angle, bold teal and "
        "burnt orange. Speed lines and dust clouds."
    ),
    "lucky_seven": (
        "A slot machine built into the side of a mecha, its three reels all "
        "showing sevens. Coins pour from the machine's mouth into a growing "
        "pile. Neon-lit arcade background, hazy with machine smoke. Bold "
        "jade and orange palette. Lucky-charm energy."
    ),
    "momentum": (
        "A series of pendulums in a Newton's cradle, each ball replaced by "
        "a mahjong tile, captured mid-swing with the last tile flying off. "
        "Motion blur streaks. Dark workshop table, dramatic side-lighting. "
        "The cradle frame is brass with rivets. Teal and steel palette."
    ),
    "minimalist": (
        "A zen garden with a single rock and perfectly raked sand, viewed from "
        "above. The rock is a pair of identical small tiles leaning against "
        "each other. Extreme negative space. Late afternoon shadow stretches "
        "across the frame. Deep teal and sand palette. Very still."
    ),
    "turtle_shell": (
        "A tortoise with a shell made of hexagonal tile faces, trudging across "
        "a cracked desert floor. A distant sandstorm approaches. The shell "
        "glows faintly with a protective shimmer. Low angle, dramatic sky. "
        "Burnt orange and teal atmosphere."
    ),
    "closed_gate": (
        "A massive iron gate slamming shut, viewed from inside. The gap "
        "narrows to a sliver of light. Terminal tiles (1s and 9s) and honor "
        "characters are forged into the gate's surface. Sparks fly from the "
        "hinges. Dark interior, blinding light from outside. Navy and rust."
    ),
    "gold_furnace": (
        "A squat industrial furnace with its door ajar, molten gold pouring "
        "into ingot molds. The furnace has dial gauges showing mult readings "
        "in nonsense script. A worker in a leather apron watches from shadow. "
        "Intense orange glow against dark navy walls. Austere, industrial."
    ),
    "snowball": (
        "A snowball rolling downhill, growing enormous, picking up tiles and "
        "coins and debris. A village at the bottom of the hill looks on in "
        "alarm. Moonlit winter night, deep blue-white palette. The snowball "
        "casts sharp geometric shadows. Military precision in the chaos."
    ),
    "second_wind": (
        "A runner collapsed at a finish line, one hand reaching forward, a "
        "second ghostly version of the same runner rising from the body and "
        "sprinting onward. Stadium lights, empty bleachers, dusk sky. "
        "Navy and ochre with ethereal blue translucency on the ghost figure."
    ),
    "glass_cannon": (
        "A cannon made entirely of cut crystal and glass, mid-firing. The "
        "muzzle blast shatters hairline cracks through the barrel. Prismatic "
        "light refracts through the crystal body. Velvet display room with "
        "ornate gold-foil frame elements. Dramatic, luxurious, fragile. "
        "Royal purple, deep gold, midnight blue."
    ),

    # ═══════════════════════════════════════════════════════════════════
    # BALATRO-INSPIRED (Patch F)
    # ═══════════════════════════════════════════════════════════════════
    "last_breath": (
        "A deep-sea diver's helmet with the last air bubble escaping upward, "
        "the bubble containing a glowing tile. Murky abyss below, faint "
        "bioluminescent creatures in the distance. The diver's hand reaches "
        "for something off-frame. Navy, ochre, deep rust. Suffocating calm."
    ),
    "tile_polisher": (
        "A watchmaker's bench covered in tiles being hand-polished, each one "
        "progressively shinier from left to right. The rightmost tile gleams "
        "like a mirror. Magnifying glasses, fine cloths, wood shavings. "
        "Single overhead lamp. Precise, meditative. Navy and muted gold."
    ),
    "paper_lantern": (
        "A single red paper lantern hanging from a wire in an empty alley, "
        "its candle burning too brightly. The paper is visibly thin, on the "
        "edge of catching fire. Smoke wisps curl upward. Wet cobblestones "
        "reflect the glow. Night scene. Bold teal and burnt orange."
    ),
    "iron_lantern": (
        "A heavy cast-iron lantern bolted to a fortress wall, its flame "
        "burning cold blue through thick glass. Frost rimes the bolts. The "
        "wall stretches into fog in both directions. Military checkpoint "
        "energy. Austere. Deep navy, steel gray, muted gold."
    ),
    "mirror_tile": (
        "Two identical ornate hand mirrors facing each other on a velvet "
        "pedestal, creating an infinite regression. In the deepest reflection, "
        "a tile glows. Gold-foil border elements frame the scene. Museum "
        "lighting, purple velvet, midnight blue. Luxurious and unsettling."
    ),
    "way_of_purity": (
        "A monk in white robes walking a single-color tile path across a "
        "misty mountain bridge. Every tile is the same suit. The monk's "
        "reflection in the chasm below shows the path in a different color. "
        "Austere, meditative. Deep navy and ochre. Technical precision."
    ),

    # ═══════════════════════════════════════════════════════════════════
    # PATCH G: 25 BALATRO-INSPIRED RELICS
    # ═══════════════════════════════════════════════════════════════════
    # -- Retrigger --
    "leading_tile": (
        "A marching band of tile-faced figures, the drum major at the front "
        "glowing twice as brightly as the others, mid-stride through a foggy "
        "street parade. Confetti and banners in nonsense script. Dynamic "
        "angle. Bold teal and orange, parade-float energy."
    ),
    "low_echo": (
        "A cave mouth with numbered tiles (1, 2, 3, 4) embedded in the rock. "
        "Sound waves visualized as concentric rings echo outward from each "
        "tile. A bat hangs from the ceiling, too large. Deep teal interior, "
        "amber glow from within the cave. Moist atmosphere."
    ),
    "tea_ceremony": (
        "A traditional tea ceremony setup — iron kettle, bamboo whisk, three "
        "cups arranged on a tatami mat. Steam rises from the kettle forming "
        "tile shapes. The third cup has a crack running through it. Warm "
        "afternoon light through shoji screens. Navy and ochre. Calm before "
        "destruction."
    ),
    "ghost_hand": (
        "A translucent spectral hand hovering above a mahjong table, its "
        "fingers gently touching tiles that glow where contacted. The living "
        "player's hand is visible at the table edge, oblivious. Candlelit "
        "room, deep teal shadows. Eerie but not frightening."
    ),
    # -- Scaling --
    "clean_streak": (
        "A pristine white bowling lane stretching to infinity, every pin "
        "replaced by a numbered tile. The ball mid-roll is impossibly clean "
        "and reflective. No honor symbols anywhere. Fluorescent lighting, "
        "waxed floor gleam. Teal and steel palette. Satisfying, clinical."
    ),
    "obsession": (
        "A detective's evidence board with red string connecting tile "
        "photographs, newspaper clippings in nonsense script, and circled "
        "patterns. One yaku pattern is crossed out with a thick red X. "
        "Dim office, single desk lamp. Navy and rust. Paranoid energy."
    ),
    "bonfire": (
        "A bonfire made of broken relics — shattered mirrors, cracked jade, "
        "twisted metal frames — burning bright on a hillside at night. A "
        "figure warms their hands, silhouetted. Sparks rise into a starless "
        "sky. Deep teal and burnt orange. Bittersweet warmth."
    ),
    "river_runner": (
        "A kayaker shooting rapids, each wave crest formed by a sequence of "
        "three tiles in ascending order. The river cuts through a canyon with "
        "layers of geological strata visible. Dramatic spray and motion blur. "
        "Navy and ochre. Adventure, accumulation."
    ),
    # -- Fragile --
    "melting_ice": (
        "A block of ice on a wooden table, melting under a single heat lamp. "
        "Embedded in the ice: a glowing tile, gradually becoming exposed. "
        "Puddle spreading across the table. Clock on the wall. Simple, warm "
        "lighting. Cheerful but anxious. Cream and faded blue."
    ),
    "silk_thread": (
        "A spool of luminous silk thread on a loom, the thread pulled taut "
        "and fraying at one point. The woven fabric shows tile patterns. "
        "Gentle workshop light through a dusty window. The thread glows "
        "faintly where it's thinnest. Teal and amber. Delicate tension."
    ),
    # -- Copy / Meta --
    "shadow_hand": (
        "A puppet theater stage where a hand puppet casts a shadow on the "
        "back wall — but the shadow is a different, more elaborate puppet "
        "performing the same gesture. Velvet curtains, footlights. The "
        "shadow is sharper than the puppet casting it. Royal purple, deep "
        "gold, burgundy. Luxurious and uncanny."
    ),
    "empty_frame": (
        "A gallery wall with ornate picture frames, each frame empty except "
        "for a faint glow where a painting should be. One frame has a small "
        "mult counter ticking upward in its corner. Polished marble floor "
        "reflects the frames. Teal and steel. Elegant absence."
    ),
    # -- Economy --
    "gold_idol": (
        "A small golden idol sitting on a stone pedestal in a jungle clearing. "
        "Three gold coins materialize from thin air above the idol. Vines "
        "and moss creep up the pedestal. Dappled green light. The idol's "
        "expression is pleased. Cheerful, warm tan and gold."
    ),
    "jade_abacus": (
        "A jade-and-brass abacus on a merchant's desk, beads arranged to "
        "show a calculation. Tiny gold coins balance on the top rail. "
        "Ledger books and an ink pot nearby. Warm lamplight, wooden interior. "
        "Simple, cheerful. Cream, olive green, faded gold."
    ),
    "nest_egg": (
        "A bird's nest in a bare winter tree, containing a single golden egg "
        "that grows larger in each of three sequential panels (like assembly "
        "instructions on a model kit). The egg cracks slightly in the last "
        "panel. Dawn light. Cheerful, cream and soft gold."
    ),
    "patience": (
        "An hourglass on a game table, sand trickling slowly. Around the "
        "hourglass: untouched discard tiles and a growing pile of gold coins. "
        "The sand grains are tiny coins. Warm amber lamplight, wood grain "
        "table. Calm, cozy. Tan, cream, faded gold."
    ),
    # -- Conditional ×mult --
    "way_of_pairs": (
        "Two identical samurai statues facing each other in a stone garden, "
        "each holding a mirror that reflects the other infinitely. Cherry "
        "blossoms fall between them. The symmetry is perfect and unsettling. "
        "Dawn light. Deep navy, ochre, rust. Meditative precision."
    ),
    "way_of_triplets": (
        "Three identical watchtowers on a ridge, each tower a perfect copy "
        "of the others, connected by heavy chains. Lightning strikes all "
        "three simultaneously. Dark storm sky, dramatic chiaroscuro. The "
        "towers cast triple overlapping shadows. Navy, ochre, deep rust."
    ),
    "way_of_sequences": (
        "A train with sequentially numbered carriages (1-2-3, 4-5-6) crossing "
        "a bridge over a vast canyon. The bridge arches form ascending numbers. "
        "Dramatic perspective from below. Sunset light. Navy and ochre. "
        "Industrial precision, satisfying order."
    ),
    # -- Probability / Chaos --
    "fortunes_favor": (
        "A double-sided coin spinning in mid-air, both visible faces showing "
        "heads. The coin hovers above a fortune-teller's crystal ball on a "
        "velvet cloth. Tarot cards scattered around it. The crystal ball "
        "reflects a different room. Navy, ochre, muted gold. Rigged luck."
    ),
    "cracked_tile": (
        "A mahjong tile with a deep crack running diagonally through it, "
        "placed on a felt table. Through the crack, impossible light spills — "
        "sometimes bright, sometimes dark. A hand hesitates above it. "
        "Simple composition. Cream, tan, uncertain warm light. Beginner-kit "
        "gamble energy."
    ),
    "star_tile": (
        "A single tile floating in a planetarium dome, a five-pointed star "
        "etched into its face, rotating slowly. Star projections spin across "
        "the domed ceiling around it. A figure watches from a reclining "
        "chair below. Deep teal and amber glow. Magical, uncertain."
    ),
    # -- Sell-to-activate --
    "smoke_bomb": (
        "A round black bomb with a lit fuse, sitting on a shop counter next "
        "to a 'FOR SALE' sign in nonsense script. Smoke already wisps from "
        "the fuse. The shopkeeper has stepped back, hands raised. Dramatic "
        "shadows. Navy, ochre, rust. Tense, darkly funny."
    ),
    "phantom_relic": (
        "A glass display case containing nothing — but the shadow inside the "
        "case shows the silhouette of an ornate relic that isn't there. A "
        "museum placard reads nonsense text. Three tally marks scratched "
        "into the case glass. Navy and deep rust. Haunted patience."
    ),
    "ritual_blade": (
        "A ceremonial dagger on a stone altar, its blade reflecting not the "
        "room but the relic next to it, which appears to dissolve in the "
        "reflection. Candle smoke, carved symbols on the altar. The blade "
        "has a faint golden edge. Navy, ochre, rust. Sacrificial precision."
    ),
    "disgust": (
        "A weathered cast-iron weather vane on a slate roof, the East and "
        "West cardinal arms bent inward until they nearly touch — fused at "
        "the tips by years of corrosion into a single grimacing mouth. The "
        "North and South arms hang slack and unused. A sour wind visibly "
        "curls from the joined mouth in pale green wisps. Overcast sky, "
        "muted teal and tarnished bronze. Quietly grotesque."
    ),

    # ═══════════════════════════════════════════════════════════════════
    # DISABLED/STUB — generate art anyway for when they ship
    # ═══════════════════════════════════════════════════════════════════
    "riichi_stick": (
        "A single ornate chopstick-like stick standing perfectly upright on a "
        "tatami mat, defying gravity. Around it, tile pieces orbit slowly like "
        "a planetary system. A hand hovers above, about to grab it. "
        "Traditional Japanese room, sliding doors ajar, moonlight. "
        "Dark navy, austere precision."
    ),
    "river_eraser": (
        "A river flowing through a valley, but a giant eraser is dragging "
        "across its surface, leaving blank whiteness where water was. Half the "
        "river is gone. Trees on the erased bank float in empty space. Dreamy, "
        "surreal. Teal and burnt orange."
    ),
    "furiten_ward": (
        "A paper talisman nailed to a heavy wooden door. The calligraphy on "
        "the talisman shifts and writhes. The hallway behind the viewer is "
        "reflected in the door's brass fittings, but the reflection shows a "
        "different hallway. Deep teal and steel gray."
    ),
    "codex_compass": (
        "An open book whose pages fold outward into a three-dimensional "
        "compass rose. The book sits on a lectern in an ancient library. "
        "Each compass direction points to a different bookshelf. Dust hangs "
        "in the air. The compass needle is a quill pen. Jade and amber."
    ),
}

# ---------------------------------------------------------------------------
# Rarity color tint overlays (subtle, applied post-generation)
# ---------------------------------------------------------------------------

# Maps relic slug -> rarity for optional border tinting.
RELIC_RARITY: dict[str, str] = {
    "triplet_boost": "common",
    "green_luck": "common",
    "wall_peek": "common",
    "zodiac_pouch": "common",
    "sequence_surge": "uncommon",
    "pair_power": "uncommon",
    "quick_draw": "uncommon",
    "set_magnet": "uncommon",
    "shanten_shove": "uncommon",
    "round_compass": "uncommon",
    "yaku_scholar": "uncommon",
    "honor_fury": "rare",
    "red_dragon_rage": "rare",
    "white_silence": "rare",
    "joker_tile": "rare",
    "overflow": "rare",
    "chain_reaction": "rare",
    "multiplier_master": "rare",
    "wild_winds": "rare",
    "kan_drum": "rare",
    "dora_crown": "rare",
    "tenpai_talisman": "rare",
    "lunar_almanac": "rare",
    "dragon_echo": "legendary",
    "eight_treasures": "legendary",
    "kongs_blessing": "legendary",
    "riichi_stick": "rare",
    "river_eraser": "uncommon",
    "furiten_ward": "uncommon",
    "codex_compass": "uncommon",
    "garden_keeper": "uncommon",
    "ikebana": "rare",
    "hanami": "common",
    # ── 15 new relics ──
    "jade_serpent": "common",
    "ink_brush": "common",
    "pearl_diver": "common",
    "low_tide": "common",
    "merchants_eye": "common",
    "edge_runner": "uncommon",
    "lucky_seven": "uncommon",
    "momentum": "uncommon",
    "minimalist": "uncommon",
    "turtle_shell": "uncommon",
    "closed_gate": "rare",
    "gold_furnace": "rare",
    "snowball": "rare",
    "second_wind": "rare",
    "glass_cannon": "legendary",
    # ── Balatro-inspired (Patch F) ──
    "last_breath": "rare",
    "tile_polisher": "rare",
    "paper_lantern": "uncommon",
    "iron_lantern": "rare",
    "mirror_tile": "legendary",
    "way_of_purity": "rare",
    # ── Patch G ──
    "leading_tile": "uncommon",
    "low_echo": "uncommon",
    "tea_ceremony": "rare",
    "ghost_hand": "uncommon",
    "clean_streak": "uncommon",
    "obsession": "rare",
    "bonfire": "uncommon",
    "river_runner": "rare",
    "melting_ice": "common",
    "silk_thread": "uncommon",
    "shadow_hand": "legendary",
    "empty_frame": "uncommon",
    "gold_idol": "common",
    "jade_abacus": "common",
    "nest_egg": "common",
    "patience": "common",
    "way_of_pairs": "rare",
    "way_of_triplets": "rare",
    "way_of_sequences": "rare",
    "fortunes_favor": "rare",
    "cracked_tile": "common",
    "star_tile": "uncommon",
    "smoke_bomb": "rare",
    "phantom_relic": "rare",
    "ritual_blade": "rare",
    "disgust": "rare",
}

# Subtle border tint color per rarity (RGBA).
RARITY_BORDER: dict[str, tuple[int, int, int, int]] = {
    "common": (180, 180, 170, 40),     # warm gray
    "uncommon": (100, 160, 130, 50),   # sage green
    "rare": (90, 120, 180, 55),        # dusty blue
    "legendary": (200, 170, 60, 65),   # aged gold
}


def apply_rarity_border(img: Image.Image, rarity: str, width: int = 4) -> Image.Image:
    """Draw a faint colored border to hint at rarity."""
    tint = RARITY_BORDER.get(rarity)
    if tint is None:
        return img
    overlay = Image.new("RGBA", img.size, (0, 0, 0, 0))
    r, g, b, a = tint
    w, h = img.size
    for x in range(w):
        for y in range(h):
            if x < width or x >= w - width or y < width or y >= h - width:
                overlay.putpixel((x, y), (r, g, b, a))
    return Image.alpha_composite(img.convert("RGBA"), overlay)


# ---------------------------------------------------------------------------
# Aging / post-processing
# ---------------------------------------------------------------------------

def age_image(img: Image.Image) -> Image.Image:
    """Apply subtle aging: slight yellow tint, soft vignette, mild blur at edges."""
    img = img.convert("RGBA")
    w, h = img.size

    # Slight warm/yellow tint overlay.
    tint = Image.new("RGBA", (w, h), (210, 190, 140, 25))
    img = Image.alpha_composite(img, tint)

    # Soft corner vignette.
    import math
    vignette = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    vp = vignette.load()
    cx, cy = w / 2, h / 2
    max_dist = math.sqrt(cx * cx + cy * cy)
    for y in range(h):
        for x in range(w):
            dist = math.sqrt((x - cx) ** 2 + (y - cy) ** 2)
            # Only darken outer 30%.
            t = max(0.0, (dist / max_dist - 0.7) / 0.3)
            alpha = int(t * 60)
            vp[x, y] = (30, 25, 15, alpha)
    img = Image.alpha_composite(img, vignette)

    return img


# ---------------------------------------------------------------------------
# OpenAI image generation
# ---------------------------------------------------------------------------

def generate_box_art(client: OpenAI, slug: str) -> Image.Image:
    """Call the OpenAI API and return a PIL Image (RGBA)."""
    rarity = RELIC_RARITY.get(slug, "common")
    prompt = build_style_prefix(rarity) + "\nSUBJECT:\n" + RELIC_PROMPTS[slug]

    print(f"  [{slug}] requesting image from OpenAI...")
    response = client.images.generate(
        model="gpt-image-1",
        prompt=prompt,
        n=1,
        size="1024x1024",
        quality="high",
    )

    b64 = response.data[0].b64_json
    img_bytes = base64.b64decode(b64)
    img = Image.open(io.BytesIO(img_bytes))
    return img.convert("RGBA")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(
        description="Generate relic box-art icons via OpenAI image generation."
    )
    parser.add_argument(
        "slugs",
        nargs="*",
        default=[],
        help="Which relic(s) to generate (by slug, e.g. 'triplet_boost'). Omit for all.",
    )
    parser.add_argument(
        "--preview",
        action="store_true",
        help=f"Output at {PREVIEW_SIZE}x{PREVIEW_SIZE} instead of {SIZE}x{SIZE}.",
    )
    parser.add_argument(
        "--no-age",
        action="store_true",
        help="Skip the aging/vignette post-processing.",
    )
    parser.add_argument(
        "--no-border",
        action="store_true",
        help="Skip the rarity-colored border.",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="List all available relic slugs and exit.",
    )
    args = parser.parse_args()

    if args.list:
        for slug in sorted(RELIC_PROMPTS.keys()):
            rarity = RELIC_RARITY.get(slug, "?")
            print(f"  {slug:24s}  ({rarity})")
        return

    slugs = args.slugs if args.slugs else list(RELIC_PROMPTS.keys())

    # Validate slugs.
    bad = [s for s in slugs if s not in RELIC_PROMPTS]
    if bad:
        print(f"Error: unknown relic slug(s): {', '.join(bad)}", file=sys.stderr)
        print(f"  Use --list to see available slugs.", file=sys.stderr)
        sys.exit(1)

    api_key = os.environ.get("OPENAI_API_KEY")
    if not api_key:
        print("Error: set OPENAI_API_KEY environment variable.", file=sys.stderr)
        sys.exit(1)

    client = OpenAI(api_key=api_key)

    os.makedirs(OUT_DIR, exist_ok=True)
    out_size = PREVIEW_SIZE if args.preview else SIZE
    print(f"Generating relic box art for {len(slugs)} relic(s) at {out_size}x{out_size}")

    for slug in slugs:
        img = generate_box_art(client, slug)

        # Post-processing.
        if not args.no_age:
            img = age_image(img)

        if not args.no_border:
            rarity = RELIC_RARITY.get(slug, "common")
            border_w = 6 if out_size >= PREVIEW_SIZE else 4
            img = apply_rarity_border(img, rarity, width=border_w)

        # Downscale to final size.
        img = img.resize((out_size, out_size), Image.LANCZOS)

        out_path = os.path.join(OUT_DIR, f"{slug}.png")
        img.save(out_path)
        print(f"  wrote {out_path}  ({os.path.getsize(out_path)} bytes)")

    print("Done.")


if __name__ == "__main__":
    main()
