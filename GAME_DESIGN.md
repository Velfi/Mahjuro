# **Mahjuro — Game Design**

## **1. Concept Overview**

**Title:** Mahjuro
**Genre:** Roguelite / Tile-Based Strategy / Riichi-Inspired
**Core Idea:**
Mahjuro is a riichi-style mahjong roguelite that makes mahjong approachable for western players. Players form hands, stack multipliers, and break traditional rules while progressing through increasingly chaotic runs. The focus is on **pattern recognition, deck manipulation, and absurd combos**.

---

## **2. Core Gameplay Loop**

1. **Pick a Blind** — Choose to play (or skip for a penalty) the next Small, Big, or Boss blind in the current ante.
2. **Draw Tiles** — Draw from the wall into a hand of limited size.
3. **Form Melds** — Select tiles and play them as:
   * Pair (2 tiles)
   * Sequence / Chow (3 consecutive numbers, same suit)
   * Triplet / Pong (3 of a kind)
   * Kong (4 of a kind)
4. **Score the Hand** — Chips × Mult in the style of Balatro, modified by yaku, relics, tile enhancements, dora, and the active rule modifier.
5. **Discard to Refine** — Sacrifice unwanted tiles from a limited discard budget (4/round by default) to redraw from the wall. Unused discards can still pay out via relics and end-of-round bonuses.
6. **Repeat Plays** — Each round gives a fixed number of plays (4 by default) to meet the blind's score target.
7. **Shop Phase** — Between blinds, spend earned coins on relics, zodiacs, talismans, and rerolls from a 3D curio cabinet.
8. **Advance Antes** — Clear Small → Big → Boss across 8 antes. The final boss at ante 8 ends the run.

---

## **3. Core Mechanics**

### **A. Tile Types**

* **Number Tiles:** 1–9 in 3 suits — Characters, Bamboos, Circles
* **Honor Tiles:** Winds (East, South, West, North), Dragons (Red, Green, White)
* **Flowers:** Act as wildcards — one flower can substitute for a missing tile in a meld (max one substitution per meld); flowers themselves contribute no base chips
* **Seasons:** Solitaire-only bonus tiles

### **B. Tile Enhancements**

Persistent per-tile modifiers applied via talismans in the shop:

* **Jade** — +20 chips
* **Pearl** — +30 chips
* **Gilded** — +0.5 mult
* **Polychrome** — ×1.2 mult per meld containing the tile
* **Kiln** — consumable that permanently destroys a tile from the wall

### **C. Meld Types & Base Scoring**

| Meld     | Tiles              | Base Chips |
| -------- | ------------------ | ---------- |
| Pair     | 2 identical        | 18         |
| Sequence | 3 consecutive      | 28         |
| Triplet  | 3 identical        | 50         |
| Kong     | 4 identical        | 80         |

Kongs additionally flip a new **dora indicator**, and each dora indicator adds +35 chips to matching tiles on subsequent plays.

### **D. Yaku (Hand Patterns)**

Yaku grant flat mult on top of meld chips. Players carry a **yaku loadout** (3 base slots, expandable to 4 via relic) plus two always-active patterns (FullHand, Yakuhai). Fourteen yaku are implemented:

* **Always active:** FullHand (4 melds + pair), Yakuhai (dragon or round-wind triplet)
* **Loadout:** Tanyao (2–8 only), Toitoi (all triplets), Iipeikou (doubled sequence), Sanshoku Doujun (same sequence in 3 suits), Ittsu (1-2-3 + 4-5-6 + 7-8-9), Honitsu (one suit + honors), Chinitsu (one suit only), Junchan (every meld touches a terminal), Honroutou (terminals and honors only), Chiitoitsu (seven pairs), Chicken Hand (zero-mult fallback)

Yaku level up via **zodiacs**: using a zodiac consumable grants +0.5 mult to its bound yaku (capped at +5).

### **E. Scoring System**

`final_score = floor(chips × mult)`

* **Chips** come from rank values (1–9 for numbers, flat 12 for honors) + meld bonuses + enhancements + dora
* **Mult** accumulates from yaku, relics, zodiac levels, and conditional triggers
* Cascading score steps drive the score popup animation

### **F. Relics**

**79 relics** are implemented across six content patches, spanning categories such as:

* **Core scoring** — TripletBoost, SequenceSurge, PairPower, HonorFury
* **Info / tempo** — ShantenShove, WallPeek, KanDrum, DoraCrown, Riichi
* **Flower / suit synergy** — GardenKeeper, Ikebana, JadeSerpent, InkBrush, PearlDiver
* **Conditional scaling** — Momentum, Minimalist, ClosedGate, GoldFurnace, Snowball
* **Play budget** — SecondWind (+1 play), GlassCannon (fewer plays, ×2 mult)
* **Retrigger / echo** — LeadingTile, LowEcho, TeaCeremony, GhostHand
* **Fragile / scaling** — MeltingIce, SilkThread, CleanStreak, Obsession, Bonfire
* **Economy** — GoldIdol, JadeAbacus, NestEgg, Patience
* **Way-of** conditional mult — WayOfPairs, WayOfTriplets, WayOfSequences, WayOfPurity
* **Chaos / sell-to-activate** — FortunesFavor, CrackedTile, StarTile, SmokeBomb, PhantomRelic, RitualBlade

Relics come in Common / Uncommon / Rare tiers that gate shop availability and price.

### **G. Rule Modifiers**

**12 implemented modifiers**, split between neutral round rules and boss taxers:

* **Round rules:** Sequence Wrap (7-8-9, 8-9-1), Pair Double Score, No Sequences, Reduced Plays, Honor Triple Score, No-Sequence Bonus
* **Boss taxers:** Pairs Score Zero (Hermit), Sequences Halved (Forest), Middle Tiles Zero (Drunkard), Must Play Four (Bureaucrat), Require Honor (Dragon final boss), Censor Repeats (Censor)

### **H. Consumables & Slots**

Talismans and zodiacs share a **base inventory of 2 slots**, expandable via Zodiac Pouch (+1) and Lunar Almanac (+1, with duplication). Both are purchased from the shop and applied manually.

---

## **4. Progression Systems**

### **A. In-Run Progression**

* 8 antes, each containing Small (×0.85 target) → Big (×1.35) → Boss (×1.5 + effect)
* Round wind cycles East → South → West → North, affecting Yakuhai eligibility
* Coins earned from blinds fund shop purchases between rounds
* Skipping a blind grants a smaller reward but forfeits its score contribution

### **B. Meta Progression (Between Runs)**

Tracked in `PlayerProgress` with a **7-level unlock system** driven by runs completed:

* Level 1 (0 runs) → level 7 (20+ runs)
* Each level unlocks a gated pool of relics, rule modifiers, yaku, and dora mechanics
* Top 10 high scores tracked per profile
* Achievements grant permanent run upgrades (tutorial clear, first win → Plastic tile material, etc.)

### **C. Knowledge Progression**

* Tutorial overlay highlights hand tiles, play button, discard bowl, and score panel with pulsing cues
* Tutorial recap screen summarizes the first run
* Meld Guide, Tile Literacy, and Collection scenes teach patterns outside a run

---

## **5. Difficulty Scaling**

* **14 boss encounters** (12 regular bosses + the Dragon final boss), each with a unique tax or restriction effect
* Score targets scale per ante and per blind tier
* Rule modifiers stack with boss effects in late antes, forcing unusual play patterns
* The ante-8 Dragon final boss requires honors to score and caps the run

---

## **6. Visual & UI Design**

* **3D render pipeline** (wgpu) — over 30 render modules including particles, fluid simulation, falling bones, flying coins, and bone-rigged meshes
* **Scenes implemented:** Gameplay, Pick Blind, Shop (3D curio cabinet with spotlight hover), Splash, Start Screen, Profile Select, Tutorial Overlay, Tutorial Recap, Solitaire, Collection, Journal, Options, Pause Menu, Game Over, Tile Literacy, Meld Guide
* **Score popups** animate each chip/mult cascade step
* **Level-up carousel** pages through newly unlocked relics, rules, and yaku after each run
* **Tutorial tooltips** pulse on relevant UI anchors and show contextual banners

---

## **7. Accessibility & Approachability**

* Start with core melds and a small yaku pool; advanced patterns unlock via meta progression
* Friendly terminology:
  * Pair → Pair
  * Triplet → Three of a Kind
  * Sequence → Straight
  * Yaku → Hand Bonus
  * Riichi → Lock-In Bet
* Visual teaching through tile highlights, tutorial overlay, and recap screen rather than text walls

---

## **8. Art & Audio Style**

* Clean, modern 3D tile art with multiple materials (Plastic unlock is one example)
* Smooth cascade animations for draws, melds, and scoring
* "Midnight Gold" palette — cool indigo base with gold accents
* Audio cues:
  * *Click* → tile placement
  * *Clack* → completed meld
  * *Whoosh / ding* → scoring or multiplier

---

## **9. Current State (post-MVP)**

The original MVP scope is fully implemented and significantly exceeded:

| MVP target                        | Status      | Current reality                           |
| --------------------------------- | ----------- | ----------------------------------------- |
| Tile draw/discard                 | Done        | Wall refill, discard bowl, river tracking |
| Pair / Triplet / Sequence         | Done        | + Kong, flower wildcards                  |
| Scoring system                    | Done        | Balatro-style chips × mult with cascades  |
| 10–15 relics                      | Exceeded    | **79 relics** across 6 patches            |
| 1–2 rule modifiers                | Exceeded    | **12 modifiers** + 14 boss effects        |
| Score targets                     | Done        | 8 antes × 3 blinds each                   |
| Post-run unlocks                  | Done        | 7-level unlock gating                     |
| *Bonus: Yaku system*              | Implemented | 14 patterns, loadout + zodiac leveling    |
| *Bonus: Talismans & enhancements* | Implemented | 5 enhancement types, persistent per tile  |
| *Bonus: 3D shop*                  | Implemented | Curio cabinet with spotlight hover        |
| *Bonus: Tutorial / onboarding*    | Implemented | Overlay, recap, dedicated learn scenes    |
| *Bonus: VFX / animation*          | Implemented | Fluid sim, particles, bone rigs           |

---

## **10. Longer-Term Features / Expansion Ideas**

* Additional boss encounters and themed antes
* More yaku and advanced patterns (Riichi-style wait detection, Dora bombs)
* Cosmetic tile sets beyond the current material unlocks
* Leaderboards and daily seeds for score attack
* Full Riichi optional mode for advanced players
* Additional relic patches (patches H+) extending the 79-relic pool

---

## **11. Branding & Tone**

* Title: **Mahjuro** — short, memorable, modern
* Tone: playful but strategic, chaotic but understandable
* Tagline ideas:
  * "Break the rules. Build the hands."
  * "Mahjong, reimagined for chaos."
  * "Stack. Score. Shatter."
