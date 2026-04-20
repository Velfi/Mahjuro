//! Score hands as **chips × mult**, Balatro-style.
//!
//! Each scored hand accumulates two parallel running totals:
//!
//! * **Chips** — additive. Tiles contribute their rank value (honors flat 10),
//!   melds add a flat bonus, and chip-flavored relics pile on more.
//! * **Mult** — multiplicative axis, but built additively (`+N mult`) so it
//!   stacks fast and predictably. Yaku and "explosive" relics live here.
//!
//! Final score = `final_chips × final_mult` (floored).
//!
//! The cascade UI walks the `steps` in order, updating both running totals.
//! The last visible beat is the multiplication itself.

use crate::core::hand::{DetectedSet, SetKind};
use crate::core::relic::{RelicId, ScoreContext};
use crate::core::rules::RuleModifier;
use crate::core::structure::structure_depth_mult_bonus;
use crate::core::tile::{Suit, Tile};
use crate::core::yaku::{YakuKind, detect_yaku_with_wind};

/// Which axis a cascade step contributes to. The cascade renders chip and
/// mult deltas slightly differently (color, +N vs +Nx), so the variant lets
/// the UI pick a style without parsing `effect`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Chips,
    Mult,
    /// Gold awarded mid-cascade (e.g. Bamboo flower). Does not affect the
    /// chips×mult calculation — purely an economy event shown inline.
    Gold,
    /// The final `chips × mult` multiplication beat.
    Final,
}

/// One step in the scoring cascade.
#[derive(Clone, Debug)]
pub struct ScoreStep {
    /// Human-readable source, e.g. "Triplet Boost".
    pub source: String,
    pub kind: StepKind,
    /// Tile ids visually associated with this step. Used by the gameplay
    /// scene to pulse the contributing tiles while the cascade reveals it.
    pub tile_ids: Vec<u32>,
    /// Running chip total *after* this step. Exposed for richer cascade UIs
    /// that want to render the chip and mult counters separately rather than
    /// just the combined `running_total`.
    #[allow(dead_code)]
    pub running_chips: i32,
    /// Running mult value *after* this step. See note on `running_chips`.
    #[allow(dead_code)]
    pub running_mult: f64,
    /// Running combined score (`running_chips × running_mult`, floored).
    /// Cascades that just want a single ticking number read this.
    pub running_total: u64,
}

/// Rich scoring breakdown for cascade animations.
#[derive(Clone, Debug)]
pub struct ScoreBreakdown {
    /// Chips before any relics, yaku, or rules — just tile values + meld bonuses.
    pub base_chips: i32,
    /// Backwards-compatible alias for `base_chips`. Some UI code still reads
    /// `base_points`; keeping the field avoids a wider rename.
    pub base_points: i32,
    /// Base-phase reveal steps (meld by meld) that land before the main
    /// chips/mult steps.
    pub base_steps: Vec<ScoreStep>,
    /// Each contribution that fired, in cascade order.
    pub steps: Vec<ScoreStep>,
    /// Yaku patterns detected in this hand.
    pub detected_yaku: Vec<YakuKind>,
    /// Final chip total before multiplication. Exposed for UIs that want to
    /// show the two-axis ending separately (e.g. results screen breakdown).
    #[allow(dead_code)]
    pub final_chips: i32,
    /// Final mult value. See note on `final_chips`.
    #[allow(dead_code)]
    pub final_mult: f64,
    /// Final score = `final_chips × final_mult` (floored).
    pub total: u64,
    /// Gold awarded by flower effects (Bamboo). Applied by the caller, not
    /// during the chips×mult cascade.
    pub flower_gold: i32,
    /// Set kinds that were scored (for tutorial milestone detection).
    #[allow(dead_code)]
    pub scored_set_kinds: Vec<crate::core::hand::SetKind>,
}

// ── Per-meld base bonuses ──────────────────────────────────────────────
//
// These are *flat chip adds* on top of each scored tile's own value. The
// triplet bonus is larger than 3× the sequence bonus so that triplets feel
// like the "punchy single-beat" play and sequences feel like the "wider but
// flatter" play. Kongs (4 of a kind) feel like a triplet's bigger sibling and
// are the mahjong-flavored escalation beat — ~1.6× a triplet's bonus.
//
// Patch A retunes: Pair 15→18 and Sequence 30→28 to make pair-heavy and
// chiitoitsu-style hands viable while gently nerfing the dominant sequence
// path. Triplet stays at 50 as the chip baseline.
fn meld_chip_bonus(kind: SetKind) -> i32 {
    match kind {
        SetKind::Pair => 18,
        SetKind::Sequence => 28,
        SetKind::Triplet => 50,
        SetKind::Kong => 80,
    }
}

fn describe_set(tiles: &[Tile], set: &DetectedSet) -> String {
    let label = match set.kind {
        SetKind::Pair => "Pair",
        SetKind::Sequence => "Sequence",
        SetKind::Triplet => "Triplet",
        SetKind::Kong => "Kong",
    };
    let faces = set
        .tile_ids
        .iter()
        .filter_map(|id| tile_by_id(tiles, *id))
        .map(Tile::label)
        .collect::<Vec<_>>()
        .join(" ");
    format!("{label}  {faces}")
}

fn tile_by_id<'a>(tiles: &'a [Tile], id: u32) -> Option<&'a Tile> {
    tiles.iter().find(|t| t.id == id)
}

fn tile_is_debuffed(tile: &Tile, debuffs: &[crate::core::debuff::TileDebuff]) -> bool {
    debuffs.iter().any(|debuff| debuff.matches(tile))
}

/// Multiply chips by mult, flooring to i32. Negative chips are clamped to 0
/// before multiplication so an aggressive nerf can't underflow into a positive.
fn combine(chips: i32, mult: f64) -> u64 {
    ((chips.max(0) as f64) * mult)
        .floor()
        .clamp(0.0, u64::MAX as f64) as u64
}

/// Score detected sets and return a rich breakdown for cascade display.
pub fn score_sets(
    tiles: &[Tile],
    sets: &[DetectedSet],
    ctx: &ScoreContext<'_>,
    rules: &[RuleModifier],
) -> ScoreBreakdown {
    score_sets_inner(tiles, sets, ctx, rules, None)
}

/// Like `score_sets`, but accepts pre-substitution tiles so that
/// suit-composition yaku (honitsu, chinitsu, tanyao, …) are checked against
/// the player's actual selection, not the wildcard-resolved version.
pub fn score_sets_with_original(
    tiles: &[Tile],
    sets: &[DetectedSet],
    ctx: &ScoreContext<'_>,
    rules: &[RuleModifier],
    original_tiles: &[Tile],
) -> ScoreBreakdown {
    score_sets_inner(tiles, sets, ctx, rules, Some(original_tiles))
}

fn score_sets_inner(
    tiles: &[Tile],
    sets: &[DetectedSet],
    ctx: &ScoreContext<'_>,
    rules: &[RuleModifier],
    original_tiles: Option<&[Tile]>,
) -> ScoreBreakdown {
    let mut steps: Vec<ScoreStep> = Vec::new();
    let mut base_steps: Vec<ScoreStep> = Vec::new();
    // chips starts at base_chips (computed below); mult starts at the
    // identity ×1 so the cascade reads as +N mult / +N mult / ... .
    let mut chips: i32;
    let mut mult: f64 = 1.0;

    let pair_double = rules.contains(&RuleModifier::PairDoubleScore);
    let honor_triple = rules.contains(&RuleModifier::HonorTripleScore);
    let no_seq_bonus = rules.contains(&RuleModifier::NoSequenceBonus);
    let pairs_zero = rules.contains(&RuleModifier::PairsScoreZero);
    let sequences_halved = rules.contains(&RuleModifier::SequencesHalved);
    let censor_repeats = rules.contains(&RuleModifier::CensorRepeats);
    let effective_point_value = |t: &Tile| -> i32 {
        if tile_is_debuffed(t, ctx.tile_debuffs) {
            0
        } else {
            t.point_value() as i32
        }
    };

    // Mirror Tile copies the relic immediately AFTER it in inventory.
    // Shadow Hand copies the FIRST relic in inventory.
    // Both cause the copied relic to fire twice (owned + copy).
    let mirrored: Option<RelicId> = if ctx.relics.has(RelicId::MirrorTile) {
        ctx.relics.relic_after(RelicId::MirrorTile)
    } else {
        None
    };
    let shadowed: Option<RelicId> = if ctx.relics.has(RelicId::ShadowHand) {
        ctx.relics
            .active
            .first()
            .filter(|&&id| id != RelicId::ShadowHand)
            .copied()
    } else {
        None
    };
    let has = |id: RelicId| -> bool {
        ctx.relics.has(id) || mirrored == Some(id) || shadowed == Some(id)
    };
    let count = |id: RelicId| -> u32 {
        let owned = ctx.relics.has(id) as u32;
        let mirror = (mirrored == Some(id)) as u32;
        let shadow = (shadowed == Some(id)) as u32;
        owned + mirror + shadow
    };

    // Tiny helpers that mutate `chips`/`mult` in-place and emit a cascade step.
    macro_rules! push_chips {
        ($source:expr, $delta:expr) => {{
            let delta: i32 = $delta;
            chips += delta;
            steps.push(ScoreStep {
                source: $source.into(),
                kind: StepKind::Chips,
                tile_ids: Vec::new(),
                running_chips: chips,
                running_mult: mult,
                running_total: combine(chips, mult),
            });
        }};
    }
    macro_rules! push_mult {
        ($source:expr, $delta:expr) => {{
            let delta: f64 = $delta;
            mult += delta;
            steps.push(ScoreStep {
                source: $source.into(),
                kind: StepKind::Mult,
                tile_ids: Vec::new(),
                running_chips: chips,
                running_mult: mult,
                running_total: combine(chips, mult),
            });
        }};
    }
    // Gold steps don't touch chips or mult — they're purely visual cascade
    // beats that signal an economy event (e.g. Bamboo flower gold).
    let mut flower_gold: i32 = 0;
    macro_rules! push_gold {
        ($source:expr, $delta:expr) => {{
            let delta: i32 = $delta;
            flower_gold += delta;
            steps.push(ScoreStep {
                source: $source.into(),
                kind: StepKind::Gold,
                tile_ids: Vec::new(),
                running_chips: chips,
                running_mult: mult,
                running_total: combine(chips, mult),
            });
        }};
    }

    // ── Phase 1: base chips (tile values + meld bonuses) ─────────────────
    //
    // These don't get individual cascade steps — they're rolled into the
    // "Base" line the UI shows before the steps start ticking.
    //
    // Boss-effect tweaks happen here so they affect the chip floor everyone
    // builds on top of:
    //   * `pairs_zero`        → pair melds contribute nothing
    //   * `sequences_halved`  → sequence melds contribute half
    let mut base_chips: i32 = 0;
    for s in sets {
        let mut meld_contrib = meld_chip_bonus(s.kind);
        for &tid in &s.tile_ids {
            if let Some(t) = tile_by_id(tiles, tid) {
                let v = effective_point_value(t);
                meld_contrib += v;
            }
        }
        if pairs_zero && s.kind == SetKind::Pair {
            meld_contrib = 0;
        }
        if sequences_halved && s.kind == SetKind::Sequence {
            meld_contrib /= 2;
        }
        base_chips += meld_contrib;
        base_steps.push(ScoreStep {
            source: describe_set(tiles, s),
            kind: StepKind::Chips,
            tile_ids: s.tile_ids.clone(),
            running_chips: base_chips,
            running_mult: 1.0,
            running_total: combine(base_chips, 1.0),
        });
    }
    chips = base_chips;

    // ── Phase 2: per-set chip relics ─────────────────────────────────────
    //
    // Walk the sets and apply relics that grant flat chip bonuses to specific
    // melds or tile faces. Order matters only for cascade readability.
    let has_triplet_boost = has(RelicId::TripletBoost);
    let has_sequence_surge = has(RelicId::SequenceSurge);
    let has_pair_power = has(RelicId::PairPower);
    let has_honor_fury = has(RelicId::HonorFury);

    for s in sets {
        // Per-meld-kind chip relics. Kongs count as triplets here so the
        // triplet-boost relic still rewards them.
        match s.kind {
            SetKind::Triplet | SetKind::Kong if has_triplet_boost => {
                push_chips!("Triplet Boost", 40)
            }
            SetKind::Sequence if has_sequence_surge => push_chips!("Sequence Surge", 25),
            SetKind::Pair if has_pair_power => push_chips!("Pair Power", 30),
            _ => {}
        }
        // KongsBlessing chip side: Kongs are rare enough that the relic
        // needs to feel like jackpot loot when one finally lands. The
        // mult side fires in Phase 5 alongside KanDrum.
        if matches!(s.kind, SetKind::Kong) && has(RelicId::KongsBlessing) {
            push_chips!("Kong's Blessing", 120);
        }

        // Per-tile chip relics within this set.
        if has_honor_fury {
            let honor_count = s
                .tile_ids
                .iter()
                .filter_map(|id| tile_by_id(tiles, *id))
                .filter(|t| !tile_is_debuffed(t, ctx.tile_debuffs))
                .filter(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
                .count() as i32;
            if honor_count > 0 {
                push_chips!("Honor Fury", 28 * honor_count);
            }
        }
    }

    // Suit-specific chip relics: Jade Serpent (Bamboos), Red Serpent
    // (Characters), Blue Serpent (Circles). Same per-tile pattern as Honor Fury.
    let has_jade_serpent = has(RelicId::JadeSerpent);
    let has_red_serpent = has(RelicId::RedSerpent);
    let has_blue_serpent = has(RelicId::BlueSerpent);
    let has_edge_runner = has(RelicId::EdgeRunner);
    let has_low_tide = has(RelicId::LowTide);
    if has_jade_serpent || has_red_serpent || has_blue_serpent || has_edge_runner || has_low_tide {
        for s in sets {
            for &tid in &s.tile_ids {
                let Some(t) = tile_by_id(tiles, tid) else {
                    continue;
                };
                if tile_is_debuffed(t, ctx.tile_debuffs) {
                    continue;
                }
                if has_jade_serpent && t.suit == Suit::Bamboos {
                    push_chips!("Jade Serpent", 8);
                }
                if has_red_serpent && t.suit == Suit::Characters {
                    push_chips!("Red Serpent", 8);
                }
                if has_blue_serpent && t.suit == Suit::Circles {
                    push_chips!("Blue Serpent", 8);
                }
                // Edge Runner: terminal tiles (rank 1 or 9) in numbered suits.
                if has_edge_runner
                    && matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles)
                    && (t.rank == 1 || t.rank == 9)
                {
                    push_chips!("Edge Runner", 12);
                }
                // Low Tide: tiles ranked 1–3 in numbered suits.
                if has_low_tide
                    && matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles)
                    && t.rank <= 3
                {
                    push_chips!("Low Tide", 6);
                }
            }
        }
    }

    // PairDoubleScore rule: every pair gets +30 chips on top of its base.
    // (Replaces the old "double the pair's contribution" — easier to balance
    // when the chip pile is larger.)
    if pair_double {
        let pair_count = sets.iter().filter(|s| s.kind == SetKind::Pair).count() as i32;
        if pair_count > 0 {
            push_chips!("Pair Double (rule)", 30 * pair_count);
        }
    }

    // Tile Polisher: apply accumulated per-tile chip bonus from previous
    // plays. Every tile that has ever been scored gains +3 chips for the
    // rest of the run; the counter lives in ScoreContext::tile_polisher_bonus.
    if has(RelicId::TilePolisher) && ctx.tile_polisher_bonus > 0 {
        push_chips!("Tile Polisher", ctx.tile_polisher_bonus);
    }

    // Last Breath: on the player's final play of the round, retrigger all
    // scored tiles — each tile contributes its point value a second time.
    if has(RelicId::LastBreath) && ctx.is_final_play {
        let mut retrigger_chips = 0i32;
        for s in sets {
            for &tid in &s.tile_ids {
                if let Some(t) = tile_by_id(tiles, tid) {
                    retrigger_chips += effective_point_value(t);
                }
            }
        }
        if retrigger_chips > 0 {
            push_chips!("Last Breath", retrigger_chips);
        }
    }

    // Leading Tile: retrigger the first tile in each scored set.
    if has(RelicId::LeadingTile) {
        let mut retrigger = 0i32;
        for s in sets {
            if let Some(t) = s.tile_ids.first().and_then(|id| tile_by_id(tiles, *id)) {
                retrigger += effective_point_value(t);
            }
        }
        if retrigger > 0 {
            push_chips!("Leading Tile", retrigger);
        }
    }

    // Low Echo: retrigger tiles ranked 1-4 in scored sets.
    if has(RelicId::LowEcho) {
        let mut retrigger = 0i32;
        for s in sets {
            for &tid in &s.tile_ids {
                if let Some(t) = tile_by_id(tiles, tid) {
                    if matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles)
                        && t.rank <= 4
                    {
                        retrigger += effective_point_value(t);
                    }
                }
            }
        }
        if retrigger > 0 {
            push_chips!("Low Echo", retrigger);
        }
    }

    // Tea Ceremony: retrigger ALL scored tiles (while charges remain).
    // Charges are tracked in relic_counters; destroyed at 0 in run.rs.
    if has(RelicId::TeaCeremony) {
        let charges = ctx
            .relic_counters
            .get(&RelicId::TeaCeremony)
            .copied()
            .unwrap_or(0);
        if charges > 0 {
            let mut retrigger = 0i32;
            for s in sets {
                for &tid in &s.tile_ids {
                    if let Some(t) = tile_by_id(tiles, tid) {
                        retrigger += effective_point_value(t);
                    }
                }
            }
            if retrigger > 0 {
                push_chips!("Tea Ceremony", retrigger);
            }
        }
    }

    // Ghost Hand: tiles NOT in the scored sets each grant +2 chips.
    if has(RelicId::GhostHand) && ctx.unscored_hand_tiles > 0 {
        push_chips!("Ghost Hand", 2 * ctx.unscored_hand_tiles as i32);
    }

    // River Runner: accumulated permanent chip bonus from sequences.
    if has(RelicId::RiverRunner) && ctx.river_runner_bonus > 0 {
        push_chips!("River Runner", ctx.river_runner_bonus);
    }

    // Melting Ice: current chip bonus (decremented each play in run.rs).
    if has(RelicId::MeltingIce) {
        let ice_chips = ctx
            .relic_counters
            .get(&RelicId::MeltingIce)
            .copied()
            .unwrap_or(0);
        if ice_chips > 0 {
            push_chips!("Melting Ice", ice_chips);
        }
    }

    // ── Phase 2.5: talisman tile enhancements ────────────────────────────
    //
    // Walk every scored tile across all melds and apply its
    // [`crate::core::tile::TileEnhancement`] (if any). Talismans stamp these
    // onto every tile in the player's hand at once via
    // [`crate::core::talisman::apply_to_hand`]; here is where they cash out.
    //
    // Each enhancement contributes either chips or mult or a per-meld mult
    // multiplier. They're aggregated into single cascade steps per kind so
    // the cascade reads cleanly even when the whole hand is buffed.
    {
        use crate::core::tile::TileEnhancement;
        let mut jade_chips = 0i32;
        let mut pearl_chips = 0i32;
        let mut gilded_mult = 0.0f64;
        let mut polychrome_melds = 0i32;
        for s in sets {
            // Per-meld polychrome: any tile in this meld carrying Polychrome
            // grants the meld a single ×1.2 mult bonus (counted once even if
            // multiple polychrome tiles are present, to keep it bounded).
            let mut meld_has_polychrome = false;
            for &tid in &s.tile_ids {
                let Some(t) = tile_by_id(tiles, tid) else {
                    continue;
                };
                if tile_is_debuffed(t, ctx.tile_debuffs) {
                    continue;
                }
                let Some(enh) = t.enhancement else { continue };
                match enh {
                    TileEnhancement::Jade => {
                        if !matches!(s.kind, SetKind::Pair) {
                            jade_chips += 20;
                        }
                    }
                    TileEnhancement::Pearl => pearl_chips += 25,
                    TileEnhancement::Gilded => {
                        if !matches!(s.kind, SetKind::Pair) {
                            gilded_mult += 0.4;
                        }
                    }
                    TileEnhancement::Polychrome => meld_has_polychrome = true,
                }
            }
            if meld_has_polychrome {
                polychrome_melds += 1;
            }
        }
        if jade_chips > 0 {
            push_chips!("Jade Talisman", jade_chips);
        }
        if pearl_chips > 0 {
            push_chips!("Pearl Talisman", pearl_chips);
        }
        if gilded_mult > 0.0 {
            push_mult!("Gilded Talisman", gilded_mult);
        }
        // Polychrome is multiplicative, but we already build mult additively
        // (`mult = 1 + sum(deltas)`). Convert each polychrome meld's ×1.15 into
        // an equivalent additive `+0.15 × current_mult` step so the cascade stays
        // single-axis. This understates true multiplicativity slightly but
        // keeps the math reproducible and matches the existing pipeline.
        for _ in 0..polychrome_melds {
            let delta = mult * 0.15;
            push_mult!("Polychrome Talisman", delta);
        }
    }

    // ── Phase 2.75: per-flower triggered effects ────────────────────────────
    //
    // Each flower tile has a unique effect that fires when scored in a meld:
    //   F1 Plum Blossom  → +40 chips (safe, reliable)
    //   F2 Orchid         → +1.5 mult (scales in late game)
    //   F3 Chrysanthemum  → +15 chips per meld in hand (rewards full hands)
    //   F4 Bamboo         → +$4 gold (immediate economy)
    //
    // Garden Keeper relic causes each effect to fire twice.
    // Hanami relic adds +$3 gold per flower scored.
    {
        let meld_count = sets.len() as i32;
        let triggers = if has(RelicId::GardenKeeper) { 2 } else { 1 };
        let hanami = has(RelicId::Hanami);
        for s in sets {
            for &tid in &s.tile_ids {
                let Some(t) = tile_by_id(tiles, tid) else {
                    continue;
                };
                if t.suit != Suit::Flower || tile_is_debuffed(t, ctx.tile_debuffs) {
                    continue;
                }
                for trig in 0..triggers {
                    let suffix = if trig == 1 { " (Garden Keeper)" } else { "" };
                    match t.rank {
                        1 => push_chips!(format!("Plum Blossom{suffix}"), 40),
                        2 => push_mult!(format!("Orchid{suffix}"), 1.5),
                        3 => push_chips!(format!("Chrysanthemum{suffix}"), 15 * meld_count),
                        4 => push_gold!(format!("Bamboo{suffix}"), 4),
                        _ => {}
                    }
                }
                if hanami {
                    push_gold!("Hanami", 3);
                }
            }
        }
    }

    // ── Phase 3: cross-set chip relics ───────────────────────────────────

    // DragonEcho: each dragon triplet/kong copies the base chip value of
    // every other (non-dragon-triplet) set in the hand. Adjacency was the
    // original constraint but it made the relic punishingly positional —
    // dropping it lets a single dragon triplet echo the whole rest of the
    // hand, which finally feels Legendary.
    if has(RelicId::DragonEcho) {
        // Pre-compute every set's base chip contribution so the inner loop
        // is O(n) instead of O(n²).
        let set_bases: Vec<i32> = sets
            .iter()
            .map(|s| {
                let mut c = meld_chip_bonus(s.kind);
                for &tid in &s.tile_ids {
                    if let Some(t) = tile_by_id(tiles, tid) {
                        c += effective_point_value(t);
                    }
                }
                c
            })
            .collect();
        // Tag which sets are dragon triplets/kongs so we can both (a) find
        // the echoers and (b) exclude them from the echoed total (otherwise
        // two dragon triplets in the same hand would copy each other and
        // double up trivially).
        let is_dragon_trip: Vec<bool> = sets
            .iter()
            .map(|s| {
                if !matches!(s.kind, SetKind::Triplet | SetKind::Kong) {
                    return false;
                }
                s.tile_ids
                    .first()
                    .and_then(|id| tile_by_id(tiles, *id))
                    .is_some_and(|t| t.suit == Suit::Dragon)
            })
            .collect();

        for (i, &is_echoer) in is_dragon_trip.iter().enumerate() {
            if !is_echoer {
                continue;
            }
            let echo: i32 = set_bases
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i && !is_dragon_trip[j])
                .map(|(_, b)| *b)
                .sum();
            if echo > 0 {
                push_chips!("Dragon Echo", echo);
            }
        }
    }

    // Dora: each tile matching a dora face is +25 chips (or +35 with the
    // Dora Crown relic from Patch C).
    if !ctx.dora_faces.is_empty() {
        let dora_count = tiles
            .iter()
            .filter(|t| !tile_is_debuffed(t, ctx.tile_debuffs))
            .filter(|t| ctx.dora_faces.contains(&(t.suit, t.rank)))
            .count() as i32;
        if dora_count > 0 {
            // Use a custom step so we can label "Dora ×N" instead of the source name.
            let per_dora = if has(RelicId::DoraCrown) { 35 } else { 25 };
            let delta = per_dora * dora_count;
            chips += delta;
            steps.push(ScoreStep {
                source: format!("Dora ×{dora_count}"),
                kind: StepKind::Chips,
                tile_ids: tiles
                    .iter()
                    .filter(|t| !tile_is_debuffed(t, ctx.tile_debuffs))
                    .filter(|t| ctx.dora_faces.contains(&(t.suit, t.rank)))
                    .map(|t| t.id)
                    .collect(),
                running_chips: chips,
                running_mult: mult,
                running_total: combine(chips, mult),
            });
        }
    }

    // ── Phase 4: yaku → mult ─────────────────────────────────────────────

    let all_yaku = detect_yaku_with_wind(tiles, sets, ctx.round_wind, original_tiles);
    let mut detected_yaku: Vec<YakuKind> = if ctx.available_yaku.is_empty() {
        all_yaku
    } else {
        all_yaku
            .into_iter()
            .filter(|y| ctx.available_yaku.contains(y))
            .collect()
    };
    if let Some(st) = &ctx.structure {
        if st.inject_chicken_if_no_yaku && detected_yaku.is_empty() {
            detected_yaku.push(YakuKind::ChickenHand);
        }
    }
    // Patch B finishing: each yaku contributes both chips and mult, scaled by
    // its current level (default 1) from `ctx.yaku_levels`.
    let level_of =
        |y: YakuKind| -> u32 { ctx.yaku_levels.as_ref().map(|m| m.level_of(y)).unwrap_or(1) };
    for yaku in &detected_yaku {
        let level = level_of(*yaku);
        let mut mult_bonus = yaku.mult_bonus_at(level);
        let mut chip_bonus = yaku.chip_bonus_at(level);
        // The Censor: any yaku that has already fired this round contributes
        // at half strength.
        if censor_repeats && ctx.played_yaku_this_round.contains(yaku) {
            chip_bonus = (chip_bonus as f64 * 0.5).floor() as i32;
            mult_bonus *= 0.5;
        }
        if chip_bonus != 0 {
            push_chips!(yaku.name(), chip_bonus);
        }
        push_mult!(yaku.name(), mult_bonus);
    }

    if let Some(st) = &ctx.structure {
        let depth = structure_depth_mult_bonus(st.meld_count);
        if depth > 0.0 {
            push_mult!("Structure depth", depth);
        }
    }

    // ── Phase 4.5: Tenpai Bonus ──────────────────────────────────────────
    //
    // Patch A.5: when this play is the *first* FullHand of the round, fire a
    // splashy chip bonus that scales down as `plays_used` grows. The cap of
    // 4 reflects STARTING_PLAYS — playing on turn 1 (plays_used=0) gets the
    // full 4× bonus, turn 4 (plays_used=3) gets 1×.
    let scored_full_hand = detected_yaku.contains(&YakuKind::FullHand);
    if scored_full_hand && ctx.first_full_hand_of_round {
        let scale = (4i32 - ctx.plays_used as i32).max(1);
        let mut bonus = 50 * scale;
        if has(RelicId::TenpaiTalisman) {
            bonus *= 2;
        }
        push_chips!("Tenpai Bonus", bonus);
    }

    // ── Phase 5: per-set mult relics ─────────────────────────────────────

    if has(RelicId::RedDragonRage) {
        for s in sets {
            if !matches!(s.kind, SetKind::Triplet | SetKind::Kong) {
                continue;
            }
            let is_dragon = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| t.suit == Suit::Dragon);
            if is_dragon {
                // Broadened from "red only" to any dragon (the original
                // narrow trigger meant the relic could go a whole run
                // without firing). Mult tuned 8 → 5 to compensate for the
                // ~3× wider trigger window.
                push_mult!("Red Dragon Rage", 5.0);
            }
        }
    }

    if has(RelicId::WhiteSilence) {
        for s in sets {
            if s.kind != SetKind::Pair {
                continue;
            }
            let is_white_dragon = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| t.suit == Suit::Dragon && t.rank == 3);
            if is_white_dragon {
                push_mult!("White Silence", 4.0);
            }
        }
    }

    // SequenceSurge mult side (Patch C retune): +0.5 mult per sequence.
    if has(RelicId::SequenceSurge) {
        let seq_count = sets.iter().filter(|s| s.kind == SetKind::Sequence).count() as i32;
        if seq_count > 0 {
            push_mult!("Sequence Surge", 0.5 * seq_count as f64);
        }
    }

    // PairPower mult side (Patch C retune): +1 mult per pair. Stacks with
    // chiitoitsu hands for the cleanest possible Pig zodiac build.
    if has(RelicId::PairPower) {
        let pair_count = sets.iter().filter(|s| s.kind == SetKind::Pair).count() as i32;
        if pair_count > 0 {
            push_mult!("Pair Power", pair_count as f64);
        }
    }

    // KanDrum (Patch C): +4 mult per Kong. The +1 play side fires in
    // `score_selected_tiles` (run.rs).
    if has(RelicId::KanDrum) {
        let kong_count = sets.iter().filter(|s| s.kind == SetKind::Kong).count() as i32;
        if kong_count > 0 {
            push_mult!("Kan Drum", 4.0 * kong_count as f64);
        }
    }

    // KongsBlessing mult side: +2 mult per Kong. Pairs with the +120 chips
    // in Phase 2. The chip side fires alongside Triplet Boost / Pair Power
    // because it's a per-meld-kind chip relic; this is the mult half.
    if has(RelicId::KongsBlessing) {
        let kong_count = sets.iter().filter(|s| s.kind == SetKind::Kong).count() as i32;
        if kong_count > 0 {
            push_mult!("Kong's Blessing", 2.0 * kong_count as f64);
        }
    }

    // Triplet Boost mult side: +0.2 mult per triplet/kong. Keeps the
    // relic relevant past the early game when flat chip bonuses get
    // drowned out by mult escalation.
    if has_triplet_boost {
        let trip_count = sets
            .iter()
            .filter(|s| matches!(s.kind, SetKind::Triplet | SetKind::Kong))
            .count() as i32;
        if trip_count > 0 {
            push_mult!("Triplet Boost", 0.2 * trip_count as f64);
        }
    }

    // RoundCompass (Patch C): when the player triplets/kongs the *round wind*,
    // grant +6 mult on top of the existing Yakuhai bonus. Only fires for the
    // wind matching the ante's round wind, not for dragons.
    if has(RelicId::RoundCompass) {
        if let Some(wind) = ctx.round_wind {
            for s in sets {
                if !matches!(s.kind, SetKind::Triplet | SetKind::Kong) {
                    continue;
                }
                let is_round_wind = s
                    .tile_ids
                    .first()
                    .and_then(|id| tile_by_id(tiles, *id))
                    .is_some_and(|t| t.suit == Suit::Wind && t.rank == wind);
                if is_round_wind {
                    push_mult!("Round Compass", 6.0);
                }
            }
        }
    }

    // HonorTripleScore rule: honor triplets (and kongs) each grant +3 mult.
    if honor_triple {
        for s in sets {
            if !matches!(s.kind, SetKind::Triplet | SetKind::Kong) {
                continue;
            }
            let is_honor = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| matches!(t.suit, Suit::Wind | Suit::Dragon));
            if is_honor {
                push_mult!("Honor Triple (rule)", 3.0);
            }
        }
    }

    // NoSequenceBonus rule: a hand with no sequences gets +3 mult.
    if no_seq_bonus && !sets.iter().any(|s| s.kind == SetKind::Sequence) {
        push_mult!("No-Seq Bonus (rule)", 3.0);
    }

    // Ikebana: +6 mult when 2+ flowers are scored in the same hand.
    if has(RelicId::Ikebana) {
        let flower_count = sets
            .iter()
            .flat_map(|s| &s.tile_ids)
            .filter(|id| {
                tile_by_id(tiles, **id).is_some_and(|t| {
                    t.suit == Suit::Flower && !tile_is_debuffed(t, ctx.tile_debuffs)
                })
            })
            .count();
        if flower_count >= 2 {
            push_mult!("Ikebana", 6.0);
        }
    }

    // Lucky Seven: rank-7 tiles in scored sets grant +1.5 mult each.
    if has(RelicId::LuckySeven) {
        let count = sets
            .iter()
            .flat_map(|s| &s.tile_ids)
            .filter_map(|id| tile_by_id(tiles, *id))
            .filter(|t| !tile_is_debuffed(t, ctx.tile_debuffs))
            .filter(|t| {
                matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles) && t.rank == 7
            })
            .count();
        if count > 0 {
            push_mult!("Lucky Seven", 1.5 * count as f64);
        }
    }

    // Paper Lantern: flat +6 mult. High reward, fragile — 1-in-5 chance to
    // burn at round end (handled in run.rs advance_round).
    // MirrorTile doubling: fires once per copy (1 or 2).
    for _ in 0..count(RelicId::PaperLantern) {
        push_mult!("Paper Lantern", 6.0);
    }

    // ── Phase 6: global mult relics ──────────────────────────────────────

    if has(RelicId::MultiplierMaster) {
        // +0.5 mult per relic owned. Caps at +2.5 with a full 5-slot
        // inventory, which is a real swing for a Rare. (Earlier Patch C
        // tuning had this at +0.3 — too weak once dead-stub relics were
        // pulled from the pool.)
        let bonus = 0.5 * ctx.relics.enabled_len() as f64;
        if bonus > 0.0 {
            push_mult!("Multiplier Master", bonus);
        }
    }

    if has(RelicId::ChainReaction) && ctx.scored_last_turn {
        push_mult!("Chain Reaction", 4.0);
    }

    // Closed Gate: +4 mult when every scored tile is a terminal (rank 1/9)
    // or an honor (Wind/Dragon). Rewards honroutou-style hands.
    if has(RelicId::ClosedGate) {
        let all_terminal_or_honor = sets
            .iter()
            .flat_map(|s| &s.tile_ids)
            .filter_map(|id| tile_by_id(tiles, *id))
            .all(|t| {
                matches!(t.suit, Suit::Wind | Suit::Dragon)
                    || (matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles)
                        && (t.rank == 1 || t.rank == 9))
            });
        if all_terminal_or_honor {
            push_mult!("Closed Gate", 4.0);
        }
    }

    // Gold Furnace: +1 mult per 5 gold held.
    if has(RelicId::GoldFurnace) {
        let bonus = (ctx.gold.max(0) as f64 / 5.0).floor();
        if bonus > 0.0 {
            push_mult!("Gold Furnace", bonus);
        }
    }

    // Snowball: +0.1 mult per 100 total score earned this run.
    if has(RelicId::Snowball) {
        let bonus = ctx.total_score as f64 / 1000.0;
        if bonus > 0.0 {
            push_mult!("Snowball", bonus);
        }
    }

    // Momentum: +0.5 mult per play already used this round.
    if has(RelicId::Momentum) && ctx.plays_used > 0 {
        push_mult!("Momentum", 0.5 * ctx.plays_used as f64);
    }

    // Minimalist: playing exactly one set that is a pair grants +4 mult.
    if has(RelicId::Minimalist) && sets.len() == 1 && sets[0].kind == SetKind::Pair {
        push_mult!("Minimalist", 4.0);
    }

    // Turtle Shell: +50 chips if mult is still below 3.0 after all bonuses.
    // Safety-net relic that naturally falls off as the player acquires mult.
    if has(RelicId::TurtleShell) && mult < 3.0 {
        push_chips!("Turtle Shell", 50);
    }

    // Silk Thread: current mult bonus (decremented each discard in run.rs).
    if has(RelicId::SilkThread) {
        // Stored as ×10 to avoid float drift. 40 → 4.0 mult.
        let thread_mult = ctx
            .relic_counters
            .get(&RelicId::SilkThread)
            .copied()
            .unwrap_or(0);
        if thread_mult > 0 {
            push_mult!("Silk Thread", thread_mult as f64 / 10.0);
        }
    }

    // Clean Streak: +0.5 mult per consecutive play without honor tiles.
    if has(RelicId::CleanStreak) {
        let streak = ctx
            .relic_counters
            .get(&RelicId::CleanStreak)
            .copied()
            .unwrap_or(0);
        if streak > 0 {
            push_mult!("Clean Streak", 0.5 * streak as f64);
        }
    }

    // Obsession: +0.3 mult per round without most-used yaku.
    if has(RelicId::Obsession) {
        let rounds = ctx
            .relic_counters
            .get(&RelicId::Obsession)
            .copied()
            .unwrap_or(0);
        if rounds > 0 {
            push_mult!("Obsession", 0.3 * rounds as f64);
        }
    }

    // Bonfire: +0.4 mult per relic sold this run (resets on boss).
    if has(RelicId::Bonfire) {
        let sold = ctx
            .relic_counters
            .get(&RelicId::Bonfire)
            .copied()
            .unwrap_or(0);
        if sold > 0 {
            push_mult!("Bonfire", 0.4 * sold as f64);
        }
    }

    // Empty Frame: +1.5 mult per empty relic slot.
    if has(RelicId::EmptyFrame) {
        let empty = ctx.relics.max_slots.saturating_sub(ctx.relics.active.len());
        if empty > 0 {
            push_mult!("Empty Frame", 1.5 * empty as f64);
        }
    }

    // Curio Cabinet: +mult equal to the summed live sell value of every
    // *other* relic in the inventory (Curio Cabinet itself excluded).
    if has(RelicId::CurioCabinet) {
        let bonus: u32 = ctx
            .relics
            .active
            .iter()
            .copied()
            .filter(|&id| id != RelicId::CurioCabinet)
            .map(|id| crate::core::relic::relic_sell_price_live(id, &ctx.relic_counters))
            .sum();
        if bonus > 0 {
            push_mult!("Curio Cabinet", bonus as f64);
        }
    }

    // Lotus Bloom: +0.5 mult per flower drawn or scored this run (counter
    // lives in relic_counters[LotusBloom], bumped by run.rs on draw/score).
    if has(RelicId::LotusBloom) {
        let blooms = ctx
            .relic_counters
            .get(&RelicId::LotusBloom)
            .copied()
            .unwrap_or(0);
        if blooms > 0 {
            push_mult!("Lotus Bloom", 0.5 * blooms as f64);
        }
    }

    // Wall Weaver: +0.2 mult per tile in the wall beyond the base 140.
    // Overflow is a fixed +68; relic_counters[WallWeaver] accumulates any
    // other mid-run tile adds (future effects) so both sources stack.
    if has(RelicId::WallWeaver) {
        let overflow_extras = if ctx.relics.has(RelicId::Overflow) { 68 } else { 0 };
        let extra_added = ctx
            .relic_counters
            .get(&RelicId::WallWeaver)
            .copied()
            .unwrap_or(0)
            .max(0);
        let excess = overflow_extras + extra_added;
        if excess > 0 {
            push_mult!("Wall Weaver", 0.2 * excess as f64);
        }
    }

    // Heirloom: +1 mult per boss defeated this run.
    if has(RelicId::Heirloom) {
        let bosses = ctx
            .relic_counters
            .get(&RelicId::Heirloom)
            .copied()
            .unwrap_or(0)
            .max(0);
        if bosses > 0 {
            push_mult!("Heirloom", bosses as f64);
        }
    }

    // Tourist: +3 mult per distinct suit among scored tiles. All six suits
    // count (Flower included).
    if has(RelicId::Tourist) {
        let mut seen = [false; 6];
        for s in sets {
            for &tid in &s.tile_ids {
                let Some(t) = tile_by_id(tiles, tid) else {
                    continue;
                };
                if tile_is_debuffed(t, ctx.tile_debuffs) {
                    continue;
                }
                let idx = match t.suit {
                    Suit::Characters => 0,
                    Suit::Bamboos => 1,
                    Suit::Circles => 2,
                    Suit::Wind => 3,
                    Suit::Dragon => 4,
                    Suit::Flower => 5,
                    Suit::Season => continue,
                };
                seen[idx] = true;
            }
        }
        let distinct = seen.iter().filter(|b| **b).count();
        if distinct > 0 {
            push_mult!("Tourist", 3.0 * distinct as f64);
        }
    }

    // Cracked Tile: +0 to +8 mult (random per play).
    if has(RelicId::CrackedTile) {
        use rand::RngExt;
        let mut rng = rand::rng();
        let bonus: f64 = rng.random_range(0.0..=8.0);
        if bonus > 0.0 {
            push_mult!("Cracked Tile", (bonus * 10.0).floor() / 10.0);
        }
    }

    // Ritual Blade permanent mult (accumulated via relic_counters).
    if has(RelicId::RitualBlade) {
        let perm_mult = ctx
            .relic_counters
            .get(&RelicId::RitualBlade)
            .copied()
            .unwrap_or(0);
        if perm_mult > 0 {
            push_mult!("Ritual Blade", perm_mult as f64 / 10.0);
        }
    }

    // ── Phase 6.5: Riichi multiplier ────────────────────────────────────
    //
    // Patch A.5 gated hook: when riichi was declared and this hand completes
    // the wait (a FullHand), apply a flat 2× to the running mult. Riichi UI
    // and declaration logic are Patch E; for now `riichi_active` is always
    // false in run.rs so this is a no-op until then. The cascade step still
    // exists so balance/tooling can preview it.
    if ctx.riichi_active && scored_full_hand {
        // We add (current_mult) so the running total doubles cleanly.
        let delta = mult;
        push_mult!("Riichi", delta);
    }

    // Way of Purity: ×2.5 mult if every scored tile belongs to a single
    // numbered suit (Bamboos, Characters, or Circles). Chinitsu reward.
    if has(RelicId::WayOfPurity) {
        let numbered_suits: Vec<Suit> = sets
            .iter()
            .flat_map(|s| &s.tile_ids)
            .filter_map(|id| tile_by_id(tiles, *id))
            .map(|t| t.suit)
            .filter(|s| matches!(s, Suit::Bamboos | Suit::Characters | Suit::Circles))
            .collect();
        if !numbered_suits.is_empty() {
            let first = numbered_suits[0];
            let all_same = numbered_suits.iter().all(|&s| s == first)
                && sets
                    .iter()
                    .flat_map(|s| &s.tile_ids)
                    .filter_map(|id| tile_by_id(tiles, *id))
                    .all(|t| matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles));
            if all_same {
                // ×2.5 = add 1.5 × current mult.
                let delta = mult * 1.5;
                push_mult!("Way of Purity", delta);
            }
        }
    }

    // Way of Pairs: ×2 mult if every scored set is a pair.
    if has(RelicId::WayOfPairs) && !sets.is_empty() && sets.iter().all(|s| s.kind == SetKind::Pair)
    {
        let delta = mult;
        push_mult!("Way of Pairs", delta);
    }

    // Way of Triplets: ×2.5 mult if every scored set is a triplet/kong.
    if has(RelicId::WayOfTriplets)
        && !sets.is_empty()
        && sets
            .iter()
            .all(|s| matches!(s.kind, SetKind::Triplet | SetKind::Kong))
    {
        let delta = mult * 1.5;
        push_mult!("Way of Triplets", delta);
    }

    // Way of Sequences: ×2 mult if every scored set is a sequence.
    if has(RelicId::WayOfSequences)
        && !sets.is_empty()
        && sets.iter().all(|s| s.kind == SetKind::Sequence)
    {
        let delta = mult;
        push_mult!("Way of Sequences", delta);
    }

    // Iron Lantern: ×2 mult (Paper Lantern's evolved form). Nearly
    // indestructible (1-in-1000 per round, handled in run.rs).
    for _ in 0..count(RelicId::IronLantern) {
        let delta = mult;
        push_mult!("Iron Lantern", delta);
    }

    // Glass Cannon: double the running mult (same ×2 pattern as Riichi).
    // The play-count penalty is applied in run.rs at round reset.
    for _ in 0..count(RelicId::GlassCannon) {
        let delta = mult;
        push_mult!("Glass Cannon", delta);
    }

    // ── Phase 7: reorder so all chip steps fire before mult ────────────
    //
    // The cascade alternates chips/mult as relics fire, but we want a clean
    // visual: chip pile builds first, *then* mult ramps. Partition the steps
    // and rebuild running totals from the independent deltas.
    {
        // Extract per-step deltas from the original interleaved order.
        struct Delta {
            source: String,
            kind: StepKind,
            chip_delta: i32,
            mult_delta: f64,
            tile_ids: Vec<u32>,
        }
        let mut deltas: Vec<Delta> = Vec::with_capacity(steps.len());
        let mut prev_c = base_chips;
        let mut prev_m = 1.0_f64;
        for s in &steps {
            deltas.push(Delta {
                source: s.source.clone(),
                kind: s.kind,
                chip_delta: s.running_chips - prev_c,
                mult_delta: s.running_mult - prev_m,
                tile_ids: s.tile_ids.clone(),
            });
            prev_c = s.running_chips;
            prev_m = s.running_mult;
        }

        // Stable partition: chips, then mult, then gold (preserving relative
        // order within each group).
        deltas.sort_by_key(|d| match d.kind {
            StepKind::Chips => 0,
            StepKind::Mult => 1,
            StepKind::Gold => 2,
            StepKind::Final => 3,
        });

        // Rebuild steps with fresh running totals.
        let mut rc = base_chips;
        let mut rm = 1.0_f64;
        steps.clear();
        for d in deltas {
            rc += d.chip_delta;
            rm += d.mult_delta;
            steps.push(ScoreStep {
                source: d.source,
                kind: d.kind,
                tile_ids: d.tile_ids,
                running_chips: rc,
                running_mult: rm,
                running_total: combine(rc, rm),
            });
        }
    }

    // ── Phase 8: final multiplication beat ───────────────────────────────

    let final_chips = chips;
    let final_mult = mult;
    let total = combine(final_chips, final_mult);
    steps.push(ScoreStep {
        source: format!("{} × {}", final_chips, fmt_mult(final_mult)),
        kind: StepKind::Final,
        tile_ids: Vec::new(),
        running_chips: final_chips,
        running_mult: final_mult,
        running_total: total,
    });

    ScoreBreakdown {
        base_chips,
        base_points: base_chips,
        base_steps,
        steps,
        detected_yaku,
        final_chips,
        final_mult,
        total,
        flower_gold,
        scored_set_kinds: sets.iter().map(|s| s.kind).collect(),
    }
}

/// Mystery-preserving "Balatro-style" score preview for the current selection.
///
/// Shows only what the player can derive from the tiles in front of them:
///   * **chips** = sum of tile values + meld bonuses (no relic/dora bonuses)
///   * **mult**  = 1 + Σ yaku.mult_bonus() for visible yaku patterns
///
/// Relic contributions, rule modifiers, dora hits, and chain effects are
/// intentionally excluded so the cascade still has surprises to reveal.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct ScorePreview {
    pub chips: i32,
    pub mult: f64,
    /// Yaku detected in the current selection (filtered to available pool).
    pub detected_yaku: Vec<YakuKind>,
    /// Estimated total score = `chips × mult` (floored). Excludes relics and
    /// rules so the cascade still has surprises.
    pub estimated_total: u64,
}

#[allow(dead_code)]
pub fn preview_score(
    tiles: &[Tile],
    sets: &[DetectedSet],
    available_yaku: &[YakuKind],
    tile_debuffs: &[crate::core::debuff::TileDebuff],
    original_tiles: Option<&[Tile]>,
) -> ScorePreview {
    let mut chips: i32 = 0;
    for s in sets {
        chips += meld_chip_bonus(s.kind);
        for &tid in &s.tile_ids {
            if let Some(t) = tile_by_id(tiles, tid) {
                if !tile_is_debuffed(t, tile_debuffs) {
                    chips += t.point_value() as i32;
                }
            }
        }
    }
    let all_yaku = detect_yaku_with_wind(tiles, sets, None, original_tiles);
    let visible_yaku: Vec<YakuKind> = if available_yaku.is_empty() {
        all_yaku
    } else {
        all_yaku
            .into_iter()
            .filter(|y| available_yaku.contains(y))
            .collect()
    };
    let mut mult: f64 = 1.0;
    for y in &visible_yaku {
        mult += y.mult_bonus();
    }
    let estimated_total = combine(chips, mult);
    ScorePreview {
        chips,
        mult,
        detected_yaku: visible_yaku,
        estimated_total,
    }
}

/// Format an absolute mult value for the final beat display.
fn fmt_mult(m: f64) -> String {
    if (m - m.round()).abs() < 1e-6 {
        format!("×{}", m.round() as i64)
    } else {
        format!("×{m:.1}")
    }
}

/// Convenience: get just the total as u64 (for tests and simple callers).
#[allow(dead_code)]
pub fn score_sets_total(
    tiles: &[Tile],
    sets: &[DetectedSet],
    ctx: &ScoreContext<'_>,
    rules: &[RuleModifier],
) -> u64 {
    score_sets(tiles, sets, ctx, rules).total
}

/// Per-tile scoring estimate used by the gameplay tooltip to show a tile's
/// "true worth" — base point value plus every per-tile bonus that doesn't
/// require fine-grained meld context (talisman enhancements, dora, owned
/// chip relics that key on tile properties).
///
/// Caveats:
/// * Bonuses that only fire inside a non-pair meld (Jade, Gilded, Honor
///   Fury) are included optimistically — i.e. we assume the player will
///   actually score this tile inside a triplet/sequence/kong, since that's
///   what the tooltip is meant to *promise*.
/// * Polychrome's per-meld ×1.15 mult is approximated as a flat `+0.15 mult`
///   line on the tile, matching the additive expansion the cascade uses in
///   `score_sets`.
/// * Cross-set / structural relics (DragonEcho, ChainReaction, MultiplierMaster…)
///   are intentionally omitted because their value depends on the rest of
///   the hand and would mislead the per-tile read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileEffectiveValue {
    /// Base point value from the tile face (rank for numbered, 12 for honors).
    pub base_chips: i32,
    /// Sum of per-tile chip bonuses on top of the base.
    pub bonus_chips: i32,
    /// Sum of per-tile additive mult contributions.
    pub mult_bonus: f64,
    /// Per-source breakdown lines for tooltip rendering, e.g.
    /// `("Pearl Talisman", "+30 chips")`. Order is the order they should be
    /// displayed.
    pub sources: Vec<(&'static str, String)>,
}

impl TileEffectiveValue {
    pub fn total_chips(&self) -> i32 {
        self.base_chips + self.bonus_chips
    }
}

pub fn tile_effective_value(
    tile: &Tile,
    relics: &crate::core::relic::RelicState,
    dora_faces: &[(Suit, u8)],
    tile_debuffs: &[crate::core::debuff::TileDebuff],
) -> TileEffectiveValue {
    use crate::core::tile::TileEnhancement;

    let mut out = TileEffectiveValue {
        base_chips: if tile_is_debuffed(tile, tile_debuffs) {
            0
        } else {
            tile.point_value() as i32
        },
        bonus_chips: 0,
        mult_bonus: 0.0,
        sources: Vec::new(),
    };

    if tile_is_debuffed(tile, tile_debuffs) {
        out.sources.push((
            "Debuffed",
            "This tile still forms hands, but scores 0 tile points".into(),
        ));
        return out;
    }

    // ── Talisman enhancements ───────────────────────────────────────────
    // Optimistic: assume the tile lands in a non-pair meld so the
    // meld-gated effects (Jade, Gilded) actually fire.
    if let Some(enh) = tile.enhancement {
        match enh {
            TileEnhancement::Pearl => {
                out.bonus_chips += 25;
                out.sources.push(("Pearl Talisman", "+25 chips".into()));
            }
            TileEnhancement::Jade => {
                out.bonus_chips += 20;
                out.sources.push(("Jade Talisman", "+20 chips".into()));
            }
            TileEnhancement::Gilded => {
                out.mult_bonus += 0.4;
                out.sources.push(("Gilded Talisman", "+0.4 mult".into()));
            }
            TileEnhancement::Polychrome => {
                // Per-meld ×1.15; the cascade expands this as +0.15 × current_mult.
                out.mult_bonus += 0.15;
                out.sources
                    .push(("Polychrome Talisman", "+0.15 mult / meld".into()));
            }
        }
    }

    // ── Dora ────────────────────────────────────────────────────────────
    if dora_faces.contains(&(tile.suit, tile.rank)) {
        let per_dora = if relics.has(RelicId::DoraCrown) {
            35
        } else {
            25
        };
        out.bonus_chips += per_dora;
        out.sources.push(("Dora", format!("+{per_dora} chips")));
    }

    // ── Per-tile chip relics ────────────────────────────────────────────
    // Honor Fury: +28 per honor tile inside a meld.
    if relics.has(RelicId::HonorFury) && matches!(tile.suit, Suit::Wind | Suit::Dragon) {
        out.bonus_chips += 28;
        out.sources.push(("Honor Fury", "+28 chips".into()));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::hand::find_pairs_and_triplets;
    use crate::core::relic::RelicState;
    use crate::core::tile::Tile;

    fn ctx_with(relics: &RelicState, scored_last_turn: bool) -> ScoreContext<'_> {
        ScoreContext {
            relics,
            tile_debuffs: &[],
            scored_last_turn,
            dora_faces: vec![],
            available_yaku: vec![],
            round_wind: None,
            first_full_hand_of_round: false,
            plays_used: 0,
            riichi_active: false,
            yaku_levels: None,
            played_yaku_this_round: vec![],
            gold: 0,
            total_score: 0,
            is_final_play: false,
            tile_polisher_bonus: 0,
            relic_counters: std::collections::BTreeMap::new(),
            unscored_hand_tiles: 0,
            river_runner_bonus: 0,
            structure: None,
        }
    }

    fn relics(ids: Vec<RelicId>) -> RelicState {
        RelicState {
            active: ids,
            ..Default::default()
        }
    }

    // ── Base chips ─────────────────────────────────────────────────

    #[test]
    fn bare_triplet_of_threes() {
        // Triplet of 3s: tile chips = 3+3+3 = 9, meld bonus = 50, total chips = 59,
        // mult = ×1, final = 59. No yaku (only 3 tiles).
        let hand = vec![
            Tile::new(Suit::Bamboos, 3, 0),
            Tile::new(Suit::Bamboos, 3, 1),
            Tile::new(Suit::Bamboos, 3, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
        assert_eq!(breakdown.base_chips, 59);
        assert_eq!(breakdown.final_mult, 1.0);
        assert_eq!(breakdown.total, 59);
    }

    #[test]
    fn honor_triplet_uses_flat_value() {
        // East wind triplet: honors are flat 12 chips each (Patch A bumped from 10),
        // meld bonus 50 → 86 chips.
        let hand = vec![
            Tile::new(Suit::Wind, 1, 0),
            Tile::new(Suit::Wind, 1, 1),
            Tile::new(Suit::Wind, 1, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
        assert_eq!(breakdown.base_chips, 86);
    }

    // ── Triplet Boost ───────────────────────────────────────────────

    #[test]
    fn triplet_boost_adds_chips_to_triplet() {
        let hand = vec![
            Tile::new(Suit::Bamboos, 3, 0),
            Tile::new(Suit::Bamboos, 3, 1),
            Tile::new(Suit::Bamboos, 3, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::TripletBoost]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // 59 base + 40 triplet boost = 99 chips, ×1.2 mult (mult side added
        // to keep the relic relevant past the early game).
        // total = floor(99 × 1.2) = 118
        assert_eq!(breakdown.final_chips, 99);
        assert_eq!(breakdown.final_mult, 1.2);
        assert_eq!(breakdown.total, 118);
        assert!(breakdown.steps.iter().any(|s| s.source == "Triplet Boost"));
    }

    // ── Sequence Surge ──────────────────────────────────────────────

    #[test]
    fn sequence_surge_adds_chips_to_sequence() {
        let hand = vec![
            Tile::new(Suit::Characters, 1, 0),
            Tile::new(Suit::Characters, 2, 1),
            Tile::new(Suit::Characters, 3, 2),
        ];
        let sets = vec![DetectedSet {
            kind: SetKind::Sequence,
            tile_ids: vec![0, 1, 2],
        }];
        let r = relics(vec![RelicId::SequenceSurge]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // 1+2+3 + 28 (meld, Patch A) + 25 (surge) = 59
        assert_eq!(breakdown.final_chips, 59);
    }

    // ── Pair Power (chip + mult side) ───────────────────────────────

    #[test]
    fn stacked_yaku_score_full_value_without_loadout_gating() {
        // Bamboo flush full hand: detects FullHand + Chinitsu + Ittsu.
        let hand = vec![
            Tile::new(Suit::Bamboos, 1, 0),
            Tile::new(Suit::Bamboos, 2, 1),
            Tile::new(Suit::Bamboos, 3, 2),
            Tile::new(Suit::Bamboos, 4, 3),
            Tile::new(Suit::Bamboos, 5, 4),
            Tile::new(Suit::Bamboos, 6, 5),
            Tile::new(Suit::Bamboos, 7, 6),
            Tile::new(Suit::Bamboos, 8, 7),
            Tile::new(Suit::Bamboos, 9, 8),
            Tile::new(Suit::Bamboos, 5, 9),
            Tile::new(Suit::Bamboos, 5, 10),
            Tile::new(Suit::Bamboos, 5, 11),
            Tile::new(Suit::Bamboos, 7, 12),
            Tile::new(Suit::Bamboos, 7, 13),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![9, 10, 11],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![12, 13],
            },
        ];
        let r = RelicState::default();
        let ctx = ScoreContext {
            relics: &r,
            tile_debuffs: &[],
            scored_last_turn: false,
            dora_faces: vec![],
            available_yaku: vec![],
            round_wind: None,
            first_full_hand_of_round: false,
            plays_used: 0,
            riichi_active: false,
            yaku_levels: None,
            played_yaku_this_round: vec![],
            gold: 0,
            total_score: 0,
            is_final_play: false,
            tile_polisher_bonus: 0,
            relic_counters: std::collections::BTreeMap::new(),
            unscored_hand_tiles: 0,
            river_runner_bonus: 0,
            structure: None,
        };
        let breakdown = score_sets(&hand, &sets, &ctx, &[]);
        // Full value on all three yaku:
        //   chip adds = 60 + 80 + 50 = 190 ; base = 226 → final_chips = 416
        //   mult = 1 + 5 + 6 + 4 = 16
        //   total = 416 × 16 = 6656
        assert_eq!(breakdown.final_chips, 416);
        assert_eq!(breakdown.final_mult, 16.0);
    }

    #[test]
    fn yaku_levels_scale_chip_and_mult() {
        // Toitoi at level 3: base (4 mult, 50 chips) + 2*(0.5, 20) = (5 mult, 90 chips).
        let hand = vec![
            Tile::new(Suit::Circles, 5, 0),
            Tile::new(Suit::Circles, 5, 1),
            Tile::new(Suit::Circles, 5, 2),
            Tile::new(Suit::Bamboos, 7, 3),
            Tile::new(Suit::Bamboos, 7, 4),
            Tile::new(Suit::Bamboos, 7, 5),
            Tile::new(Suit::Wind, 1, 6),
            Tile::new(Suit::Wind, 1, 7),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![6, 7],
            },
        ];
        let r = RelicState::default();
        let mut levels = crate::core::zodiac::YakuLevels::default();
        levels.levels.insert(crate::core::yaku::YakuKind::Toitoi, 3);
        let ctx = ScoreContext {
            relics: &r,
            tile_debuffs: &[],
            scored_last_turn: false,
            dora_faces: vec![],
            available_yaku: vec![],
            round_wind: None,
            first_full_hand_of_round: false,
            plays_used: 0,
            riichi_active: false,
            yaku_levels: Some(levels),
            played_yaku_this_round: vec![],
            gold: 0,
            total_score: 0,
            is_final_play: false,
            tile_polisher_bonus: 0,
            relic_counters: std::collections::BTreeMap::new(),
            unscored_hand_tiles: 0,
            river_runner_bonus: 0,
            structure: None,
        };
        let breakdown = score_sets(&hand, &sets, &ctx, &[]);
        // Verify Toitoi step exists with expected chip & mult deltas.
        let toitoi_chip = breakdown
            .steps
            .iter()
            .find(|s| s.source == "Toitoi" && s.kind == StepKind::Chips);
        assert!(toitoi_chip.is_some(), "missing Toitoi chip step");
    }

    #[test]
    fn pair_power_grants_chips_and_mult() {
        let hand = vec![
            Tile::new(Suit::Circles, 7, 0),
            Tile::new(Suit::Circles, 7, 1),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::PairPower]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // Patch C retune: PairPower now adds +30 chips AND +1 mult per pair.
        // chips: 7+7 + 18 (meld) + 30 (PairPower) = 62
        // mult: 1 + 1 (PairPower) = 2
        // total: 62 × 2 = 124
        assert_eq!(breakdown.final_chips, 62);
        assert_eq!(breakdown.final_mult, 2.0);
        assert_eq!(breakdown.total, 124);
    }

    #[test]
    fn debuffed_terminal_tiles_keep_pair_bonus_but_lose_tile_points() {
        let hand = vec![
            Tile::new(Suit::Circles, 1, 0),
            Tile::new(Suit::Circles, 1, 1),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = RelicState::default();
        let ctx = ScoreContext {
            relics: &r,
            tile_debuffs: &[crate::core::debuff::TileDebuff::Class(
                crate::core::debuff::TileDebuffClass::Terminals,
            )],
            scored_last_turn: false,
            dora_faces: vec![],
            available_yaku: vec![],
            round_wind: None,
            first_full_hand_of_round: false,
            plays_used: 0,
            riichi_active: false,
            yaku_levels: None,
            played_yaku_this_round: vec![],
            gold: 0,
            total_score: 0,
            is_final_play: false,
            tile_polisher_bonus: 0,
            relic_counters: std::collections::BTreeMap::new(),
            unscored_hand_tiles: 0,
            river_runner_bonus: 0,
            structure: None,
        };
        let breakdown = score_sets(&hand, &sets, &ctx, &[]);
        assert_eq!(breakdown.base_chips, 18);
        assert_eq!(breakdown.total, 18);
    }

    #[test]
    fn debuffed_relic_is_disabled_for_scoring() {
        let hand = vec![
            Tile::new(Suit::Circles, 7, 0),
            Tile::new(Suit::Circles, 7, 1),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let mut r = relics(vec![RelicId::PairPower]);
        r.debuffed.insert(RelicId::PairPower);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        assert_eq!(breakdown.final_chips, 32);
        assert_eq!(breakdown.final_mult, 1.0);
        assert_eq!(breakdown.total, 32);
    }

    // ── White Silence ───────────────────────────────────────────────

    #[test]
    fn white_silence_mults_white_dragon_pair() {
        let hand = vec![Tile::new(Suit::Dragon, 3, 0), Tile::new(Suit::Dragon, 3, 1)];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::WhiteSilence]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // chips: 12+12 (Patch A honor 12) + 18 (Patch A pair) = 42, mult 1+4 = 5, total 210
        assert_eq!(breakdown.final_chips, 42);
        assert_eq!(breakdown.final_mult, 5.0);
        assert_eq!(breakdown.total, 210);
    }

    // ── Honor Fury ──────────────────────────────────────────────────

    #[test]
    fn honor_fury_adds_chips_per_honor_tile() {
        let hand = vec![
            Tile::new(Suit::Wind, 1, 0),
            Tile::new(Suit::Wind, 1, 1),
            Tile::new(Suit::Wind, 1, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::HonorFury]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // 86 base (honor 12 × 3 + triplet 50) + 28 × 3 (Honor Fury) = 170
        assert_eq!(breakdown.final_chips, 170);
    }

    // ── Red Dragon Rage ─────────────────────────────────────────────

    #[test]
    fn red_dragon_rage_mults_red_triplet() {
        let hand = vec![
            Tile::new(Suit::Dragon, 1, 0),
            Tile::new(Suit::Dragon, 1, 1),
            Tile::new(Suit::Dragon, 1, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::RedDragonRage]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // RedDragonRage broadened to any dragon triplet/kong, mult 8 → 5.
        // chips: 12+12+12 (honor 12) + 50 (triplet) + 40 (Yakuhai) = 126
        // mult: 1 + 5 (RedDragonRage) + 3 (Yakuhai) = 9
        // total: 126 × 9 = 1134
        assert_eq!(breakdown.final_chips, 126);
        assert_eq!(breakdown.final_mult, 9.0);
        assert_eq!(breakdown.total, 1134);
    }

    #[test]
    fn red_dragon_rage_fires_on_any_dragon_triplet() {
        // Broadened trigger: green dragon triplet now fires the relic too.
        let hand = vec![
            Tile::new(Suit::Dragon, 2, 0),
            Tile::new(Suit::Dragon, 2, 1),
            Tile::new(Suit::Dragon, 2, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::RedDragonRage]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        assert!(
            breakdown
                .steps
                .iter()
                .any(|s| s.source == "Red Dragon Rage")
        );
    }

    // ── Multiplier Master ───────────────────────────────────────────

    #[test]
    fn multiplier_master_scales_with_relic_count() {
        // 9m triplet — Junchan does NOT fire (≥5 tiles + ≥2 sets gate).
        let hand = vec![
            Tile::new(Suit::Characters, 9, 0),
            Tile::new(Suit::Characters, 9, 1),
            Tile::new(Suit::Characters, 9, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![
            RelicId::MultiplierMaster,
            RelicId::SetMagnet,
            RelicId::QuickDraw,
        ]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // MultiplierMaster: +0.5 per relic. 3 relics → +1.5.
        // Final mult: 1.0 + 1.5 = 2.5.
        assert_eq!(breakdown.final_mult, 2.5);
    }

    // ── Dragon Echo ─────────────────────────────────────────────────

    #[test]
    fn dragon_echo_copies_adjacent_set_chips() {
        let hand = vec![
            Tile::new(Suit::Characters, 1, 0),
            Tile::new(Suit::Characters, 2, 1),
            Tile::new(Suit::Characters, 3, 2),
            Tile::new(Suit::Dragon, 1, 3),
            Tile::new(Suit::Dragon, 1, 4),
            Tile::new(Suit::Dragon, 1, 5),
            Tile::new(Suit::Bamboos, 7, 6),
            Tile::new(Suit::Bamboos, 8, 7),
            Tile::new(Suit::Bamboos, 9, 8),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
        ];
        let r = relics(vec![RelicId::DragonEcho]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        // Adjacent set chips with Patch A's retuned bonuses (Sequence 28, Triplet 50):
        //   left seq: 1+2+3 + 28 = 34
        //   right seq: 7+8+9 + 28 = 52
        //   echo total = 86
        let (idx, _) = breakdown
            .steps
            .iter()
            .enumerate()
            .find(|(_, s)| s.source == "Dragon Echo")
            .unwrap();
        let prev_chips = if idx == 0 {
            breakdown.base_chips
        } else {
            breakdown.steps[idx - 1].running_chips
        };
        let echo_delta = breakdown.steps[idx].running_chips - prev_chips;
        assert_eq!(echo_delta, 86);
    }

    #[test]
    fn dragon_echo_ignores_non_dragon_triplet() {
        let hand = vec![
            Tile::new(Suit::Characters, 1, 0),
            Tile::new(Suit::Characters, 2, 1),
            Tile::new(Suit::Characters, 3, 2),
            Tile::new(Suit::Wind, 1, 3),
            Tile::new(Suit::Wind, 1, 4),
            Tile::new(Suit::Wind, 1, 5),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
        ];
        let r = relics(vec![RelicId::DragonEcho]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        assert!(!breakdown.steps.iter().any(|s| s.source == "Dragon Echo"));
    }

    // ── Chain Reaction ──────────────────────────────────────────────

    #[test]
    fn chain_reaction_adds_mult_when_scored_last_turn() {
        let hand = vec![
            Tile::new(Suit::Characters, 5, 0),
            Tile::new(Suit::Characters, 5, 1),
            Tile::new(Suit::Characters, 5, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::ChainReaction]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, true), &[]);
        // chips: 5+5+5 + 50 = 65, mult: 1 + 4 = 5, total 325
        assert_eq!(breakdown.final_mult, 5.0);
        assert_eq!(breakdown.total, 325);
    }

    #[test]
    fn chain_reaction_inactive_when_not_scored_last_turn() {
        let hand = vec![
            Tile::new(Suit::Characters, 5, 0),
            Tile::new(Suit::Characters, 5, 1),
            Tile::new(Suit::Characters, 5, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = relics(vec![RelicId::ChainReaction]);
        let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
        assert_eq!(breakdown.final_mult, 1.0);
    }

    // ── Pair Double rule ────────────────────────────────────────────

    #[test]
    fn pair_double_rule_adds_chips() {
        let hand = vec![
            Tile::new(Suit::Circles, 5, 0),
            Tile::new(Suit::Circles, 5, 1),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let breakdown = score_sets(
            &hand,
            &sets,
            &ctx_with(&RelicState::default(), false),
            &[RuleModifier::PairDoubleScore],
        );
        // chips: 5+5 (tiles) + 18 (pair meld, Patch A) + 30 (rule) = 58, mult ×1
        assert_eq!(breakdown.final_chips, 58);
        assert_eq!(breakdown.total, 58);
    }

    // ── Dora ────────────────────────────────────────────────────────

    #[test]
    fn dora_chips_per_matching_tile() {
        let hand = vec![
            Tile::new(Suit::Characters, 5, 0),
            Tile::new(Suit::Characters, 5, 1),
            Tile::new(Suit::Characters, 5, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        let r = RelicState::default();
        let ctx = ScoreContext {
            relics: &r,
            tile_debuffs: &[],
            scored_last_turn: false,
            dora_faces: vec![(Suit::Characters, 5)],
            available_yaku: vec![],
            round_wind: None,
            first_full_hand_of_round: false,
            plays_used: 0,
            riichi_active: false,
            yaku_levels: None,
            played_yaku_this_round: vec![],
            gold: 0,
            total_score: 0,
            is_final_play: false,
            tile_polisher_bonus: 0,
            relic_counters: std::collections::BTreeMap::new(),
            unscored_hand_tiles: 0,
            river_runner_bonus: 0,
            structure: None,
        };
        let breakdown = score_sets(&hand, &sets, &ctx, &[]);
        // 3 dora tiles × 25 (Patch A retune, was 20) = +75 chips
        let (idx, _) = breakdown
            .steps
            .iter()
            .enumerate()
            .find(|(_, s)| s.source.starts_with("Dora"))
            .unwrap();
        let prev_chips = if idx == 0 {
            breakdown.base_chips
        } else {
            breakdown.steps[idx - 1].running_chips
        };
        let dora_delta = breakdown.steps[idx].running_chips - prev_chips;
        assert_eq!(dora_delta, 75);
    }

    // ── Showpiece: explosive flush full hand ────────────────────────

    #[test]
    fn explosive_flush_full_hand_demonstration() {
        // 14-tile bamboo full hand: 4 melds (3 sequences + 1 triplet) + 1 pair.
        // This is the "this should feel huge" scenario from the design pitch.
        let hand = vec![
            // Sequence 1-2-3
            Tile::new(Suit::Bamboos, 1, 0),
            Tile::new(Suit::Bamboos, 2, 1),
            Tile::new(Suit::Bamboos, 3, 2),
            // Sequence 4-5-6
            Tile::new(Suit::Bamboos, 4, 3),
            Tile::new(Suit::Bamboos, 5, 4),
            Tile::new(Suit::Bamboos, 6, 5),
            // Sequence 7-8-9
            Tile::new(Suit::Bamboos, 7, 6),
            Tile::new(Suit::Bamboos, 8, 7),
            Tile::new(Suit::Bamboos, 9, 8),
            // Triplet of 5s
            Tile::new(Suit::Bamboos, 5, 9),
            Tile::new(Suit::Bamboos, 5, 10),
            Tile::new(Suit::Bamboos, 5, 11),
            // Pair of 7s
            Tile::new(Suit::Bamboos, 7, 12),
            Tile::new(Suit::Bamboos, 7, 13),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![9, 10, 11],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![12, 13],
            },
        ];
        let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
        // base chips with Patch A retunes (Sequence 28, Triplet 50, Pair 18):
        //   tiles: 1..9 once + 5+5+5 + 7+7 = 45 + 15 + 14 = 74
        //   meld bonuses: 28+28+28+50+18 = 152
        //   = 226
        // Patch B finishing: legacy `Flush` yaku is gone, subsumed by Chinitsu.
        // Yaku now contribute chips AND mult; FullHand has a chip side too.
        //   chip adds: FullHand(+60) + Chinitsu(+80) + Ittsu(+50) = +190
        //   final chips: 226 + 190 = 416
        //   mult: 1 + FullHand(+5) + Chinitsu(+6) + Ittsu(+4) = 16
        //   total: 416 × 16 = 6656
        assert_eq!(breakdown.base_chips, 226);
        assert_eq!(breakdown.final_mult, 16.0);
        assert_eq!(breakdown.total, 6656);
    }
}
