# Changelog

All notable changes to Mahjuro are listed here. Entries are grouped by
release; the most recent release is on top.

Fragment-based changelog authoring (see `.changes/README.md`) starts with
the next release after v0.3.2. Earlier releases are summarized below from
commit history.

## 0.5.6-9 — 2026-06-05

### Added
- Four new relics reward simple-tile play: **Plain Dealing** and **Even Keel** add chips for simple ranks and middle tiles; **Chow Line** bonuses hands with three or more sequences; **Open Gate** rewards structures built entirely from simple tiles.
- Steam achievements now have custom icons drawn from in-game art — relics, ordeal bosses, season stakes, zodiac tiles, and more — with colorful unlocked and grayscale locked variants ready for upload to Steamworks.

### Changed
- **Tanyao** now scores **+2.0 mult** and **+75 chips** (was +1.5 / +60), making simple-tile hands a bit more rewarding.

## 0.5.6-8 — 2026-06-05

- maintenance, development, and bugfixes.


## 0.5.6-7 — 2026-06-05

- maintenance, development, and bugfixes.


## 0.5.6-6 — 2026-06-04

- maintenance, development, and bugfixes.


## 0.5.6-5 — 2026-06-04

### Changed
- Hold-to-act actions (cash in, shop buy, shop sell) now play a rising windup sound while charging; invalid holds play a soft rejection ping instead.

### Fixed
- Pressing LT or RT during a scoring cascade no longer skips the reveal animation.

## 0.5.6-4 — 2026-06-04

### Fixed
- Win and loss screens now wait until the scoring cascade, score counter rolls, and flying score popups finish before the round resolves, so the full score beat is not cut off by the celebration modal.

## 0.5.6-3 — 2026-06-04

### Fixed
- Zodiac level-up ribbons and meta level-up relic unlock screens no longer cast a drop shadow on the black celebration backdrop.
- Discarding your entire hand at once no longer ends the round early — tiles are redrawn as usual after the discard animation.
- The in-game Guide now turns pages with LB and RB on controller (and Page Up / Page Down on keyboard), matching the footer hint.

## 0.5.6-1 — 2026-06-04

### Changed
- Moving focus with the left stick, d-pad, or arrow keys during a scoring cascade no longer skips the reveal. Confirm and other actions still let you fast-forward as before.

## 0.5.6-0 — 2026-06-04

- maintenance, development, and bugfixes.


## 0.5.5 — 2026-06-04

### Changed
- The tutorial and Lessons practice blind give clearer guidance throughout. The in-run Lessons banner is now a compact panel on the left instead of a full-width strip, and it explains why a **Play** attempt failed when your tile selection isn't a valid meld. Campaign copy was refreshed — **Play** and **Discard** are described as limited resources, the scoring glossary mentions kongs, and page tips no longer overlap the Next/Back buttons. Finale failure messages focus on honor debuffs in plain language, and the start-screen and summary screens describe the flow more accurately (including that finishing or skipping the tutorial is permanent for that profile).

### Fixed
- Carved shop and memorial talismans no longer show jagged mesh spikes along their edges when viewed up close in the storeroom, archive, or inspect orbit.

## 0.5.5-1 — 2026-06-03

### Added
- Options now includes a **Low memory** graphics preset for weaker GPUs: slightly lower internal resolution, shadows and screen-space reflections off, and tighter room loading so 1080p stays playable on cards with about 4 GB of VRAM. On first launch the game may choose it automatically when it detects an integrated or entry-level GPU.

## 0.5.5-0 — 2026-06-03

- maintenance, development, and bugfixes.


## 0.5.4 — 2026-06-03

- maintenance, development, and bugfixes.


## 0.5.4-3 — 2026-06-03

- maintenance, development, and bugfixes.


## 0.5.4-2 — 2026-06-03

- maintenance, development, and bugfixes.


## 0.5.4-1 — 2026-06-03

- maintenance, development, and bugfixes.


## 0.5.4-0 — 2026-06-03

- maintenance, development, and bugfixes.


## 0.5.3 — 2026-06-03

### Fixed
- The shop’s hold-to-sell prompt now draws a smooth circular progress ring around the sell button (Q or West), including on keyboard, and the ring stays visible while you charge the sell.

## 0.5.2 — 2026-06-03

### Changed
- Menus and overlays now show Kenney input glyphs for common actions — navigate, select, back, scroll, page, inspect, and open the guide — using icons that match your connected controller (including Swap A/B and Swap X/Y). Footer hints are slightly smaller, and screens that pin controls to the bottom (Options, Credits, the tutorial flow, Guide, and others) leave room so the row no longer overlaps Back buttons or version text. Celebration prompts, run summary, stairway, wall ledger, and the yaku journal plaque use the same glyph row instead of plain “press confirm” text. During a run, the bottom guide shortcut and table action legends still follow the **Hints** option in Settings; hub and menu footers always show. In collection inspect, **Confirm** previews the focused relic or scored yaku instead of closing the view; the exit hint shows only the North face button (or **E** on keyboard).
- Structure retrigger relics — Geese, XXXL Egg, Last Breath, Voice of the People, and Voice of the Elite — now retrigger whole melds instead of individual tiles. Geese retriggers your first five melds in full, and the Voice relics only retrigger melds whose tiles all fit their rank band.

## 0.5.1 — 2026-06-02

### Fixed
- The production logo no longer plays twice at startup; it runs once across the early load screen and the main loading splash.
- Main-menu rain streaks and splashes now scale down on smaller resolutions, so they no longer look oversized at 720p while staying the same on 1080p and 4K.

## 0.5.0 — 2026-06-01

- maintenance, development, and bugfixes.


## 0.5.0-26 — 2026-06-01

- maintenance, development, and bugfixes.


## 0.5.0-25 — 2026-06-01

- maintenance, development, and bugfixes.


## 0.5.0-24 — 2026-05-31

### Added
- Wall Ledger shows the full round wall grouped by suit — open it from the tile count in the shop or during a run. Undrawn tiles stay vivid; tiles already drawn from the wall appear faded.

## 0.5.0-22 — 2026-05-28

- maintenance, development, and bugfixes.


## 0.5.0-21 — 2026-05-26

- maintenance, development, and bugfixes.


## 0.5.0-20 — 2026-05-25

- maintenance, development, and bugfixes.


## 0.5.0-19 — 2026-05-23

### Fixed
- Yaku detection now matches standard mahjong more closely: Junchan allows honors and stacks with Honroutou, each Yakuhai triplet scores separately, Chinitsu no longer overlaps Honitsu, and Iipeikou only counts on a full winning hand. Full-hand previews pick the meld split that awards the strongest yaku.

## 0.5.0-18 — 2026-05-21

### Changed
- Numbered suits are now **Manzu**, **Souzu**, and **Pinzu** (was Characters, Bamboos, Dots). Talisman ids and art filenames follow the new names; in-progress saves still load via serde aliases.

- maintenance, development, and bugfixes.


## 0.5.0-17 — 2026-05-21

### Added
- Discarded tiles now lift from your hand, arc into the discard river, and rest face-up along the water until your next discard, when the previous pile sinks away before the new tiles land.

### Changed
- The Meld Guide now has a dedicated Flowers page that walks through every legal way a flower can be used: filling a triplet, mending a sequence, pairing with another flower, or forming an all-flower triplet.
- Flowers no longer grant different chips, mult, or gold by face (Plum, Orchid, and the rest). **Garden Keeper** now adds **+25 chips** for each flower tile in a scored hand instead of doubling Hanami payouts.
- You can now invert your current hand selection instantly with `Z` on keyboard or left-stick click (L3) on controller.

## 0.5.0-16 — 2026-05-20

- maintenance, development, and bugfixes.


## 0.5.0-15 — 2026-05-20

### Fixed
- Quick Draw now draws an extra tile after every play for the rest of the blind, as described on the relic.

## 0.5.0-14 — 2026-05-20

### Fixed
- Steam Deck game mode no longer caps the game to ~10 FPS. Two Linux Vulkan swapchain paths were serializing every frame to gamescope's nested compositor: a Windows-only frame-latency clamp that was leaking into Linux builds, and wgpu's post-acquire fence wait (originally added for Windows DXGI pacing) that also blocked on Linux. Both are now Windows-only.

## 0.5.0-13 — 2026-05-19

- maintenance, development, and bugfixes.


## 0.5.0-12 — 2026-05-19

### Fixed
- Steam Deck game mode no longer caps the game to ~10 FPS. The Vulkan swapchain latency clamp was meant only for Windows AMD drivers; on Linux it forced a 2-image swapchain that gamescope's nested compositor could throttle to its own pacing.

## 0.5.0-11 — 2026-05-18

### Added
- Added main menu scene rain and updated shop scene.

## 0.5.0-10 — 2026-05-16

### Removed
- The Phantom Relic is no longer in the relic pool.

## 0.5.0-9 — 2026-05-14

### Added
- Archive cubby zodiac ribbons are now a tunable arrange-mode target (`collection.cubby_zodiac`) so the whole row of ribbons can be re-centred inside their cubbies without editing each cell. Previously the cubby ribbons fell through the renderer's `arrange_name` default and reported as `shop.for_sale.ribbons` — a placement that doesn't exist on the Archive scene, so confirms only landed on the clipboard.
- Optional VHS overlay in Options → Visual ("VHS overlay: ON/OFF"). Adds a subtle creepy-footage look — soft scanlines, vignette, low-amplitude grain, and micro chromatic aberration. Off by default. Tuned to stay accessible: HUD text and tile faces remain legible, no flashing or hue swings, and the journal-page pre-pass deliberately skips the effect so the in-game journal never accumulates artifacts.

### Changed
- Archive Back and Switch save buttons are now reachable from controller and keyboard. Pressing Up from the top cabinet row parks focus on the title-bar chrome (column-nearest of Back / Switch save); Down from the bottom row enters the footer Prev / Next arrows. A / Enter on a focused chrome button activates it, and a brass focus ring (matching the rest of the menu) reads in both controller and cursor modes.
- Added two named theme tokens — **`LAPIS`** (sky-blue, the cool counterpart to `RUBY`) and **`PORCELAIN_AGED`** (well-loved ceramic cream, distinct from `PARCHMENT` paper) — and a `CascadeTokenKind::color()` helper that funnels the score-popup, cascade-HUD label, and 3D cascade-token meshes through a single Chips→`LAPIS` / Mult→`RUBY` mapping. Score-popup constants now resolve to `LAPIS / RUBY / RELIC_GOLD / TALLOW` instead of four drifting literals; the consumable dish, action-prompt legend, and shop legend ceramic surfaces now share `PORCELAIN_AGED`. See `COLOR_THEME.md` and `python3 tools/color_inventory.py` for the audit.
- Color palette renamed from "Midnight Gold" to **"Walnut, Brass & Felt"** to match how the game actually looks. Added felt-green, twilight-indigo, lacquer-black, cinnabar-red, and tallow-cream as named tokens — round-win modals now sit on lit felt with a brass border instead of an ad-hoc olive panel. Full design rationale lives in `COLOR_THEME.md`; run `python3 tools/palette_preview.py` to render the new palette sheet.
- Zodiac silk ribbons now use a single tall portrait texture per zodiac (`zodiac_<slug>.png`) instead of a fragile 3-piece tile set (`_top` / `_mid` / `_bot`). `gpt-image-2` accepts portrait aspects up to 3:1 directly, so the entire ribbon — finial, embroidered animal, and tasselled tip — is generated in one shot. Renderer collapses to one mesh + one bind group per ribbon (3× fewer draw slots used per zodiac), shadow caster does the same, and `scripts/generate_zodiac_ribbons.py` is half the size with no joining-edge prompt hacks. Re-run the script to regenerate art (the 42 old `_top/_mid/_bot.png` files have been removed).

### Fixed
- Archive relic close-up now decals the relic flavor onto a single visible description sign instead of leaving both boards blank while printing the flavor in champagne text under the close-up. Sign side selection runs in inspect mode too (using the eased inspect camera and the existing cursor / focused-slot reference X), so the chosen board sits opposite the player's reference point as in grid mode. Locked relics and non-relic artifacts get the standard `name + body` decal on the same sign instead of the previous floating tooltip panel.
- Tile-pack celebration: pack mesh and reveal-row tiles are visible again. The default `celeb_pack_closeup` placement (and any saved arrange-mode override that mirrored those values) had drifted to a near-top, sub-felt position that pushed the pack out of frame.

## 0.5.0-8 — 2026-05-13

### Changed
- The rare relic is now called **Dragon Rage** instead of Red Dragon Rage. Its effect is unchanged: any dragon triplet or kong still adds extra mult.

## 0.5.0-6 — 2026-05-11

### Added
- The final ante can now pit you against **The House**, which won’t let you cash in your structure until you’ve used every discard for the round.

### Changed
- **Silver Filigree Lantern** is now **Stone Lantern** (ishidōrō stone lantern art direction). Internal id is `stone_lantern`; older saves that still store `silver_filigree_lantern` load correctly.
- The **Meld Guide** no longer appears on the main menu; open it from **Pause** while you’re in a run.
- **Snowball** adds **+15 chips** to every scored hand for each **cleared** blind while you own it (skips don’t count), up to **15** clears counted; selling the relic resets the counter. Chip bonus is flat per clear (not tied to the blind’s score target).

## 0.5.0-5 — 2026-05-09

### Removed
- The Kiln talisman is no longer in the shop or the talisman pool—you can’t permanently remove tiles from your hand that way anymore.

## 0.5.0-4 — 2026-05-09

### Added
- New relic **Chrysalis**: once you've already reached the blind's target score, further scores that round no longer add to your round or run totals—instead that value is absorbed. Gather enough absorbed score and it hatches into **Monarch Butterfly**, which adds bonus chips each hand based on tiers that grow from total absorbed excess (with diminishing tier gains as excess climbs).
- New rare relic **Euler's Number** adds a flat mult bonus equal to the constant *e* (about 2.718) every time you score a hand.
- New relic **I Got A Guy**: three times per run you can restock the shop without paying gold.

### Changed
- Ghost Hand now adds chips equal to the **point value** of hand tiles that are not part of what you scored (your full remaining hand when you cash in the structure bank). The relic shows a live chip preview on the tray and in tooltips.
- Glass Cannon now gives a huge one-time ×4 mult on your next scored hand, then breaks. It no longer costs you a play each round, and it shows up as a cheap common relic in the shop.
- Second Wind no longer gives an extra play each round. The first time you would lose a blind, it destroys itself instead: you forfeit that blind with no coin payout, but the run continues and your other relics can still react as the round ends.
- **Tea Ceremony** now gives a different principle bonus on each of the next four scored hands, then becomes **Rakuware** in the same relic slot. **Rakuware** now grants every one of those principle bonuses on each score when you meet their conditions (instead of scaling with other relics being destroyed).

## 0.5.0-3 — 2026-05-09

### Added
- Added a new Yaku: **Kokushi Musō** (thirteen orphans). It's one of every terminal and honor tile, plus a second copy of any one of those tiles. It scores as its own pattern bonus and does not stack the “all terminals and honors” pattern on top of it.
- You can send full startup logs to a text file using an environment variable, which makes it easier to figure out why the game won’t start from Steam or another launcher when nothing shows on screen.

### Changed
- Tea Ceremony, Silk Thread, and Melting Ice now burn out like Paper Lantern: the slot empties, and their follow-up relic can appear in shops for the rest of that run only. Those follow-ups are no longer unlocked through leveling or kept in your permanent shop pool—you reveal them in the collection the first time you earn them, and each new run you need to burn the fragile relic again before the successor can show up for sale.
- The jade talisman is removed. The pearl talisman now pays a flat chip bonus once per scored meld that includes a stamped tile (including pairs), in the same spirit as the polychrome stamp’s per-meld multiplier.
- Several relics were moved to different rarity tiers so their shop price and sell value line up better with how strong they are.

## 0.5.0-2 — 2026-05-07

### Added
- Options now include Swap X/Y next to Swap A/B. Score, discard, shop hold-to-sell and inspect, and collection inspect use the swapped face buttons when it’s on, and shop controller hint icons stay in sync.

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
- The 3D table path does less redundant work when tiles move or when nothing casts a shadow—especially noticeable on slower machines or in busy scenes.
- Polychrome talismans no longer pick up an extra self-lit glow in the shop, so the token stays readable instead of washing out in bloom.
- Release builds no longer ship the Debug menu, and on Windows the game now launches without a black console window behind it. Both are still present in development builds.
- Reworked the Leading Tile relic into Geese — it now retriggers the first 5 scored tiles in the hand instead of just the first tile of each scored set.
- I redid the art for many relics to make them more mahjong themed, added some relics, and reworked some relics. Stuff that expires should be more fun now.
- Shop counter interaction props (sell tray, restock, leave, journal) are tuned for clearer silhouettes and contrast under the lamp, including a lacquered wood pedestal under the sell tray.
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
- Seasons — four difficulty tiers (Spring, Summer, Autumn, Winter) with progressively higher target scores, shop prices, restock costs, and earlier boss appearances. Winter also disables the sequence scoring bonus. Each tile material has its own unlock ladder: clear a season on any deck to unlock the next season for that deck. The start-game modal picks the season, the blind plaque shows which season you're playing, and the Collection footer lists the highest season cleared per material.
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
- **Ink Brush** is now **Ruby Serpent** (Manzu chip bonus), and **Pearl Diver** is now **Lapis Serpent** (Pinzu chip bonus). Both relics have new art.
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
