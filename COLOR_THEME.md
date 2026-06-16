# Mahjuro Color Theme — Walnut, Brass & Felt

A late-night palette for the **shuttered House** described in [`THEME.md`](THEME.md). Walnut paneling, brass fittings, candlelight pools, and small pockets of mahjong felt — wrapped in lacquer-deep shadow. The base stays moody and small; scoring, fireworks, and smoke are *the* sources of color.

The **single source of truth for code** is [`src/render/theme.rs`](src/render/theme.rs). Every token below has a corresponding constant there. This doc is the *design* layer: rationale, material story, and where each token belongs.

---

## Design intent

- **One lantern ahead.** Light is precious, never ambient. Backgrounds slide toward black; highlights are pulled out of darkness, not painted on top of it.
- **Warm interior, cold elsewhere.** The House itself is candlelit walnut. Anything *outside* the House — sky, journal, archive ledger — leans cool. The contrast is the dread.
- **Brass, not gold.** Aged, slightly burnt. Real gold-leaf has shadow inside it. Pure-yellow accents read as UI; brass reads as *fixture*.
- **Cream on dark walnut.** Body text is warm `PARCHMENT` cream, not pure white. Warmth comes from the brown base and brass accents, not from neon UI chrome.
- **Quiet base, loud effects.** Rule-breaking moments (cascades, fireworks, smoke) explode against a flat, darkened room. If the base ever competes with the fireworks, the base is wrong.

---

## Walnut ladder — the room itself

Backgrounds, panels, modals, tooltips, button rests. Lean dark; only step up the ladder where focus genuinely needs to land.

| Token | RGBA | Hex | Where |
|---|---|---|---|
| `WALNUT_INK` | `[0.019, 0.012, 0.009, 1.0]` | `#040302` | Deepest base. Behind everything. Edges of the frame. |
| `WALNUT_DEEP` | `[0.040, 0.024, 0.018, 1.0]` | `#0A0604` | Modal/panel background, tooltip fill. |
| `WALNUT_RAISED` | `[0.073, 0.052, 0.036, 1.0]` | `#120D09` | Raised panel, one step above `WALNUT_DEEP`. |
| `WALNUT_SOFT` | `[0.111, 0.080, 0.058, 1.0]` | `#1C140E` | Hover/selected panel; default button rest. |
| `WALNUT_BRIGHT` | `[0.154, 0.112, 0.083, 1.0]` | `#271C15` | Strongest panel tone; primary button rest. |

**Rule:** never use a flat mid-gray for a panel. Every recess is wood; every step up the ladder is *lit walnut*, not an "elevated surface."

## Brass — the fixtures

Sparing. Reserve for headers, score numerals, selected-tile rims, currency, relic borders, signage. If it's gold, it should feel **forged**.

| Token | RGBA | Hex | Where |
|---|---|---|---|
| `CHAMPAGNE` | `[0.855, 0.786, 0.666, 1.0]` | `#DAC8A9` | Hero score numerals, selected-tile rims. The brightest brass. |
| `GOLD` | `[0.785, 0.704, 0.553, 1.0]` | `#C8B38D` | Headers, currency, primary button border on hover. |
| `BRASS` | `[0.663, 0.573, 0.392, 1.0]` | `#A89263` | Default fixture borders, inset frames. |
| `ANTIQUE` | `[0.454, 0.377, 0.248, 1.0]` | `#73603F` | Shadow lines under brass, deepest fixture tone. |

**Rule:** if you find yourself reaching for brass on a button label, ask first whether the *border* should be brass and the label should stay parchment. Brass on text turns ceremonial fast.

## Neutrals — paper and stone

Body copy, secondary labels, dividers. Warm enough to belong on walnut.

| Token | RGBA | Hex | Where |
|---|---|---|---|
| `PARCHMENT` | `[0.960, 0.955, 0.940, 1.0]` | `#F5F3F0` | All body text. Warm cream on the dark walnut ladder. |
| `STONE` | `[0.716, 0.683, 0.645, 1.0]` | `#B6AEA4` | Captions, secondary labels, inactive state, common-rarity tag. |
| `UMBER` | `[0.385, 0.361, 0.328, 1.0]` | `#625C53` | Tertiary text, disabled labels, dividers. |

## Semantic accents

Desaturated on purpose so they sit on warm wood instead of vibrating against it.

| Token | RGBA | Hex | Meaning |
|---|---|---|---|
| `JADE` | `[0.613, 0.755, 0.702, 1.0]` | `#9CC0B3` | Success, target met, valid, uncommon rarity. |
| `RUBY` | `[0.611, 0.438, 0.459, 1.0]` | `#9B6F74` | Danger, exit, abandon, destroy. |
| `AMBER` | `[0.776, 0.680, 0.553, 1.0]` | `#C5AD8C` | Warning, attention, "this will cost you." |
| `LAPIS` | `[0.686, 0.757, 0.825, 1.0]` | `#AEC0D2` | Cool score signal (Chips), info Fu, soft boss tier. |

## Chart encodings (`color::chart::*`)

More chroma than UI semantics — for bars, sparklines, outcome strips, and other data where hue carries meaning. Chronicle and shared chart helpers use these; labels and chrome stay on the muted ladder above.

| Token | RGBA | Hex | Meaning |
|---|---|---|---|
| `chart::POSITIVE` | `[0.384, 0.722, 0.580, 1.0]` | `#62B894` | Wins, victory bars, average lines. |
| `chart::NEGATIVE` | `[0.780, 0.361, 0.400, 1.0]` | `#C75C66` | Losses, defeat bars. |
| `chart::HIGHLIGHT` | `[0.851, 0.651, 0.345, 1.0]` | `#D9A658` | Peaks, KPI sparklines, last-bar emphasis. |
| `chart::ACCENT` | `[0.722, 0.573, 0.290, 1.0]` | `#B8924A` | Secondary warm accent (stamps, bar rims). |
| `chart::FILL` | `[0.722, 0.627, 0.471, 1.0]` | `#B8A078` | Neutral magnitude fills (distribution segments). |

**Rule:** `JADE` is *semantic* (a UI signal). The mahjong tabletop's green is a *material* — see Felt below — and shouldn't be sourced from `JADE`. Charts that encode win/loss or series data should use `color::chart::*`, not `JADE` / `RUBY` / `GOLD`.

---

## Suit colors

Live in [`src/core/tile.rs`](src/core/tile.rs). Spread across the wheel for instant readability at any tile size.

| Suit | RGBA | Hex | Feel |
|---|---|---|---|
| Manzu | `[0.581, 0.320, 0.298, 1.0]` | `#94514B` | Cinnabar — ink-stamped on ivory. |
| Souzu | `[0.386, 0.582, 0.429, 1.0]` | `#62946D` | Felt-adjacent green. |
| Pinzu | `[0.306, 0.393, 0.567, 1.0]` | `#4D6490` | Sapphire. |
| Wind | `[0.639, 0.596, 0.422, 1.0]` | `#A2976B` | Faded gold leaf. |
| Dragon (Chun, rank 1) | `[0.560, 0.277, 0.268, 1.0]` | `#8E4644` | Red dragon — `中`. |
| Dragon (Hatsu, rank 2) | `[0.386, 0.582, 0.429, 1.0]` | `#62946D` | Green dragon — `發`. |
| Dragon (Haku, rank 3) | `[0.90, 0.88, 0.82, 1.0]` | `#E6E0D1` | White dragon — ivory blank. |
| Flower | `[0.704, 0.508, 0.552, 1.0]` | `#B3818C` | Plum/cherry pink. |
| Season | `[0.476, 0.650, 0.628, 1.0]` | `#79A5A0` | Cool teal — distinct from flowers. |

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

Color comes from the **scoring tile** that triggered the burst — a Manzu score throws cinnabar shards, a Souzu score throws felt-green ones. Use the suit color directly; do not desaturate. The shard core blooms toward `PARCHMENT` for a moment before falling back to the suit hue.

### Score particles

Burst color is the suit that scored, with a `PARCHMENT` core that decays out over the particle lifetime.

---

## Rationale

The visual story of Mahjuro is a building. Wood, brass, felt, paper, candlelight — a parlor that **remembers** the people who lost in it. The palette has to make that building feel real before it can carry any of the gameplay's chaos.

Walnut + brass + parchment is the room. Felt and cinnabar are the *ritual* (mahjong itself). Twilight is what's *outside* the room — the cold thing the House is between you and. Lacquer is the deepest contrast, used to frame and to swallow.

When fireworks go off, they don't compete with the room. They erupt out of it. That's why everything else stays small.
