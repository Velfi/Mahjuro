# Changelog

All notable changes to Mahjuro are listed here. Entries are grouped by
release; the most recent release is on top.

Fragment-based changelog authoring (see `.changes/README.md`) starts with
the next release after v0.3.2. Earlier releases are summarized below from
commit history.

## 0.4.0 — 2026-04-20

### Added
- New uncommon relic: **Beggar's Cup** — +$1 at round end, plus an extra $1 per boss defeated this run.
- New uncommon relic: **Cosmopolitan** — at round end, +$1 per unique yaku scored this round.
- New rare relic: **Curio Cabinet** — grants +mult equal to the summed sell value of your other relics.
- New uncommon relic: **Heirloom** — +1 mult per blind played this run (skips don't count).
- New uncommon relic: **Kong Collector** — +$5 per kong scored this round, paid at round end.
- New rare relic: **Lotus Bloom** — gains +0.5 mult permanently each time a flower is drawn or scored.
- New common relic: **No Honor But Wealth** — +$1 each time an honor tile is discarded.
- New common relic: **Sweepstakes** — at round start, 25% chance to pay $2, 25% chance to pay $4, otherwise nothing.
- New uncommon relic: **Tourist** — +3 mult per distinct suit among scored tiles (Flower counts as a suit).
- New uncommon relic: **Wall Weaver** — +0.2 mult per tile remaining in the wall beyond 140. Stacks with Overflow.
- The scoring cascade now visibly hands off: the chips/×/mult trio merges into a total, launches, and lands in the score reel with three distinct sounds for merge, launch, and impact.
- Your profile now tracks per-relic activations, per-boss encounters and defeats, per-talisman purchases and uses, per-yaku score counts, and a full run history of victories and defeats.
- Relics now play bespoke trigger sounds when they activate, with the soft chime falling back only for relics that don't have their own cue yet.
- The shop now sits under a procedural mountain-haze backdrop with a flickering lamp — shade glow, point lights, and god-rays pulse together, and the lamp bugs flap their wings and cast moth silhouettes through the beams.
- Each yaku now plays a distinct audio stinger as it scores — Tanyao, Toitoi, Full Hand, Yakuhai, Iipeikou, Sanshoku, Ittsu, Honitsu, Chinitsu, Junchan, Honroutou, Chiitoitsu, and Chicken Hand each have their own cue, so stacked yaku roll out as a sequence.

### Changed
- Bosses are only marked "encountered" in the collection when you choose to fight them, not when you skip them.
- The collection browser has been rebuilt as a 3D cabinet with four tabs (Relics, Yaku, Bosses, Talismans). Focusing a cell tweens the camera and lifts the artifact onto an inspection pedestal. The Bosses tab stretches into an infinite corridor receding to the vanishing point.
- Debuff semantics clarified across bosses: debuffed tiles now score 0 but still count toward the hand (rather than "score for less"). The Drunkard debuffs rank-5 tiles, The Iconoclast debuffs honor tiles, and The Dragon now zero-scores any structure without an honor tile.
- Relic tooltips for Star Tile, Paper Lantern, and Iron Lantern now show their doubled probabilities while Fortune's Favor is owned.
- The unlock ladder has been filled out through levels 2–7 so the relic pool expands gradually with account progression instead of starting mostly unlocked.
- Relics now wear Iron / Copper / Silver / Gold finishes based on rarity, replacing the old Metal / Plastic / Glass / Wax materials.
- **Ink Brush** is now **Red Serpent** (Characters chip bonus), and **Pearl Diver** is now **Blue Serpent** (Circles chip bonus). Both relics have new art.
- **Shanten Shove** reworked: no longer triggers solely at tenpai. After a refill, if your hand holds an "invested partial" (a pair hoping for a third, a triplet hoping for a kong, or two same-suit numbered tiles within 2 ranks), you draw one extra tile from the wall.
- Rounds now open behind the smoke curtain: hand, target, and on-round-start effects (Sweepstakes coin showers, Dora Crown reveals, etc.) animate into view as the curtain clears, instead of popping in pre-faded.
- Smoke settings have been consolidated: the old Smoke / Smoke Detail / Smoke Sim rows are replaced with **Smoke Quality** (Off / Low / Medium / High / Ultra) and **Smoke Amount** (Light / Medium / Heavy). Existing settings default to Quality: High, Amount: Medium.
- Tab / Shift+Tab / PageUp / PageDown / LB / RB now cycle tabs and pages in tabbed scenes like the collection browser. `S` no longer scores the hand — it is now a down-arrow alias.
- The auto-updater no longer installs silently. When a new version is available, a modal prompts "Update Available — install now?" with Enter to install or Esc to skip.
- The yaku journal is now a full pushdown scene: thirteen yaku laid out on an infinite felt with the focused hand floating above the grid. Navigate with keyboard or d-pad; open it from the gameplay or shop journal icon.

### Fixed
- Shrine plaques in blind selection now widen to fit longer boss rule text instead of clipping.
- Selling an item in the shop keeps focus on a neighbor in the same row (or the sell tray if the row empties) instead of dropping focus entirely.
- Shop tile packs are hidden during the pack-opening celebration so they no longer bleed through the closeup.
- Overlays pushed during a scene transition (such as a zodiac celebration triggered on skip) now route to the layer that started the transition, so they aren't clobbered when the fade ends.
- The tutorial's debuff legend now reads "tile scores 0 but still counts toward the hand," matching current behavior instead of the older "scores for less" wording.

### Removed
- Collection no longer has Rules or Zodiacs tabs. Zodiac info now lives on the Yaku tab since the two are one-to-one.
- Right-clicking a relic or consumable during gameplay no longer sells it — selling now lives exclusively in the shop.
- **Zodiac Pouch** has been removed. Zodiac inventory is now fixed at 2 slots.

## 0.3.2 — 2026-04-16

### Fixed
- Smoke effect rendering.
- Dora indicator behavior.

## 0.3.1 — 2026-04-16

### Changed
- UI and balance adjustments.

## 0.3.0 — 2026-04-15

### Changed
- Large refactor: new shop, new relic art, broad visual and architectural cleanup.
- Visual updates for The Veil boss mechanics.
