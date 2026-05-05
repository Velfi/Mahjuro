# Mahjuro Color Theme: Midnight Gold

A dark, warm palette inspired by a late-night mahjong parlor — deep indigo backgrounds with gold lamplight cutting through. The base stays quiet so scoring events, fireworks, and smoke effects feel explosive by contrast.

---

## Foundation

| Role | RGBA | Hex | Notes |
|------|------|-----|-------|
| Void | `[0.04, 0.05, 0.08, 1.0]` | `#0A0D14` | Primary scene background |
| Clear | `[0.06, 0.07, 0.10, 1.0]` | `#0F1219` | Render pass clear color |
| Surface | `[0.10, 0.12, 0.20, 0.85]` | `#1A1F33` | Cards, panels, unfocused rows |
| Elevated | `[0.15, 0.18, 0.28, 0.85]` | `#262E47` | Unfocused buttons |

## Accent Colors

| Role | RGBA | Hex | Usage |
|------|------|-----|-------|
| Title Gold | `[1.0, 0.95, 0.70, 1.0]` | `#FFF2B3` | Titles, headlines |
| Bright Gold | `[0.9, 0.8, 0.2, 0.95]` | `#E6CC33` | Confirm buttons, focus highlights |
| Action Blue | `[0.25, 0.40, 0.60, 0.95]` | `#406699` | Default interactive elements |
| Go Green | `[0.20, 0.55, 0.30, 0.95]` | `#338C4D` | Continue, resume, play, valid |
| Danger Red | `[0.55, 0.20, 0.20, 0.95]` | `#8C3333` | Quit, cancel, discard |

## Text Hierarchy

| Role | RGBA | Hex |
|------|------|-----|
| Headline | `[1.0, 0.95, 0.70, 1.0]` | `#FFF2B3` |
| Primary | `[1.0, 1.0, 1.0, 1.0]` | `#FFFFFF` |
| Secondary | `[0.6, 0.6, 0.7, 0.9]` | `#9999B3` |
| Hint | `[0.4, 0.4, 0.5, 0.8]` | `#666680` |
| Disabled | `[0.35, 0.35, 0.35, 0.6]` | `#595959` |

## Suit Colors

Each mahjong suit has a distinct hue, spread across the color wheel for instant readability at any size.

| Suit | RGBA | Hex | Feel |
|------|------|-----|------|
| Characters | `[0.85, 0.25, 0.20, 1.0]` | `#D94033` | Vermillion |
| Bamboos | `[0.20, 0.65, 0.30, 1.0]` | `#33A64D` | Emerald |
| Circles | `[0.20, 0.40, 0.80, 1.0]` | `#3366CC` | Sapphire |
| Winds | `[0.70, 0.60, 0.20, 1.0]` | `#B39933` | Amber |
| Dragons | `[0.60, 0.20, 0.70, 1.0]` | `#9933B3` | Amethyst |

## Rarity Spectrum

| Rarity | RGBA | Hex |
|--------|------|-----|
| Common | `[0.55, 0.55, 0.55, 0.9]` | `#8C8C8C` |
| Uncommon | `[0.30, 0.75, 0.30, 0.9]` | `#4DBF4D` |
| Rare | `[0.30, 0.50, 1.00, 0.9]` | `#4D80FF` |
| Legendary | `[1.00, 0.78, 0.15, 0.9]` | `#FFC726` |

## Tile Surface

| Element | RGB | Hex | Notes |
|---------|-----|-----|-------|
| Face | `(0.95, 0.92, 0.85)` | `#F2EBD9` | Warm ivory, like real tiles |
| Edge (light) | `(0.60, 0.48, 0.28)` | `#997A47` | Bamboo tan |
| Edge (dark) | `(0.45, 0.35, 0.20)` | `#735933` | Walnut bevel |

## Interactive Elements

| Element | Focused | Unfocused |
|---------|---------|-----------|
| Option rows | `[0.20, 0.32, 0.50, 0.90]` | `[0.12, 0.15, 0.24, 0.75]` |
| Slider fill | `[0.35, 0.65, 0.90, 1.0]` | `[0.22, 0.42, 0.62, 0.85]` |
| Tabs | `[0.22, 0.38, 0.58, 0.95]` | `[0.10, 0.12, 0.20, 0.85]` |
| Gameplay buttons | `[0.22, 0.38, 0.55, 0.92]` | — |

## Modal Themes

### Success (round wins, level-ups)

| Element | RGBA | Hex |
|---------|------|-----|
| Background | `[0.12, 0.14, 0.08, 0.95]` | `#1F2414` |
| Border | `[0.85, 0.75, 0.20, 0.90]` | `#D9BF33` |
| Title | `[1.0, 0.92, 0.40, 1.0]` | `#FFEB66` |
| Body | `[0.9, 0.88, 0.70, 1.0]` | `#E6E0B3` |

### Failure (game over)

| Element | RGBA | Hex |
|---------|------|-----|
| Background | `[0.18, 0.06, 0.06, 0.95]` | `#2E0F0F` |
| Border | `[0.70, 0.15, 0.15, 0.90]` | `#B32626` |
| Title | `[1.0, 0.40, 0.35, 1.0]` | `#FF6659` |
| Body | `[0.85, 0.75, 0.70, 1.0]` | `#D9BFB3` |

### Info (neutral)

| Element | RGBA | Hex |
|---------|------|-----|
| Background | `[0.08, 0.10, 0.18, 0.95]` | `#141A2E` |
| Border | `[0.30, 0.50, 0.80, 0.90]` | `#4D80CC` |
| Title | `[0.60, 0.80, 1.0, 1.0]` | `#99CCFF` |
| Body | `[0.80, 0.85, 0.95, 1.0]` | `#CCD9F2` |

## Effects

### Fireworks

Six celebration colors, chosen for maximum contrast against the dark backgrounds:

| Name | RGB | Hex |
|------|-----|-----|
| Red | `(1.0, 0.3, 0.2)` | `#FF4D33` |
| Green | `(0.2, 0.9, 0.4)` | `#33E666` |
| Blue | `(0.3, 0.5, 1.0)` | `#4D80FF` |
| Gold | `(1.0, 0.85, 0.2)` | `#FFD933` |
| Magenta | `(0.9, 0.3, 0.9)` | `#E64DE6` |
| Cyan | `(0.2, 0.9, 0.9)` | `#33E6E6` |

Trail sparks: `(0.9, 0.8, 0.5)` / `#E6CC80`

### Smoke (fluid simulation)

Mouse cursor injects warm gold smoke: `[0.8, 0.6, 0.3]` / `#CC994D`. Game events inject suit-colored smoke matching the tile that triggered them. Intensity and opacity are configurable (Off / Subtle / Strong / Over the Top).

### Score Particles

Gold burst on scoring: `[1.0, 0.85, 0.3, 1.0]` / `#FFD94D`

---

## Design Rationale

The dark indigo foundation evokes a late-night mahjong parlor. Warm gold cuts through it like lamplight on lacquered tiles. This high-contrast pairing keeps text readable and gives the UI a premium feel without competing with gameplay.

The five suit colors are deliberately spread across the color wheel so they remain distinguishable at small sizes. The rarity spectrum intentionally echoes the suit colors (jade green, sapphire blue, gold) for visual harmony.

The chaos lives in the effects layer — fireworks, score pops, and smoke. The base palette stays grounded so rule-breaking moments feel explosive by contrast.
