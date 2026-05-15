# **Mahjuro — Game Design**

## **1. Concept Overview**

**Title:** Mahjuro
**Genre:** Roguelite / Tile-Based Strategy / Mahjong-Inspired
**Core Idea:**
Mahjuro is a mahjong-inspired roguelite that makes mahjong approachable for western players. Players form hands, stack multipliers, and break traditional rules while progressing through increasingly chaotic runs. The focus is on **pattern recognition, deck manipulation, and absurd combos**.

---

## **2. Core Gameplay Loop**

1. **Pick a Blind** — Choose to play (or skip for a penalty) the next Small, Big, or Boss blind in the current ante.
2. **Draw Tiles** — Draw from the wall into a hand of limited size.
3. **Form Melds** — Select tiles and play them as:
   * Pair (2 tiles)
   * Sequence / Chow (3 consecutive numbers, same suit)
   * Triplet / Pong (3 of a kind)
   * Kong (4 of a kind)
4. **Score the Hand** — Chips × Mult in the style of Balatro, modified by yaku, relics, tile enhancements, dora, and the active rule modifier. Valid plays **commit** melds until you **cash in** for that scoring cascade.
5. **Discard to Refine** — Sacrifice unwanted tiles from a limited discard budget (4/round by default) to redraw from the wall. Unused discards can still pay out via relics and end-of-round bonuses.
6. **Repeat Plays** — Each round gives a fixed number of plays and discards to meet the blind's score target. The default **Bamboo** tileset starts with **5 plays** and **4 discards** (material bonuses and meta upgrades can change this).
7. **Shop Phase** — Between blinds, spend earned coins on relics, zodiacs, talismans, packs, and rerolls from a 3D curio cabinet.
8. **Advance Antes** — Clear **Small → Big → Boss** on each ante from the first through the **final ante**; beating the boss on the final ante wins the run (`FINAL_ANTE` in `run.rs` is the single source of truth for how many antes that is).

---

## **3. Core Mechanics**

### **A. Tile Types**

* **Number Tiles:** 1–9 in 3 suits — Characters, Bamboos, Dots
* **Honor Tiles:** Winds (East, South, West, North), Dragons (Red, Green, White)
* **Flowers:** Act as wildcards — one flower can substitute for a missing tile in a meld (max one substitution per meld); flowers themselves contribute no base chips
* **Seasons:** Solitaire-only bonus tiles

### **B. Tile Enhancements & Talismans**

**Enhancements** (persistent on tiles while they stay in hand / until played) come from shop **talismans**—many talismans buff the **whole hand** at once:

* **Jade** — +20 chips
* **Pearl** — +30 chips
* **Gilded** — +0.5 mult
* **Polychrome** — ×1.2 mult per meld containing the tile

Additional talismans **transform** selected tiles (suit shifts, honors, flowers, conformity, etc.).

### **C. Meld Types & Base Scoring**

| Meld     | Tiles              | Base Chips |
| -------- | ------------------ | ---------- |
| Pair     | 2 identical        | 18         |
| Sequence | 3 consecutive      | 28         |
| Triplet  | 3 identical        | 50         |
| Kong     | 4 identical        | 80         |

**Dora** unlocks via meta progression: each round has indicator face(s) on the plinth; tiles that match score extra chips. **Dora Crown** adds another indicator and raises the payout.

### **D. Yaku (Hand Patterns)**

Many **yaku** are implemented (exact list and bonuses live in `assets/data/yaku.json`). They add chips and/or mult on top of meld scoring. Patterns include:

* **Structure / value:** FullHand (4 melds + pair), Yakuhai (dragon or round-wind triplet), Chicken Hand (valid hand with no other yaku)
* **Suit / composition:** Tanyao (2–8 only), Toitoi (all triplets/kongs), Iipeikou (doubled sequence), Sanshoku Doujun (same sequence in three suits), Ittsu (1–9 straight one suit), Honitsu (one number suit + honors), Chinitsu (one number suit), Junchan (every meld touches a terminal), Honroutou (terminals and honors only), Chiitoitsu (seven pairs)

After meta progression is applied to a run, the **full yaku list** is eligible for scoring (level-up tables still *mention* patterns alongside relic/rule/dora unlocks for pacing copy).

Yaku level up via **zodiacs**: each use on a card boosts its bound yaku by `+20 chips` and `+0.5 mult` per level above 1.

### **E. Scoring System**

`final_score = floor(chips × mult)`

* **Chips** come from rank values (1–9 for numbers, flat 12 for honors) + meld bonuses + enhancements + dora
* **Mult** accumulates from yaku, relics, zodiac levels, and conditional triggers
* Cascading score steps drive the score popup animation

### **F. Relics**

A large **relic** pool is implemented (see `assets/data/relics.json`), spanning categories such as:

* **Meld-type & dragon base** — TripletBoost, SequenceSurge, PairPower, HonorFury, DragonRage, GreenLuck, WhiteDragonsHush, JokerTile, StrengthInNumbers, QuickDraw, ChainReaction, MultiplierMaster, SetMagnet, WildWinds, DragonEcho
* **Wall & scoring infrastructure** — ShantenShove, KanDrum, KongsBlessing, DoraCrown, RoundCompass, EightTreasures
* **Flowers** — GardenKeeper, Ikebana, Hanami
* **Suit, rank, terminal, shop** — JadeSerpent, RubySerpent, LapisSerpent, LowTide, HighTide, MerchantsEye, EdgeRunner, LuckySeven
* **Plays per round & conditional mult** — Momentum, Minimalist, TurtleShell, ClosedGate, GoldenEngine, SecondWind, GlassCannon
* **Plays per round & scaling chips** — Snowball
* **Interest & passive gold** — GoldIdol, JadeAbacus, NestEgg, Patience
* **Retrigger, polish, lanterns, mirror** — LastBreath, TilePolisher, PaperLantern, SilverFiligreeLantern, MirrorTile, Geese, VoiceOfThePeople, VoiceOfTheElite, TeaCeremony, GhostHand
* **Ramp, fragile evolutions, inventory tricks** — Humility, Obsession, Bonfire, RiverRunner, MeltingIce, Taotie, SilkThread, SilkMoth, ShadowHand, SolitarySage, Disgust
* **Hand-shape mult** — WayOfPairs, WayOfTriplets, WayOfSequences, WayOfPurity
* **Chance mult & sell effects** — FortunesFavor, CrackedTile, StarTile, SmokeBomb, PhantomRelic, HungryGhost
* **Run-wide payouts & rule bends** — CurioCabinet, LotusBloom, WallWeaver, KongCollector, NoHonorButWealth, Sweepstakes, BeggarsCup, Cosmopolitan, Heirloom, Tourist, Kintsugi, AntTrail, BrocadePouch

Relics use **Common / Uncommon / Rare / Legendary** tiers (see `relics.json`) for shop presentation and pricing.

### **G. Rule Modifiers**

Multiple **`RuleModifier`** variants — round rules and boss-pushed scoring/validation hooks (see the rules module and boss defs for the live set):

* **Round rules:** Sequence Wrap (8-9-1 and 9-1-2 style wraps), Pair Double Score, No Sequences, Reduced Plays (3 plays for the round), Honor Triple Score, No-Sequence Bonus
* **Boss validation/scoring:** Pairs Score Zero (Hermit), Sequences Halved (Forest), **Must Play Five** (Bureaucrat — selection must be exactly five tiles), Require Honor (Dragon final boss), Censor Repeats (Censor)

Other boss effects use **tile debuffs**, relic taxes, gold-per-play hooks, etc.—for example **Drunkard** debuffs **rank-5** tiles so they do not contribute when scored (not expressed as a round `RuleModifier`).

### **H. Consumables & Slots**

Talismans and zodiacs share one **consumable inventory** (default **2 slots**). **Brocade Pouch** adds **+1 slot** and changes how buff talismans apply to drawn tiles. Items are bought in the shop and used manually from the gameplay dish.

---

## **4. Progression Systems**

### **A. In-Run Progression**

* The run advances through a ladder of **antes**, each with **Small → Big → Boss** in order; boss encounters pull from themed pools, and the **final ante** reserves **Dragon**.
* **Score targets** scale as **`base_target × run_number`** (`run_number` rises after every blind **cleared** or **skipped**). `base_target` is set at run creation from **stake** (e.g. Spring defaults to **500** before material/stake tweaks). Boss blinds add pressure via **boss hooks**, not a separate target multiplier in code.
* Round wind cycles East → South → West → North by ante, affecting Yakuhai eligibility
* Coins earned from blinds fund shop purchases between rounds
* Skipping a blind advances `run_number` and yields a smaller payout—you **do not** fight that blind for score or rewards

### **B. Meta Progression (Between Runs)**

Tracked in `PlayerProgress` with a **tiered unlock ladder** driven by **runs completed** (thresholds live in progression code / data):

* Each tier unlocks **relic shop pools**, **round rules** available in shops/runs, and **dora** at the configured tier. Level-up tables still reference **yaku names** for messaging; scoring uses the full yaku list once progression is applied.
* A capped **high-score list** per profile; **run history** records finished runs for analytics and stake unlocks.
* **Stake** ladder (**Spring → Summer → Autumn → Winter**) raises targets, shop prices, reroll base cost, and boss floors; **Winter** also adds the **No-Sequence Bonus** rule every round. Higher stakes unlock per **tile material** after clearing the previous stake with that material.
* First **victory** unlocks **Plastic** tiles (+1 starting discard); **Tortoise Shell** grants bonus starting gold (material choice at run start).

### **C. Knowledge Progression**

* **Tutorial overlay** (inside gameplay) highlights hand tiles, actions, and HUD with pulsing cues
* **Tutorial Campaign**, **Tutorial Summary**, and **Tutorial Recap** scenes support onboarding
* **Meld Guide**, **Tile Literacy**, **Yaku Journal**, **Collection**, **Material Viewer**, and **Solitaire** support reference play outside a standard run
* **Zodiac Celebration** marks level-ups on zodiac cards

---

## **5. Difficulty Scaling**

* Many distinct **boss kinds** in code, grouped into soft / medium / hard / reactive pools, with **Dragon** on the final ante (full roster in `assets/data/bosses.json` + `BossKind` in `boss.rs`)—effects mix `RuleModifier` pushes, **tile debuffs**, gold taxes, hand-size tweaks, and reactive “pick at reveal” variants (**Mirror**, **Counterweight**, **Tax Collector**).
* Score targets rise with **run_number** (every blind faced or skipped) and **stake** (`base_target`); ante progression swaps boss pools and round wind
* Round rules and boss hooks stack in later antes
* The **Dragon** final boss pushes **Require Honor** (structure must include an honor tile)

---

## **6. Visual & UI Design**

* **3D render pipeline** (wgpu) — many top-level `render/` modules (meshes, particles, fluid, flying coins, cabinet/shrine props, `wgpu_renderer` umbrella, etc.)
* **First-class scenes** (`Scene` enum): Splash, Start Screen, **Tile Select** (material / start modal), Profile Select, Shop, Pick Blind, Gameplay, Game Over, Meld Guide, Material Viewer, Options, Collection, Solitaire, Tutorial Recap, **Tutorial Campaign**, **Tutorial Summary**, Tile Literacy, **Transition Playground** (dev), **Yaku Journal**, **Zodiac Celebration**
* **Pause menu** is embedded in run-adjacent scenes (gameplay, shop, pick blind)—not a separate `Scene` variant
* **Score popups** animate each chip/mult cascade step
* **Level-up modal** celebrates newly unlocked **relics** and **rules** (and level-ups can flag **dora** / yaku-related milestones in progression data)
* **Tutorial overlay** is owned by the gameplay scene when a lesson is active

---

## **7. Accessibility & Approachability**

* Onboarding and stakes ease players in; the full **yaku** list is scoring-eligible in normal runs once progression is applied—difficulty comes from blinds, bosses, and relic/rule chaos rather than hiding patterns mid-run
* Friendly terminology:
  * Pair → Pair
  * Triplet → Three of a Kind
  * Sequence → Straight
  * Yaku → Hand Bonus
* Visual teaching through tile highlights, tutorial overlay, and recap screen rather than text walls

---

## **8. Art & Audio Style**

* Clean, modern 3D tile art with multiple materials (Plastic unlock is one example)
* Smooth cascade animations for draws, melds, and scoring
* "Walnut, Brass & Felt" palette — warm walnut base with sparing brass accents (see `COLOR_THEME.md`)
* Audio cues:
  * *Click* → tile placement
  * *Clack* → completed meld
  * *Whoosh / ding* → scoring or multiplier

---

## **9. Current State (post-MVP)**

The original MVP scope is fully implemented and significantly exceeded:

| MVP target                        | Status      | Current reality                           |
| --------------------------------- | ----------- | ----------------------------------------- |
| Tile draw/discard                 | Done        | Wall refill, discard bowl, river mesh     |
| Pair / Triplet / Sequence         | Done        | + Kong, flower wildcards, structure bank   |
| Scoring system                    | Done        | Balatro-style chips × mult with cascades  |
| 10–15 relics                      | Exceeded    | Large pool — `assets/data/relics.json`    |
| 1–2 rule modifiers                | Exceeded    | Broad `RuleModifier` set + rich boss hooks |
| Score targets                     | Done        | Full ante ladder (**Small / Big / Boss** per ante); **Spring–Winter stakes** |
| Post-run unlocks                  | Done        | Tiered relic/rule/**dora** gating + stakes |
| *Bonus: Yaku system*              | Implemented | Full list in `yaku.json` + zodiac leveling |
| *Bonus: Talismans & enhancements* | Implemented | Hand-wide buffs, transforms, …            |
| *Bonus: 3D shop*                  | Implemented | Curio cabinet, spotlight hover, dishes    |
| *Bonus: Tutorial / onboarding*    | Implemented | Overlay + campaign/summary/recap scenes   |
| *Bonus: VFX / animation*          | Implemented | Fluid, particles, props, transitions    |

---

## **10. Longer-Term Features / Expansion Ideas**

* More bosses / antes / themed stakes
* More yaku and depth (waits, richer dora variants)
* Cosmetic tile sets beyond Bamboo / Plastic / Tortoise Shell
* Leaderboards and daily seeds for score attack
* Further relic expansions

---

## **11. Branding & Tone**

* Title: **Mahjuro** — short, memorable, modern
* Tone: playful but strategic, chaotic but understandable
* Tagline ideas:
  * "Break the rules. Build the hands."
  * "Mahjong, reimagined for chaos."
  * "Stack. Score. Shatter."
