# **Mahjuro — Game Design**

## **1. Concept Overview**

**Title:** Mahjuro
**Genre:** Roguelite / Tile-Based Strategy / Riichi-Inspired
**Core Idea:**
Mahjuro is a riichi-style mahjong roguelite that makes mahjong approachable for western players. Players form hands, stack multipliers, and break traditional rules while progressing through increasingly chaotic runs. The focus is on **pattern recognition, deck manipulation, and absurd combos**.

---

## **2. Core Gameplay Loop**

1. **Draw Tiles** – Draw tiles from a limited pool (your “deck”).
2. **Form Hands** – Build sets:

   * Pair (2 tiles)
   * Triplet / Pong (3 of a kind)
   * Sequence / Chow (3 consecutive numbers of same suit)
3. **Score Hands** – Hands give points based on type, multipliers, and relic effects.
4. **Select Power-Ups / Modifiers** – Gain relics, tile upgrades, or rule-breaking modifiers.
5. **Repeat / Progress** – Reach a score target, face a “round boss” or modifier challenge.
6. **Post-Run Unlocks** – Unlock new rules, relics, tile types, or starting bonuses.

---

## **3. Core Mechanics**

### **A. Tile Types**

* **Number Tiles:** 1–9 in 3 suits (Characters, Bamboos, Circles)
* **Honor Tiles:** Winds (East, South, West, North), Dragons (Red, Green, White)
* **Special Tiles:** Flowers, Seasons (optional, late-game or relic-driven)

### **B. Hand Patterns**

* **Pairs:** Basic block
* **Triplets (Pong):** Medium scoring
* **Sequences (Chow):** Medium scoring
* **Full Hand:** 4 sets + 1 pair → large bonus
* **Optional Advanced Patterns:** All Triplets, Flush, Mixed Sets

### **C. Relics (Balatro-Style Upgrades)**

* Modify scoring, tile behavior, or rules. Examples:

  * *“Triplets deal double damage”*
  * *“Sequences hit all targets”*
  * *“Pairs count as triplets”*
  * *“Winning without honors doubles score”*
  * *“All bamboo tiles +2 multiplier”*

### **D. Rule Modifiers**

* Dynamic changes to the game each round:

  * “Sequences can wrap (8-9-1)”
  * “Pairs score double”
  * “Must win in 3 turns”
  * “Duplicate tiles allowed”

---

## **4. Progression Systems**

### **A. In-Run Progression**

* Gain relics and tile pool modifications mid-run
* Stack multipliers and rule-breaking effects
* Each round may include random “modifier challenges”

### **B. Meta Progression (Between Runs)**

* Unlock new rules gradually:

  * Start: basic sets and pair
  * Mid-game: simple yaku (All Simples, All Triplets)
  * Late-game: Riichi mechanics, Dora, Honors modifiers
* Unlock new relics, tile types, and starting loadouts
* Light permanent upgrades: +HP, starting relics, improved tile odds

### **C. Knowledge Progression**

* Players learn probability and pattern recognition naturally
* End-of-run summary highlights:

  * “Most favored patterns”
  * “Missed hands”
  * “Potential multipliers ignored”

---

## **5. Difficulty Scaling**

* Enemies or round challenges escalate not just by numbers but by mechanics:

  * Tile theft, locked tiles, random forced discards
  * Round time limits
  * Modifier stacks in late runs

* Boss rounds (“Blind Challenges”) introduce absurd rules for risk/reward

---

## **6. Visual & UI Design**

* **Tile Drag & Drop:** Smooth snapping, satisfying interactions
* **Hand Hints:** Highlight potential sets and near-complete hands
* **Multiplier Feedback:** Visual flares when stacking relics or modifiers
* **Score Pop-ups:** Immediate feedback when hands complete
* **Rule Notifications:** Clearly show temporary rule changes each round

---

## **7. Accessibility & Approachability**

* Start with **core sets only** – remove full yaku and complicated rules
* Introduce complexity gradually via unlocks
* Friendly terminology for beginners:

  * Pair → Pair
  * Triplet → Three of a Kind
  * Sequence → Straight
  * Yaku → Hand Bonus
  * Riichi → Lock-In Bet
* Use **visual teaching** instead of text-heavy tutorials

---

## **8. Art & Audio Style**

* Clean, modern tile art
* Smooth animations for draws, sets, and multipliers
* Audio cues:

  * *Click* → tile placement
  * *Clack* → completed set
  * *Whoosh / ding* → scoring or multiplier

---

## **9. MVP Scope**

Focus on first playable prototype:

* Tile draw/discard
* Pair, Triplet, Sequence detection
* Simple scoring system
* 10–15 relics
* 1–2 round modifiers
* Score targets
* Post-run unlocks (optional early progression)

---

## **10. Longer-Term Features / Expansion Ideas**

* Boss battles with forced modifiers
* Special tile events (flowers, seasons)
* Multiplayer / leaderboards (score attack)
* Cosmetic tile sets
* Achievements / rare relics for high-score runs
* Full Riichi optional mode for advanced players

---

## **11. Branding & Tone**

* Title: **Mahjuro** – short, memorable, modern
* Tone: playful but strategic, chaotic but understandable
* Tagline ideas:

  * “Break the rules. Build the hands.”
  * “Mahjong, reimagined for chaos.”
  * “Stack. Score. Shatter.”
