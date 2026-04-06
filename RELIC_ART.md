# Mahjuro — Relic Art Specs

## Style Guide

**Overall Aesthetic:** Sardonic vector art — bold, flat, deadpan humor. Objects that
look slightly too self-aware. Thick black outlines, solid fills, 2-3 accent colors per
icon. Muted background tones (slate, dusty purple, warm grey) so bold vector pops.

**Canvas:** 512×512px, transparent background, centered composition.
Each icon should read clearly when scaled down to 64×64 in the HUD relic bar.

**Color Language:**
- Bamboo suit = green (#4CAF50)
- Circles suit = blue (#42A5F5)
- Characters suit = red (#EF5350)
- Honor/Dragon = gold (#FFD54F) / white (#FAFAFA)
- Backgrounds: muted (slate #5C6370, dusty purple #7E6B8F, warm grey #A0937D)

**Material FX (applied at runtime via shader, NOT baked into the icon):**
- Common: matte, no FX
- Uncommon: subtle shimmer (UV-scrolling noise)
- Rare: holographic color shift + rainbow edge glow
- Legendary: liquid gold flow distortion + emissive bleed

---

## Relic Icon Specs (20 Relics)

### 1. Triplet Boost
- **Visual:** Three identical mahjong tiles stuffed into a single oversized trenchcoat,
  the top tile peering out nervously. A bold "×2" floats above.
- **Palette:** Ivory tiles, tan trenchcoat, red ×2 text.
- **Rarity:** Common
- **Humor:** "Three tiles in a trenchcoat pretending to be one very powerful tile."

### 2. Sequence Surge
- **Visual:** Three tiles numbered 1-2-3 riding a lightning bolt like a surfboard,
  the middle tile looking bored while the others scream.
- **Palette:** Blue lightning, ivory tiles, yellow sparks.
- **Rarity:** Uncommon
- **Humor:** "The 2 has done this a thousand times."

### 3. Pair Power
- **Visual:** Two tiles aggressively fist-bumping, tiny impact lines radiating from
  the bump. Both tiles wear matching sweatbands.
- **Palette:** Red sweatbands, ivory tiles, orange impact lines.
- **Rarity:** Uncommon
- **Humor:** "Gym bros but make it mahjong."

### 4. Honor Fury
- **Visual:** An honor tile (wind symbol) with veins popping out and steam coming
  from the top. Tiny cracked floor beneath it. A "×3" badge in the corner.
- **Palette:** Gold tile face, dark red veins, grey steam.
- **Rarity:** Rare
- **Humor:** "It has had ENOUGH of being undervalued."

### 5. Bamboo Charm
- **Visual:** A single bamboo stalk wearing a tiny top hat and monocle, leaning on
  a cane. A "+2" floats beside it like a speech bubble.
- **Palette:** Green bamboo, black top hat, gold monocle rim.
- **Rarity:** Common
- **Humor:** "Distinguished bamboo. Refined bamboo. +2 bamboo."

### 6. Red Dragon Rage
- **Visual:** A red dragon tile that has literally caught fire. It looks mildly
  inconvenienced rather than alarmed. A "×5" burns in the flames above.
- **Palette:** Deep red base, orange/yellow flames, white ×5 text.
- **Rarity:** Legendary
- **Humor:** "This is fine."

### 7. Green Luck
- **Visual:** A four-leaf clover where one leaf is clearly wilting and held up with
  tape. A small heart (HP) icon floats above with a "+" sign.
- **Palette:** Green clover, beige tape, pink heart.
- **Rarity:** Common
- **Humor:** "Close enough to lucky."

### 8. White Silence
- **Visual:** A white dragon tile wearing noise-canceling headphones, eyes closed
  in bliss. A frozen clock icon in the corner with icicles hanging off it.
- **Palette:** White tile, matte black headphones, icy blue clock.
- **Rarity:** Rare
- **Humor:** "Do not disturb. Timer frozen."

### 9. Joker Tile
- **Visual:** A mahjong tile wearing a comically oversized fake mustache, googly
  eyes, and a tiny hat that's falling off. A "?" symbol on its face.
- **Palette:** Ivory tile, black mustache, red hat, yellow "?".
- **Rarity:** Rare
- **Humor:** "Nobody will ever know."

### 10. Overflow
- **Visual:** A wooden bucket tipping over, mahjong tiles spilling out in a cascade.
  The bucket has a crack and looks exasperated (drawn-on face).
- **Palette:** Brown bucket, ivory tiles, blue splash lines.
- **Rarity:** Uncommon
- **Humor:** "The bucket tried its best."

### 11. Quick Draw
- **Visual:** A mahjong tile dressed as a cowboy — tiny hat, holster belt — mid-draw
  with two tiles in hand. Dust cloud at its feet.
- **Palette:** Tan cowboy hat, brown belt, ivory tiles, sandy dust.
- **Rarity:** Uncommon
- **Humor:** "Fastest draw in the East (Wind)."

### 12. Chain Reaction
- **Visual:** A line of mahjong tiles set up like dominoes, mid-topple. The first
  tile is smugly leaning back watching the chaos. Small explosion stars at each impact.
- **Palette:** Ivory tiles, orange/yellow impact stars, grey shadow.
- **Rarity:** Rare
- **Humor:** "Started something it can't stop."

### 13. Multiplier Master
- **Visual:** A tile wearing a graduation cap and tiny glasses, holding a chalkboard
  that shows "×1.1 → ×1.2 → ×1.3 → ∞". The tile looks exhausted but proud.
- **Palette:** Black grad cap, green chalkboard, white chalk text, ivory tile.
- **Rarity:** Rare
- **Humor:** "It did the math. All of it."

### 14. Set Magnet
- **Visual:** A horseshoe magnet crackling with energy, pulling a mahjong tile
  through the air toward it. The tile looks startled mid-flight.
- **Palette:** Red/silver magnet, blue energy arcs, ivory tile, motion lines.
- **Rarity:** Uncommon
- **Humor:** "You didn't ask to be here, but here you are."

### 15. Wild Winds
- **Visual:** Four wind tiles (E/S/W/N) spinning in a mini tornado, their symbols
  scrambled and swapping between them. All four look dizzy.
- **Palette:** Teal tornado, ivory tiles, mixed suit-colored symbols.
- **Rarity:** Rare
- **Humor:** "Nobody knows which direction anymore."

### 16. Dragon Echo
- **Visual:** A dragon tile shouting into a canyon, with smaller copies of itself
  bouncing back as echoes, each one slightly more faded. Musical note symbols.
- **Palette:** Red dragon, fading red echoes, grey canyon walls, yellow notes.
- **Rarity:** Legendary
- **Humor:** "It just likes hearing itself score."

### 17. Reverse Tile
- **Visual:** Two tiles mid-swap, connected by swirling arrows. One tile is upside
  down and looks confused; the other is smugly right-side up.
- **Palette:** Ivory tiles, purple swap arrows, yellow confusion stars.
- **Rarity:** Common
- **Humor:** "Identity crisis, but make it useful."

### 18. Stealth Tile
- **Visual:** A mahjong tile wearing a black ninja outfit, crouched and holding a
  finger to where its lips would be. Only its eyes are visible. Shadow behind it.
- **Palette:** Black ninja suit, ivory eyes, dark grey shadow.
- **Rarity:** Uncommon
- **Humor:** "You saw nothing."

### 19. Locked Set
- **Visual:** Three tiles chained together with a padlock, looking resigned to their
  fate. The padlock has a tiny smug face. A "3 turns" badge in the corner.
- **Palette:** Ivory tiles, grey chain, gold padlock, red badge.
- **Rarity:** Common
- **Humor:** "Nobody leaves this triplet."

### 20. Lucky Pair
- **Visual:** Two tiles on a tiny date — one offers the other a flower, both are
  blushing. A multiplier heart floats above them with a "×" inside.
- **Palette:** Ivory tiles, pink blush, red flower, pink heart with gold ×.
- **Rarity:** Common
- **Humor:** "Love is a multiplier."
