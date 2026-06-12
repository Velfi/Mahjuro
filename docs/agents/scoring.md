# Mahjuro scoring

## The loop

1. **Pick tiles** from your hand that form valid melds (nothing left over).
2. **Play** → those melds go into **structure** (saved for later). No round score yet.
3. **Cash In** → everything in structure scores **once**, all at the same time.

---

## Tiles → chips

Each tile adds chips when scored:

- **Number tiles** (1–9 suits) → chips = **rank** (a 5 is +5, a 9 is +9)
- **Winds & dragons** → **+12** each
- **Flowers** → **+0** (they help form melds, but don't add chips)
- **Debuffed tiles** → **+0**

---

## Melds

A meld is a valid group of tiles:

| Meld | Tiles |
|------|-------|
| **Pair** | 2 of a kind |
| **Sequence** | 3 in a row, same suit |
| **Triplet** | 3 of a kind |
| **Kong** | 4 of a kind |

Melds contribute **only through their tile values** — there is no separate per-meld chip bonus.

You can play **one meld or many** in a single play, as long as every selected tile fits into melds.

---

## Structure

**Structure** = all melds you've played this round, sitting ready to score.

- You can **Cash In with just 1 meld** — no yaku required.
- A full winning hand is **4 melds + 1 pair** (14 tiles), or special shapes like **7 pairs** or **Kokushi**.

Playing into structure **does not** add to your round score. Only **Cash In** does.

---

## Yaku → big chip + mult bonuses

**Yaku** are named patterns checked against your **whole structure**. Each one adds **+chips** and **+mult**.

There is **no** universal rule like "yaku = 2+ melds." Each yaku has its own requirement:

- **Yakuhai** — honor triplet/kong (can trigger on a single meld)
- **Toitoi** — 2+ triplets/kongs, no sequences
- **Tanyao** — all tiles 2–8, no terminals/honors (≥3 tiles, one meld)
- **Full Hand** — 4 melds + 1 pair
- **Chiitoitsu** — 7 pairs
- **Honitsu / Chinitsu** — one-suit hands (mutually exclusive: Chinitsu wins when both qualify)
- **Junchan / Honroutou / Chanta** — terminal-family patterns (tiered exclusive: Junchan > Honroutou > Chanta; only the strictest match scores). Riichi-style: Junchan has no honors and needs a sequence; Chanta needs every group (including pair) to touch a terminal/honor, plus at least one honor, one simple, and one sequence
- **Chanta** — every meld and pair has a terminal/honor; requires honor + simple + sequence
- **Iipeikou / Ryanpeikou** — duplicate sequence(s); Iipeikou works on partial structure, Ryanpeikou needs a full hand
- **Sanshoku Doujun / Sanshoku Doukou** — same sequence or same triplet rank in all three suits
- **Pinfu** — four sequences + 2–8 number pair on a full hand
- etc.

If you Cash In with **no yaku** on a complete hand, you get **Chicken Hand** (+0 chips, +0 mult).

Sample base yaku payouts (level 1):

| Yaku | +Mult | +Chips |
|------|-------|--------|
| Tanyao | 2.5 | 90 |
| Toitoi | 2.0 | 70 |
| Yakuhai | 2.0 | 75 |
| Pinfu / Iipeikou | 5.0 | 105 |
| Full Hand | 5.0 | 60 |
| Chiitoitsu | 6.5 | 85 |
| Kokushi | 10.0 | 130 |
| Chanta / Pinfu | 4.0 | 50 |
| Ryanpeikou | 5.5 | 72 |
| Sanshoku Doukou | 6.0 | 80 |

---

## Mult

Mult **starts at 1.0**. Everything stacks **additively**:

```
mult = 1.0 + yaku bonuses + relic bonuses + boss rules + ...
```

So two yaku giving +3 and +5 → mult = **9.0**, not 1×3×5.

---

## Final score

```
chips = tile values + yaku chips + dora + relics + …
mult  = 1.0 + yaku mult + relic mult + boss rules + …
score = floor(chips × mult)
```

**Dora** tiles on the table add **+100 chips** each if they match tiles in your scored melds.

**Gold** (from flowers/relics) is separate — it goes to your run wallet, not the round score.

---

## One-line mental model

```
tile   = +rank chips (honors +12, flowers +0)
meld   = 2–4 tiles grouped (chips come from tiles only)
structure = melds played across plays; cash in to score them all
yaku   = pattern bonus on the whole structure (+chips AND +mult each)
score  = floor( (tiles + yaku + dora + relics) × (1.0 + yaku + relics + boss rules) )
```

**Structure holds your played melds.** **Yaku are optional bonuses** (except boss rules). **Nothing counts toward the round target until you Cash In.**
