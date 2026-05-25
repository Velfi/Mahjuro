---
name: Mahjuro SFX recording guide
description: Per-cue recording brief — what each placeholder/missing SFX is, when it fires, and what it should sound like
type: production-doc
---

# Mahjuro — SFX Recording Guide

Working reference for re-recording SFX. Source of truth for design intent is [sound-design.md](sound-design.md); source of truth for the cue list is `enum SfxId` in [src/audio.rs](../src/audio.rs). This doc collapses both into a table you can record from.

## Recording targets (apply to every cue)

- **Format**: mono, 48 kHz, source ≥ 24-bit. Deliver as `.ogg` Vorbis q3+ (the bake re-encodes at `-q:a 5`).
- **Peak**: normalize to −1 dBTP.
- **HPF at 60 Hz** on everything except `RoundStart`, `BossEncountered`, `BossDefeated` (the temple-block / gong family is the only audio entitled to sub-100 Hz energy).
- **Reverb**: small-room IR ≤ 0.4 s tail, ≤ −18 dB wet on all hand/table cues — they live in the parlor, dry samples feel pasted in.
- **Saturation**: 5–8 % soft tape on percussive cues to round digital transients.
- **Cascade-tonal cues** (`DoraScored`, every `Yaku*`, `ScoreCrescendo`, `ScoreFinal`, `ZodiacReveal` chime tail) must sit in **A minor pentatonic** so they layer cleanly over `MusicId::Gameplay` and the chromatic `SCORE_TICK_PITCHES` climb.

### Loudness budget by class (short-term LUFS)

| Class | Range |
|---|---|
| Hand / table cues | −18 to −22 |
| UI chrome | −20 to −24 |
| Cascade beats (these *are* the music in their moment) | −15 to −18 |
| Stingers (`RoundStart`, `BossEncountered`, `BossDefeated`, `Victory`) | −12 to −15 |

### Material palette

Porcelain glaze · bamboo · felt / mahjong cloth · paper (thin folded for talisman, thick wrapped for pack) · brass/copper coin · struck idiophone (mokugyo / kane) · tuned percussion (clave, finger cymbal, kalimba tine).

**Out of palette**: modern UI synth woosh kits, riser sweeps, cinematic boomwhackers, voice, distorted guitar, EDM stabs, 8-bit beeps.

---

## M1 — UI chrome

Every Kenney UI sample is temporary. Shared design intent: **one base "focus tick" (a single soft pad press), with one short tail layer per category mixed at −12 dB under the base**, so navigation feels uniform with a "spice" layer that confirms what kind of thing was just focused.

| SFX | Context | Sound |
|---|---|---|
| `UiConfirm` | Every "OK / select / advance" button press across all menus. | Single felt-against-bamboo tick. Brighter than `TileSelect`. ≤ 80 ms. ≤ −20 LUFS short-term. |
| `UiCancel` | Every back / dismiss / close-modal. | Inverse of `UiConfirm`: same instrument, one semitone down, slightly shorter, no brightness boost. |
| `FocusButton` | Keyboard/controller focus moves to an action-bar button (Play, Discard, Sort, etc.). | Base focus tick, no tail. |
| `FocusHandTile` | Focus moves between tiles in the hand row. | Base tick + faint porcelain rim sympathetic-resonance tail (decayed `TilePlace` body, ~200 ms to silence). |
| `FocusConsumable` | Focus moves to a consumable slot. | Base tick + faint paper rustle tail. |
| `FocusRelic` | Focus moves to a relic on the shelf. | Base tick + soft finger-on-glaze sustain tail. |
| `FocusPeg` | Focus moves to a UI peg (hand-size, discards, etc.). | Base tick + tiny wooden click tail. |
| `FocusGold` | Focus moves to the gold readout. | Base tick + very faint coin rim ring tail. |
| `FocusYakuTablet` | Focus moves to a yaku-progress tablet. | Base tick + a single stick-on-wood block tap tail. |
| `FocusDora` | Focus moves to the dora indicator stand. | Base tick + the `DoraScored` chime at −15 dB as the tail. |
| `Pause` | Pause menu opened. | Same instrument as `Unpause`. Single damped tile-tap that "clips the room" — the world goes still. ≤ 200 ms. |
| `Unpause` | Pause menu dismissed (game resumed). | Same instrument as `Pause`. Soft inhale-style swell — the room comes back. ≤ 200 ms. |
| `SliderTick` *(new SfxId)* | Fires every ~5 % of slider drag in the options screen. **Not** on hover, only on value change. | Tiny bamboo-on-bamboo tap, mono, ≤ 30 ms, −24 dB. |
| `SettingsSaved` *(new SfxId)* | Fires once when leaving the options screen if any value changed. | `UiConfirm` variant pitched up 2 semitones. |

---

## M2 — Hand & table (gameplay diegetic)

The "table world." Every cue should feel like one physical object hitting another — porcelain glaze on a felt surface with bamboo edging.

| SFX | Context | Sound |
|---|---|---|
| `TilePlace` | Tile settles into the hand or score reel. | Today: `Snap.ogg`. Keep, but record **3 randomized variants** (`Snap_a/b/c.ogg`) at ±1 dB and ±2 % speed to defeat 200th-repeat fatigue. |
| `TileSelect` | Focus / cursor lands on a hand tile (mouse hover or kbd nav lands on a tile). | Felt thump, very short attack (≤ 5 ms), ~30 ms decay. **One semitone above `TileDeselect`** to give directionality. ≤ −18 LUFS, well under `TilePlace`. |
| `TileDeselect` | Focus / cursor leaves a hand tile. | Same instrument as `TileSelect`, one semitone lower. |
| `StructureCommit` | A meld locks into the mirror tray (set is finalized). | Soft chord-of-2 taps — two `TilePlace`-adjacent samples played 18 ms apart in mono, mixed −3 dB below a single `TilePlace`. The closeness of the two taps reads as "these tiles belong together." |
| `InvalidAction` | Bad meld attempt, full structure, no charges, etc. | Muted, dampened tap — a tile half-rejected by the tray. ≤ 120 ms total. **No descending pitch** — the cue is "stopped," not "wrong." |
| `TilesDestroyed` *(asset missing — silently no-ops today)* | Tiles permanently removed (Taotie, curse effects). | Dry shatter against cloth — a single splintery crack with an immediate damped tail, ~250 ms. **No glass tinkle** — tiles are porcelain *on felt*, not on stone. |

---

## M3 — Scoring cascade (musical)

The only sustained musical sequence in the moment-to-moment loop. Treat it as a small composition. **Every recurring cascade sample must be tonally neutral enough to layer with all three BGM tracks** — that means tuned percussion (clave, finger cymbal, kalimba tine), not pitched melodic instruments.

| SFX | Context | Sound |
|---|---|---|
| `ScoreReveal` | Inhale before the cascade climb begins. | Today: `intake.ogg`. Keep / re-record in same character if replacing. |
| `ScoreStep` | Per reveal beat, layered with `ScoreTick`. | Soft percussive accent that supports the climb without competing with it. Tonally neutral. |
| `ScoreTick` (base sample) | Fires per reveal beat, pitched up the chromatic ladder via `SCORE_TICK_PITCHES` (8 semitones). | The base sample must be a **single short tuned-percussion strike** at the cascade root (clave or finger cymbal). The engine handles pitching it up the ladder. |
| `DoraScored` | Per dora tile, 180 ms staggered (rolling ding-ding-ding). | Single struck-bell tine at a fixed pitch — suggestion: a **perfect fifth above the cascade root**. Recurring decoration; pitch must be in-key with the music or the climb falls apart. |
| `CascadeMerge` | Chips × mult trio snaps together at the start of the hand-off tween. | One continuous gesture *in two parts* with `CascadeLaunch` — pitched-up swirl that signals "the accounting is done." Same instrument as `CascadeLaunch`. |
| `CascadeLaunch` | Merged total leaves the pad and flies toward the score reel. | Second half of the gesture — release whoosh that resolves out of `CascadeMerge`. Same instrument. |
| `CascadeLand` | Merged total lands in the score reel. | Today: `Snap.ogg`. **Keep** — the totals are a tile-sized object landing. |
| `ScoreCrescendo` | Layered on top of `ScoreFinal` for closing weight. | Single two-beat resolution in the cascade's key, paired with `ScoreFinal`. Brassy hit with weight. |
| `ScoreFinal` | Totals lock in. | Companion to `ScoreCrescendo` — falls slightly off-beat, acting as a comma rather than a period. |
| `YakuKokushiMusou` *(asset missing)* | Stinger for the Kokushi Musou yaku (the missing 14th of 14). | Final-tier stinger, recognizably **bigger than `YakuChinitsu`** (current top end) but still fits the 200 ms slot without bleeding into the next yaku. In the cascade key. |

---

## M4 — Round / blind / run lifecycle

| SFX | Context | Sound |
|---|---|---|
| `RoundStart` | A gameplay round begins (curtain rising). | Gong-adjacent struck idiophone, single soft hit, **long resonant decay (~1.2 s)** — this cue is allowed length. |
| `BossEncountered` *(asset missing)* | Boss blind appears at round start. One sample for **all 23 bosses** — per-boss differentiation lives in BGM filter & visual, not unique cues. | Single low wooden temple-block thud (`mokugyo`-style hit) with a soft brief shakuhachi-style breath layer. ~700 ms. |
| `BossDefeated` *(asset missing)* | Boss blind cleared. | Same temple-block instrument as `BossEncountered`, struck **twice (call-and-resolve)** with the second hit dampened. Reuses the instrument so the encounter→defeat arc reads as the same object book-ending the fight. |

---

## M5 — Economy

| SFX | Context | Sound |
|---|---|---|
| `Sell` | Player sells a relic / consumable for gold. | Single coin drop *into* a wooden box (coin sample + soft wooden enclosure tail). **Audibly distinguishable from `CoinDrop`** — Sell is the player *receiving* gold, the timbre should reward. |
| `PackBuy` | Tile pack purchased in the shop (wallet → counter). | Paper-wrapped slap on the counter — one short hand-flesh-on-wood thump with a paper-rustle layer. |
| `PackOpen` | Pack foil wrapper tears open. | Two-stage tear — ~80 ms quick rip + ~200 ms papery unfold tail. **Single source recording, not synthesized.** |
| `PackTileReveal` | Per tile during pack opening (up to 5 in a row). | Dry, soft tile flip — a smaller cousin of `TilePlace`, half its body, more click than thump. **Recurring; must not fatigue.** |
| `ZodiacReveal` | Close-up reveal of a zodiac ribbon. | Silk unfurl + soft chime tail keyed to the cascade's tonality. |
| `TalismanPurchased` *(asset missing)* | Talisman bought from the shop. | Single short brush-stroke on paper + a soft thump as the talisman lands on the dish. Sibling to `PackBuy` but **lighter and drier**. |
| `TalismanUsed` *(asset missing)* | Talisman consumed from the dish. | Quick paper crumple/burn whoosh — the charm is being expended. ~180 ms. **Must read as expiration, not as a new item appearing.** |

---

## M6 — Ambience beds *(new class — designed, not built yet)*

Stereo loops, ≥ 90 s, with non-zero silence to avoid pattern detection. Total bed ≤ −30 dB. Cross-faded with music on scene transition.

| Bed | Context | Sound |
|---|---|---|
| `Ambience::GameplayRoom` | Plays *under* `MusicId::Gameplay` for the full gameplay loop. | Distant temple bell every 60–120 s (randomized), faint wind through paper screens, occasional wood-creak of the room settling. |
| `Ambience::ShopRoom` | Plays *under* `MusicId::Shop` for the shop / pick-blind scenes. | Paper rustle of inventory being touched, *distant* sound of a kettle, occasional wooden floor creak. |

No ambience for the main menu exterior (existing music carries it; a bed risks clashing with the logo sting) or the archive/collection (silence under menu music is appropriate).

---

## Locked — do not re-record without explicit approval

These cues define the identity everything else sits alongside. Listed here so you know to skip them in recording sessions.

**SFX**: `TileDiscard`, `TileClick`, `CashIn`, `CoinDrop`, `Purchase`, `RelicPickup`, `RoundWin`, `Victory`, `Victory2`, `Defeat`, `GameOver`, `LevelUp`, `MainMenuEnter`, `ZodiacLevelUp`, `CandleFlareWhoosh`, `CandleFlareImpact`, `StarShimmer` (revisit only if it clashes with the cascade hand-off), all `Yaku*` except `YakuKokushiMusou`, all 82 `audio/relics/<slug>.ogg` per-relic stingers.

**Music**: `MusicId::MainMenu`, `Gameplay`, `Shop`, `ChamberWin`, `ChamberLoss`, `BossWin`, `BossLoss`.

---

## Quick recording checklist (per cue)

1. Record at the source material listed in the table; capture multiple takes.
2. Trim, de-click, HPF 60 Hz (except gong family).
3. Apply small-room IR if it's a hand/table cue.
4. Light tape saturation on transients.
5. Normalize to −1 dBTP, verify short-term LUFS sits in the budget for its class.
6. Export mono OGG q3 to `assets/audio/<role>.ogg` (path matches `SfxId::filename()` in [src/audio.rs](../src/audio.rs)).
7. Audition in the **SFX Test debug overlay** ([src/debug_overlays.rs](../src/debug_overlays.rs)) — every `SfxId` shows up there automatically via `all_sfx_ids()`.
8. For high-repetition cues (`TilePlace`, `TileDiscard`, `TileSelect`, `UiConfirm`): record 3 variants and name `<role>_a/_b/_c.ogg` for the planned randomization loader (§5.8 of the design doc).
