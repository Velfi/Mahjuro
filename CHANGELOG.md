# Changelog

All notable changes to Mahjuro are listed here. Entries are grouped by
release; the most recent release is on top.

Fragment-based changelog authoring (see `.changes/README.md`) starts with
the next release after v0.3.2. Earlier releases are summarized below from
commit history.

## 0.5.0-1 — 2026-05-06

### Changed
- Windowing and input now run on SDL3 instead of the previous winit-based stack, with gamepads handled through SDL3’s controller APIs. On macOS, SDL hints are set so Xbox-style controllers are picked up more reliably.

## 0.5.0-0 — 2026-05-06

### Added
- Under **Options → Accessibility**, you can turn on **Discard undo** (off by default). With it enabled, after you discard and your hand refills, an **Undo** button appears just under the discard river’s lower-left corner; it restores that discard until you play, sort, use a consumable, or discard again.
- A new set of "painted from scratch" tiles has been added to the game. Credit to Mari Starkiller.
- The shop shows on-screen gamepad button prompts (Kenney Input Prompts) for actions like inspecting items and hold-to-sell, with glyphs that match Xbox, PlayStation, or Nintendo-style controllers when the game can tell which you’re using.

### Changed
- Hand tile hover and selection rim rendering is lighter on the GPU, so the game should stay smoother when you move the cursor over the rack or focus tiles.
- Lotus Bloom's tooltip now shows how many flowers have been drawn or scored and the permanent mult you get from them.
- Mirror Tile’s tooltip now names the relic it copies from inventory order, notes when another Mirror Tile is the one scoring cares about, and calls out when that copy fully applies to hand scoring (chips and mult). Shadow Hand’s tooltip does the same for the leftmost copied relic.
- Taotie's description now spells out that scored honor tiles are destroyed at cash-in and that each one permanently adds chips to the relic, using the same timing and keyword wording as other relics.

## 0.4.5 — 2026-04-29

### Added
- New control option "X and Y Quick Action". When off, pressing X or Y on the controller only moves focus onto the Play or Discard button — you must press A to commit. Defaults to on (the existing instant-commit behavior).

### Changed
- "Destroyed" is now a styled keyword in tooltips, highlighted in crimson and explained by a Glossary entry that ties together the relic, tile, and transformation cases. Paper Lantern, Silver Filigree Lantern, Silk Thread, and Melting Ice descriptions were rephrased to use the keyword consistently — Silk Thread and Melting Ice now say "Transforms at 0" so it's clear that becoming Silk Moth or Taotie isn't a destruction (Kintsugi will not fire on it).
- Bright spots and warm lighting no longer blow out to flat white on standard displays while looking softer on HDR—the game now maps scene lighting through a single tone curve so SDR and HDR stay in the same ballpark.
- When the Hex boss disables one of your relics for the round, that relic now shows the same debuff mark as debuffed tiles so you can see which one is shut off at a glance.
- Green felt shell detail now scales more gently with the Effects quality setting, and the 3D table path does less redundant work when tiles move or when nothing casts a shadow—especially noticeable on slower machines or in busy scenes.
- Polychrome talismans no longer pick up an extra self-lit glow in the shop, so the token stays readable instead of washing out in bloom.
- Release builds no longer ship the Debug menu, and on Windows the game now launches without a black console window behind it. Both are still present in development builds.
- Reworked the Leading Tile relic into Geese — it now retriggers the first 5 scored tiles in the hand instead of just the first tile of each scored set.
- I redid the art for many relics to make them more mahjong themed, added some relics, and reworked some relics. Stuff that expires should be more fun now.
- Shop counter interaction props (sell tray, reroll, leave, journal) are tuned for clearer silhouettes and contrast under the lamp, including a lacquered wood pedestal under the sell tray.
- When launched through Steam, Mahjuro no longer runs its built-in updater — Steam handles updates instead, so the two won't fight over replacing the game. Non-Steam builds keep updating themselves like before.

### Fixed
- Gamepad and controller navigation now stay responsive on every screen; inputs no longer sometimes pile up and dump when you move the mouse.
- Using a gamepad no longer randomly stops handling navigation when the mouse pointer shifts slightly; moving from controller to mouse now expects an actual click.
- The debug-menu "Set Player Level" shortcut now triggers the level-up celebration modal for any newly-unlocked relics and rules, matching what happens after a normal run.
- Fixed a crash that could happen when opening the in-game journal from the shop.
- The red mult preview chips next to the trigger button no longer jitter around between frames while the cascade is animating.
- The relic on a "New Relic" celebration card now appears in its slot above the relic name instead of floating off behind the card.
- Shop's mountain haze no longer renders as visible blocky grid cells on Windows; the fog now drifts as a smooth wash on every platform.
- Kintsugi, Ant Trail, and Brocade Pouch can now appear in runs — level 7 unlocks every remaining relic, so nothing in the collection stays permanently out of reach.

## 0.4.4 — 2026-04-27

### Changed
- macOS auto-update now uses Sparkle, the standard macOS update framework. Previously, updates failed on macOS because Gatekeeper blocks any app from rewriting its own bundle in `/Applications`. Sparkle handles the download and bundle swap externally, so updates install cleanly without manual drag-replace. Linux and Windows continue to use the in-game updater.

## 0.4.3 — 2026-04-27

### Changed
- Polychrome Talisman now applies ×1.2 mult per meld (up from ×1.15) and costs 12 gold (down from 16). It was priced the highest of any talisman but practically weakest — the per-meld multiplier scales with the rest of your mult stack, which starts small and only pays off on wide multi-meld plays, so the old sticker price rarely justified the buy.

### Fixed
- Boss-rule ofuda cards in blind selection no longer flicker or get holes punched through them when they hang beside the boss plaque.
- Mirror Tile now correctly doubles Dora Crown, Tenpai Talisman, and Garden Keeper. Previously these three relics quietly did nothing when copied — their bonuses appeared in the score breakdown as part of the base line they modified, so Mirror Tile had nothing distinct to duplicate.

## 0.4.2 — 2026-04-23

### Added
- New **American Spring** tile set — a bright, airy alternate styling for the mahjong tiles.

### Changed
- The Blight boss's reveal tooltip now reads "X tiles are debuffed" instead of restating what "debuffed" means — the tutorial and debuff legend already cover that.

### Fixed
- Releasing the gamepad A or B button no longer fires a spurious Cancel. Previously, letting go of B (or A with swap-AB) emitted a Cancel action on release, which could back out of menus unintentionally. Only the confirm-side button's release now emits anything, and only as the ConfirmRelease paired with its press.

### Removed
- **Lunar Almanac** has been removed. Consumable slot expansion now lives solely on the Brocade Pouch.

## 0.4.1 — 2026-04-22

### Added
- New Ant Trail relic: sequences may wrap around the ends, letting hands like 9-1-2 or 8-9-1 count as valid runs.
- New Brocade Pouch relic: buff talismans (Jade, Pearl, Gilded, Polychrome) mark every tile you draw for the rest of the run, not just the 14 in hand when you use them. Also grants +1 consumable slot.
- New Kintsugi relic: each time another relic is destroyed, gain a permanent +1 mult. Kintsugi rewards fragile builds.
- Stakes — four difficulty tiers (Spring, Summer, Autumn, Winter) with progressively higher target scores, shop prices, reroll costs, and earlier boss appearances. Winter also disables the sequence scoring bonus. Each tile material has its own unlock ladder: clear a stake on any deck to unlock the next stake for that deck. The start-game modal picks the stake, the blind plaque shows which stake you're playing, and the Collection footer lists the highest stake cleared per material.
- New Tortoise Shell tile material: honey-amber blonde bekko with dark mahogany mottling, a warm backlit rim at grazing angles, and unique blotch patterns per tile. Grants +$10 starting gold.
- Tutorial now celebrates your first Yakuhai (dragon/wind triplet bonus) and your first boss defeat with fireworks, matching the existing first-pair/triplet/sequence celebrations.
- Victory and defeat screens now play a short stinger about a second after they appear, on top of the existing transition sound.

### Changed
- Gilded Talisman now awards $1 per tile scored in a meld instead of +0.4 mult. Pairs still don't trigger it.
- Completion hints (the green spotlight and glow on tiles that would complete a meld) are now off by default. Enable them in Options → Controls → Hints.
- Relics have refreshed assets
- Buying a relic in the shop or selecting one in the collection now plays that relic's stinger.
- The Yaku Journal on the shop counter is now a wood tablet labeled "Journal," matching the journal button in the gameplay action bar. Clicking or selecting it still opens the journal.
- The startup splash is now a plain black screen with a simple "loading..." indicator.
- Stacked yaku stingers now roll out one after another on a hand commit instead of stacking on the same beat, so each yaku in a multi-yaku hand is audible.
- Tutorial no longer fires a separate fireworks modal for your first Trigger on top of the first-pair celebration — a single fireworks modal plays for the meld instead of two back-to-back.
- Yaku journal redesigned: sealed yaku now sit on warm lacquer cards with a stacked wax seal, the plaque's header leads with the yaku name and a brass level pill, stat totals read as a single right-aligned strip, and the control hint lives on a brass footer along the plaque's bottom edge.
- Yaku stay discovered across runs — once a yaku has scored in any round, its card in the journal unlocks forever instead of relocking when the run ends.

### Fixed
- Wild Winds no longer scores the same hand differently between runs. Wind tile substitutions now pick a stable face assignment, so identical plays produce identical scores.

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
