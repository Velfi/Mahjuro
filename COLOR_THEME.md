# Mahjuro Color Theme — Walnut, Brass & Felt

A late-night palette for the **shuttered House** described in [`THEME.md`](THEME.md). Walnut paneling, brass fittings, candlelight pools, and small pockets of mahjong felt — wrapped in lacquer-deep shadow. The base stays moody and small; scoring, fireworks, and smoke are *the* sources of color.

The **single source of truth for code** is [`src/render/theme.rs`](src/render/theme.rs). Every token below has a corresponding constant there. This doc is the *design* layer: rationale, material story, and where each token belongs.

---

## Design intent

- **One lantern ahead.** Light is precious, never ambient. Backgrounds slide toward black; highlights are pulled out of darkness, not painted on top of it.
- **Warm interior, cold elsewhere.** The House itself is candlelit walnut. Anything *outside* the House — sky, journal, archive ledger — leans cool. The contrast is the dread.
- **Brass, not gold.** Aged, slightly burnt. Real gold-leaf has shadow inside it. Pure-yellow accents read as UI; brass reads as *fixture*.
- **Parchment, not white.** Body text is a warm off-white. Pure white is reserved for tile faces and lit candle wax — it's a *material*, not a color slot.
- **Quiet base, loud effects.** Rule-breaking moments (cascades, fireworks, smoke) explode against a flat, darkened room. If the base ever competes with the fireworks, the base is wrong.

---

## Walnut ladder — the room itself

Backgrounds, panels, modals, tooltips, button rests. Lean dark; only step up the ladder where focus genuinely needs to land.

| Token | RGBA | Hex | Where |
|---|---|---|---|
| `WALNUT_INK` | `[0.040, 0.031, 0.024, 1.0]` | `#0A0806` | Deepest base. Behind everything. Edges of the frame. |
| `WALNUT_DEEP` | `[0.071, 0.055, 0.043, 1.0]` | `#120E0B` | Modal/panel background, tooltip fill. |
| `WALNUT_RAISED` | `[0.110, 0.086, 0.067, 1.0]` | `#1C1611` | Raised panel, one step above `WALNUT_DEEP`. |
| `WALNUT_SOFT` | `[0.165, 0.129, 0.102, 1.0]` | `#2A211A` | Hover/selected panel; default button rest. |
| `WALNUT_BRIGHT` | `[0.212, 0.165, 0.129, 1.0]` | `#362A21` | Strongest panel tone; primary button rest. |

**Rule:** never use a flat mid-gray for a panel. Every recess is wood; every step up the ladder is *lit walnut*, not an "elevated surface."

## Brass — the fixtures

Sparing. Reserve for headers, score numerals, selected-tile rims, currency, relic borders, signage. If it's gold, it should feel **forged**.

| Token | RGBA | Hex | Where |
|---|---|---|---|
| `CHAMPAGNE` | `[0.961, 0.776, 0.455, 1.0]` | `#F5C674` | Hero score numerals, selected-tile rims. The brightest brass. |
| `GOLD` | `[0.910, 0.694, 0.290, 1.0]` | `#E8B14A` | Headers, currency, primary button border on hover. |
| `BRASS` | `[0.784, 0.565, 0.118, 1.0]` | `#C8901E` | Default fixture borders, inset frames. |
| `ANTIQUE` | `[0.541, 0.369, 0.078, 1.0]` | `#8A5E14` | Shadow lines under brass, deepest fixture tone. |

**Rule:** if you find yourself reaching for brass on a button label, ask first whether the *border* should be brass and the label should stay parchment. Brass on text turns ceremonial fast.

## Neutrals — paper and stone

Body copy, secondary labels, dividers. Warm enough to belong on walnut.

| Token | RGBA | Hex | Where |
|---|---|---|---|
| `PARCHMENT` | `[0.957, 0.945, 0.910, 1.0]` | `#F4F1E8` | All body text. **Never use pure white.** |
| `STONE` | `[0.722, 0.682, 0.635, 1.0]` | `#B8AEA2` | Captions, secondary labels, inactive state, common-rarity tag. |
| `UMBER` | `[0.388, 0.361, 0.322, 1.0]` | `#635C52` | Tertiary text, disabled labels, dividers. |

## Semantic accents

Desaturated on purpose so they sit on warm wood instead of vibrating against it.

| Token | RGBA | Hex | Meaning |
|---|---|---|---|
| `JADE` | `[0.373, 0.831, 0.659, 1.0]` | `#5FD4A8` | Success, target met, valid, uncommon rarity. |
| `RUBY` | `[0.910, 0.353, 0.420, 1.0]` | `#E85A6B` | Danger, exit, abandon, destroy. |
| `AMBER` | `[0.941, 0.659, 0.282, 1.0]` | `#F0A848` | Warning, attention, "this will cost you." |

**Rule:** `JADE` is *semantic* (a UI signal). The mahjong tabletop's green is a *material* — see Felt below — and shouldn't be sourced from `JADE`.

---

## Material colors used in scenes (not tokens yet)

These are surfaces the game already shows but `theme.rs` doesn't formally name. Keep them consistent in code as ad-hoc literals; if any of them get reused enough, promote them to `theme.rs` and update this section.

### Felt — cabinet linings & tabletops

The signature mahjong green. Appears in archive cubby linings, the play table, scoring panel backings.

| Concept | RGB | Hex | Notes |
|---|---|---|---|
| Felt deep | `(0.10, 0.16, 0.13)` | `#1A2A20` | In-shadow felt, e.g. cubby corners. |
| Felt | `(0.18, 0.29, 0.22)` | `#2D4A38` | Lit tabletop, archive cubby lining. |
| Felt highlight | `(0.29, 0.42, 0.32)` | `#4A6B52` | Felt directly under a candle/lamp. |

**Rule:** felt is *forest tea*, not emerald. If it reads as Christmas green, it's wrong.

### Twilight — the world outside

The cool counterpoint. Dusk sky behind the main menu, shop wall plaster, info-modal backings, journal page tints, anything "looking out from inside."

| Concept | RGB | Hex | Notes |
|---|---|---|---|
| Twilight ink | `(0.055, 0.078, 0.133)` | `#0E1422` | Distant night through a window. |
| Twilight | `(0.118, 0.165, 0.251)` | `#1E2A40` | Shop walls, info modal background. |
| Twilight glow | `(0.353, 0.431, 0.580)` | `#5A6E94` | Distant lit window, hint text on cool surfaces. |

**Rule:** never use a saturated UI blue for "info." Reach for `TWILIGHT` first — the message is *the cold outside*, not *informational notice*.

### Lacquer — black & ceremonial red

Deep contrast. Counters, framing, ceremonial signage.

| Concept | RGB | Hex | Notes |
|---|---|---|---|
| Lacquer | `(0.039, 0.024, 0.031)` | `#0A0608` | True black-with-warmth. Counter tops, deepest shadow. |
| Cinnabar | `(0.722, 0.196, 0.157)` | `#B83228` | Ceremonial red. Character-suit ink, dragon "Chun", celebration sigils. **Distinct from `RUBY`.** |

**Rule:** `RUBY` is danger UI. `CINNABAR` is ceremony. They should never share a job. If a celebration moment uses `RUBY`, it'll feel like an error popup.

### Tallow — candle bloom

Warm highlight color. Use for emissive bloom, particle cores, lantern halos, "this just happened" flashes.

| Concept | RGB | Hex | Notes |
|---|---|---|---|
| Tallow | `(1.000, 0.910, 0.722)` | `#FFE8B8` | Candle-bloom center, score-pop core, lantern halo. |

**Rule:** highlights pull toward `TALLOW`, not toward white. Light in this game is **made of fire**, not screens.

---

## Suit colors

Live in [`src/core/tile.rs`](src/core/tile.rs). Spread across the wheel for instant readability at any tile size.

| Suit | RGBA | Hex | Feel |
|---|---|---|---|
| Characters | `[0.85, 0.25, 0.20, 1.0]` | `#D94033` | Cinnabar — ink-stamped on ivory. |
| Bamboos | `[0.20, 0.65, 0.30, 1.0]` | `#33A64D` | Felt-adjacent green. |
| Circles | `[0.20, 0.40, 0.80, 1.0]` | `#3366CC` | Sapphire. |
| Wind | `[0.70, 0.60, 0.20, 1.0]` | `#B39933` | Faded gold leaf. |
| Dragon (Chun, rank 1) | `[0.85, 0.20, 0.18, 1.0]` | `#D9332E` | Red dragon — `中`. |
| Dragon (Hatsu, rank 2) | `[0.20, 0.65, 0.30, 1.0]` | `#33A64D` | Green dragon — `發`. |
| Dragon (Haku, rank 3) | `[0.90, 0.88, 0.82, 1.0]` | `#E6E0D1` | White dragon — ivory blank. |
| Flower | `[0.90, 0.45, 0.55, 1.0]` | `#E67389` | Plum/cherry pink. |
| Season | `[0.30, 0.70, 0.65, 1.0]` | `#4DB3A6` | Cool teal — distinct from flowers. |

## Rarity spectrum

Live in `theme::color::rarity(tier)`. Echoes existing tokens for harmony.

| Rarity | Token | Hex |
|---|---|---|
| Common | `STONE` | `#B8AEA2` |
| Uncommon | `JADE` | `#5FD4A8` |
| Rare | `WALNUT_BRIGHT` | `#362A21` |
| Legendary | `CHAMPAGNE` | `#F5C674` |

**Note:** rare uses a panel tone, not a saturated color, on purpose — rare relics borrow the warmth of *more wood*, legendaries earn the brass.

## Tile surface

| Element | RGB | Hex | Notes |
|---|---|---|---|
| Face | `(0.95, 0.92, 0.85)` | `#F2EBD9` | Warm ivory. Every other "white" in the game has to lose to this one. |
| Edge (light) | `(0.60, 0.48, 0.28)` | `#997A47` | Bamboo tan bevel. |
| Edge (dark) | `(0.45, 0.35, 0.20)` | `#735933` | Walnut bevel. |

---

## Buttons

Resolved by `theme::button_colors(variant, state)`. Don't hand-roll button colors in scenes — go through this function so every button in the game speaks the same dialect.

| Variant | Rest bg | Border | Text |
|---|---|---|---|
| Default | `WALNUT_SOFT` | `BRASS` | `PARCHMENT` |
| Primary | `WALNUT_BRIGHT` | `GOLD` | `CHAMPAGNE` |
| Danger | `WALNUT_RAISED` | `RUBY` | `RUBY` |
| Subtle | `WALNUT_DEEP` | `UMBER` | `STONE` |

State transitions: **hover** lightens the bg ~15% and switches the border to `GOLD`; **press** darkens ~18%; **disabled** darkens ~35% and dims the border and text.

---

## Effects

The effects layer is where the palette gets to misbehave. The base is restrained so these can land hard.

### Fireworks (scoring cascade)

Color comes from the **scoring tile** that triggered the burst — a Characters score throws cinnabar shards, a Bamboo score throws felt-green ones. Use the suit color directly; do not desaturate. The shard core blooms toward `TALLOW` for a moment before falling back to the suit hue.

### Score particles

Burst color is the suit that scored, with a `TALLOW` core that decays out over the particle lifetime.

---

## Rationale

The visual story of Mahjuro is a building. Wood, brass, felt, paper, candlelight — a parlor that **remembers** the people who lost in it. The palette has to make that building feel real before it can carry any of the gameplay's chaos.

Walnut + brass + parchment is the room. Felt and cinnabar are the *ritual* (mahjong itself). Twilight is what's *outside* the room — the cold thing the House is between you and. Lacquer is the deepest contrast, used to frame and to swallow.

When fireworks go off, they don't compete with the room. They erupt out of it. That's why everything else stays small.
