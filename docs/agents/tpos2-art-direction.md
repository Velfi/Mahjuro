# TPOS2 — Tile Pack Opening Sequence #2

Art direction for a **second** pack-opening celebration, parallel to the current shop flow ([`src/scenes/showcase/tile_pack.rs`](../../src/scenes/showcase/tile_pack.rs) + [`PackCelebration`](../../src/scenes/shop/shared.rs)). Working title: **TPOS2** (“The Unsealing”).

---

## Why a second sequence

**TPOS1** (today) is clear and functional: hero pack closeup → confirm → tiles ease from the box into a flat row. Motion is mostly **translation + scale** on one axis; lighting is warm but static; there is no intro wipe; the pack keeps bobbing until you click. It reads as “inventory UI,” not “loot moment.”

**TPOS2** should feel like **breaking a seal and releasing a hand of tiles** — one beat of ceremony, one beat of burst, one beat of inventory. Same inputs and events as TPOS1; different timing, camera, lighting, and motion.

---

## Emotional arc (4 beats)

| Beat | Player feeling | Duration (target) |
|------|----------------|-------------------|
| **Arrival** | “Something special is happening.” | ~1.0 s (skippable via fast confirm) |
| **Anticipation** | “I’m about to break the seal.” | Until confirm |
| **Unseal** | “It opened!” | ~0.55 s |
| **Deal & settle** | “Here’s what I got.” | ~1.2–2.4 s (tile-count dependent) |
| **Dismiss** | “Got it.” | Until confirm after settle |

Total active animation after first confirm: **~1.8–3.0 s** for 4–8 tiles (slightly longer than TPOS1; pay off with stronger motion).

---

## Phase machine

Replace TPOS1’s two phases (`Closeup` | `Reveal`) with five. Names are implementation-facing.

```
Arrival → Anticipation → Unseal → Deal → Settled
```

### 1. `Arrival`

**Goal:** Separate this overlay from “shop with a dimmer” the way zodiac uses a wipe.

**Visual**

- Reuse [`ShootingStarCelebrationIntro`](../../src/scenes/celebration_overlay.rs) with a **pack-tinted** variant (not zodiac gold):
  - Cascade tint lerps from black → `pack_palette::for_kind(kind).bg` (deep pack signature) → neutral dimmer.
  - `content_alpha_for` drives dimmer + pack visibility (same pattern as [`ZodiacPresenter`](../../src/scenes/showcase/zodiac.rs)).
- Optional mid-layer: **starfield** when `EffectLayers::starfield` is on (same gate as zodiac).
- Title: pack name in **champagne**, but fade in only after wipe hits 50% (avoid text fighting the wipe).

**Audio**

- `StarShimmer` on wipe start (existing).
- No `PackOpen` yet.

**Input**

- Fullscreen click does **not** skip the wipe (zodiac grace model); confirm during `Arrival` can fast-forward wipe via `jump_to_done` on headless only.

**Headless**

- `jump_to_done` immediately; land in `Anticipation` with pack visible.

---

### 2. `Anticipation` (replaces TPOS1 `Closeup`)

**Goal:** Tension without annoying idle bob.

**Pack**

- **Scale breathe** (subtle): `1.0 + 0.012 * sin(t * 1.1)` on screen height, not positional bob.
- **Seal glow**: fullscreen **soft quad** behind pack, color = `pack_palette.seal`, alpha `0.15 + 0.08 * sin(t * 2.0)`, blurred by large rect (no new shader required).
- Pack rotation: fixed hero angle (same closeup anchor as TPOS1) + **slow yaw** `±2°` only (less seasick than TPOS1’s dual-axis bob).
- Foil: multiply `Object3d.color` by a slow shimmer: lerp foil RGB toward white by `0.06 * (0.5 + 0.5 * sin(t * 3.2))`.

**Camera / framing**

- Pack box height fraction: **0.62** (vs TPOS1 `0.56`) — slightly more dominant.
- Spotlight: **tighter** cone, higher contrast; position as TPOS1 but `intensity` ~8.5, `cos_outer` ~30°.

**Lighting**

- Rim: cool blue point light **behind** pack (`pos` = anchor − Y offset), intensity keyed to `foil` hue.
- Key: warm spot unchanged in direction, slightly closer.

**Copy**

- Bottom: “Press confirm to **unseal**” (verb change matters — sets up phase 3).
- Title band unchanged.

**Input**

- Confirm → `Unseal`, fire `GameEvent::PackOpened`, reset phase `started_at`.

---

### 3. `Unseal` (~0.55 s)

**Goal:** One punchy “break” before tiles move. This is the moment TPOS1 skips entirely.

**Pack motion (all in ~0.55 s)**

| t (norm) | Pack |
|----------|------|
| 0.00–0.20 | **Snap scale** 1.0 → 1.06 (ease out cubic) |
| 0.15–0.35 | **Tilt back** +8° on X (lid opening illusion) |
| 0.25–0.55 | **Foil flash**: color → white ×1.4, decay to foil |
| 0.40–0.55 | Pack **alpha fade** / scale down to 0.85 (still drawn, but yields stage) |

**Screen**

- Dimmer pulse: alpha ×1.15 for 2 frames of feel (cap at 0.85).
- Optional: **radial burst quad** from pack center, seal color, alpha 0 → 0.25 → 0, radius expands 0 → 0.45 * min(w,h) in 0.35 s (cheap, no particles).

**Camera**

- Micro **dolly out**: increase `shop_celebration_camera` eye distance by 4% over 0.4 s (or FOV +1.5°) so tiles have room.

**Audio**

- `PackOpen` at t=0 (two-stage tear per [sound-design.md](../sound-design.md) when assets land).
- Optional: single low **whoosh** at t=0.25 (reuse `StarShimmer` at −6 dB only if it doesn’t clash).

**Input**

- Ignored (short lockout).

**At end**

- Auto-advance to `Deal`, reset `started_at`.

---

### 4. `Deal` (replaces TPOS1 `Reveal` motion)

**Goal:** Tiles feel **dealt from the pack**, not lerped to a spreadsheet row.

**Spawn**

- Origin: upper **lip** of pack (same as TPOS1 `src_lift` idea), slightly forward in Z.
- Per-tile stagger: **0.14 s** (faster than TPOS1 `0.18` — snappier pack).
- Per-tile flight: **0.42 s** (slightly longer to allow arc).

**Two-segment path per tile `i`**

Let `p = tile_progress(i)` ∈ [0, 1], `ease_out = 1 - (1-p)³`, `ease_in_out` = smoothstep.

1. **Arc segment** (p = 0..0.55):  
   - Position: quadratic Bezier from `S` (spawn) → `C` (control point above midline, lateral offset by fan angle) → midpoint `M`.  
   - Fan angle: `θ_i = lerp(-fan_half, +fan_half, i/(n-1))` with `fan_half = 28°` (wider for n≥7).  
   - Lift: peak at `+0.22 * h` at p≈0.3.  
   - Scale: 0.25 → 0.85.  
   - Rotation: inherit row target rot + extra **spin** `rz += (1-p)*π*0.25` (tiles “tumble” into place).

2. **Snap segment** (p = 0.55..1.0):  
   - Position: lerp `M` → final row slot with **overshoot** scale on X: `1 + 0.04 * sin((p-0.55)/0.45 * π)`.  
   - Scale: 0.85 → 1.0.  
   - Spin decays to row euler.

**Row layout**

- Reuse [`compute_pack_reveal_row_layout`](../../src/render/showcase_tile_layout.rs) for final slots (readability invariant).
- During Deal, row slots are the attractor; arc is the spectacle.

**Active tile emphasis**

- While `tile_progress(i)` ∈ (0.05, 0.95): `glow = true`, `glow_color = Some(pack_palette.seal)` for tile `i` only.
- Landed tiles: `brightness = 1.0`; last tile to land gets **0.3 s** extra seal-colored glow pulse.

**Pack**

- After `Unseal`, pack mesh optional: hide when `unseal_t > 0.5` OR keep ghost at 30% scale for depth — **prefer hide** to reduce clutter.

**Lighting**

- **Traveling spot**: center X interpolates from pack cx → row cx over Deal duration; intensity 11.0.
- Point lights: warm key + cool rim, intensities +15% vs TPOS1 Reveal.

**Audio**

- `PackTileReveal` on each tile’s reveal edge (same event as TPOS1), consider **pitch ladder** +2 semitones per index (mirror score cascade) — cap at +10 st.

**Events**

- Same `PackTileRevealed` edge detection as TPOS1.

---

### 5. `Settled`

**Goal:** Identical contract to TPOS1 dismiss.

- All tiles in row, no glow (except optional last-tile pulse completing).
- Copy: “Click or press confirm to continue” (unchanged).
- `fully_settled()` when `elapsed >= total_duration()` (recompute constants for new stagger/fly).
- Input: confirm / click → pop overlay, `pending_shop_focus_snap_after_celebration`.

---

## TPOS1 vs TPOS2 (summary)

| Aspect | TPOS1 | TPOS2 |
|--------|-------|-------|
| Phases | 2 | 5 |
| Intro wipe | None | Shooting-star (pack-tinted) |
| Pre-open motion | Dual-axis bob | Breathe + seal glow + slow yaw |
| Open moment | Instant on confirm | `Unseal` punch (~0.55 s) |
| Tile motion | Linear lerp + ease³ | Bezier arc + snap overshoot + spin |
| Pack during reveal | Stays visible | Fades after unseal |
| Per-tile emphasis | None | Seal-colored glow while flying |
| Stagger / fly | 0.18 / 0.35 s | 0.14 / 0.42 s |
| Prompt verb | “open” | “unseal” |

---

## Pack-kind theming (data-driven, no new art)

All seven kinds already expose [`PackPalette`](../../src/render/pack_palette.rs): `foil`, `seal`, `bg`.

| Kind | TPOS2 accent use |
|------|------------------|
| Honors | Gold foil flash; navy wipe tail |
| Terminals | Copper foil; warm obsidian wipe |
| Flowers | Jade seal glow; emerald wipe |
| Bamboo Grove | Green foil; forest bg burst |
| Coin Cache | Brass foil; umber burst |
| Scroll Library | Parchment foil; ink bg |

No per-pack code branches beyond `for_kind(pack_kind)`.

---

## Integration sketch (for implementation)

**Selection**

```rust
pub enum PackOpeningSequence {
    V1, // current TilePackPresenter + PackCelebration
    V2, // Tpos2Presenter + PackCelebrationV2 (or shared state with phase enum)
}
```

- Debug: `F8` cycle or `mahjuro --pack-opening v2` (mirror `--scene tile_pack_celebration`).
- Production: profile flag or always V2 after bake — **keep V1** until TPOS2 ships.

**Files (proposed)**

| File | Role |
|------|------|
| `src/scenes/showcase/tile_pack_v2.rs` | `Tpos2Presenter` draw/update |
| `src/scenes/shop/pack_celebration_v2.rs` | Phase enum, timing, `tile_progress` |
| `src/scenes/celebration_overlay.rs` | `ShootingStarCelebrationIntro::new_pack_tinted(TilePackKind)` |
| `docs/agents/tpos2-art-direction.md` | This doc |

**Reuse unchanged**

- `shop_celebration_camera`, ray-plane placement hints, `ShowcaseTileBatch`, row layout, event bus events, headless screenshot entry points (add `screenshot_*_v2` presets).

**Tests**

- Port `pack_closeup_projects_into_viewport` for larger box fraction.
- Golden headless: `Arrival` skipped, `Anticipation` pack, `Settled` row.

---

## Timing constants (V2 defaults)

```text
ARRIVAL_WIPE_SECS     = 1.7   # reuse ShootingStarCelebrationIntro
UNSEAL_SECS           = 0.55
DEAL_STAGGER          = 0.14
DEAL_TILE_FLY_SECS    = 0.42
SETTLE_SECS           = 0.25
ARC_SPLIT             = 0.55  # fraction of per-tile p
FAN_HALF_DEG          = 28.0
PACK_BOX_H_FRAC       = 0.62
```

`total_duration` = `(n-1)*DEAL_STAGGER + DEAL_TILE_FLY_SECS + SETTLE_SECS` (same formula as V1).

---

## Screenshot / marketing frames

| Frame | Phase | CLI hint |
|-------|-------|----------|
| Hero pack + seal glow | `Anticipation` | extend headless hold |
| Mid-unseal flash | `Unseal` @ t≈0.25 | freeze `started_at` |
| 3-tile fan in air | `Deal` @ staggered mids | composite |
| Full row settled | `Settled` | existing settled preset pattern |

---

## Out of scope for V1 of TPOS2

- New pack mesh / lid rig (rotation-only illusion).
- Particle systems or ribbon shaders.
- Per-pack unique SFX (palette-only visuals).
- Changing tile count or shop purchase flow.

---

## Acceptance criteria

1. First-time player understands: wait for wipe → unseal → read row → continue (same as today, one extra beat).
2. At 1080p, 8-tile pack (Bamboo/Coin/Scroll) stays inside row layout caps — no overlap.
3. `PackOpened` / `PackTileRevealed` fire at the same semantic edges as TPOS1.
4. Headless screenshots and `--scene tile_pack_celebration` work for both sequences.
5. With `transition_fullscreen_fx` off, TPOS2 still reads clearly (pack-tinted wipe forced on, like zodiac).

---

## Next step

Implement `Tpos2Presenter` behind a debug toggle, tune `FAN_HALF_DEG` and arc peak for 8-tile packs at 1280×720 and 1920×1080, then A/B with TPOS1 in the shop.
