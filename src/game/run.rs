//! Single-run state: wall, hand, score target, round modifiers.
//!
//! Hand, selection, structure bank, and the fields mirrored by
//! [`crate::game::engine_state::GameplayCoreState`] should be mutated through
//! [`GameplayCoreState::with_run_mut`](crate::game::engine_state::GameplayCoreState::with_run_mut)
//! or [`crate::game::engine::GameEngine`] so `hand` and `selected` stay aligned.

pub mod discard_undo;

mod consumables;
mod hand_ops;
mod onboarding;
mod relic_removal;
mod round_flow;
mod scoring_flow;
mod tags;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::boss::{self, BossKind};
use crate::core::debuff::TileDebuff;
use crate::core::deck::Wall;
use crate::core::hand::{DetectedMeld, validate_selection_with_rules};
use crate::core::relic::{RelicId, RelicState};
use crate::core::rules::{BlindKind, RuleModifier};
use crate::core::scoring::ScoreBreakdown;
use crate::core::tile::{Suit, Tile, TileEnhancement};
use crate::core::yaku::YakuKind;
use crate::game::event_bus::{EventBus, GameEvent};
use crate::game::game_mode::GameMode;
use crate::game::onboarding::OnboardingState;
pub use discard_undo::DiscardUndoSnapshot;

/// Boss-blind state for the current run.  Extracted from `RunState` so
/// boss-specific logic has a single owner.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct BossState {
    /// Bosses still available for this run, drawn without replacement.
    pub pool_remaining: Vec<BossKind>,
    /// The boss for the current ante.
    pub upcoming: Option<BossKind>,
    /// Resolved effect for `upcoming`, rebuilt from the kind on load.
    #[serde(skip)]
    pub effect: Option<crate::core::boss::ResolvedBossEffect>,
    /// Per-round hand-size delta from boss effects.
    pub bonus_hand_size: i32,
    /// Gold cost charged after each successful play (set by The Tribute).
    pub gold_cost_per_play: u32,
    /// Per-play cost baked in by The Tax Collector at reveal time.
    #[serde(default)]
    pub tax_collector_cost: u32,
}

/// Result of consuming a slot from the shared consumable inventory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsumableUseResult {
    Zodiac {
        yaku: crate::core::yaku::YakuKind,
        new_level: u32,
    },
    Talisman {
        kind: crate::core::talisman::TalismanKind,
    },
    Memorial {
        kind: crate::core::memorial_talisman::MemorialTalismanKind,
    },
}

/// Defeating the Boss of this ante completes the run (Balatro-style).
pub const FINAL_ANTE: u32 = 7;

fn default_auto_cash_in_on_full_structure() -> bool {
    true
}

fn default_hints_enabled() -> bool {
    false
}

fn default_available_relics() -> Vec<RelicId> {
    crate::core::relic::all_relic_defs()
        .iter()
        .map(|def| def.id)
        .collect()
}

/// Paper Lantern–style shop pool swaps: once the primary relic has burned,
/// it cannot appear again this run and its successor becomes eligible.
#[derive(Clone, Copy, Debug, Default)]
pub struct RelicShopPoolExtinction {
    pub paper_lantern: bool,
    pub silk_thread: bool,
    pub melting_ice: bool,
    pub xxxl_egg: bool,
    pub tea_ceremony: bool,
    pub chrysalis: bool,
}

/// Silk Moth / Taotie / Geese / Stone Lantern: shop-only after the primary
/// burns **this run**; never carried in meta `available_relics`.
fn transformation_successor_shop_eligible(
    id: RelicId,
    available_relics: &[RelicId],
    ex: RelicShopPoolExtinction,
) -> bool {
    match id {
        RelicId::StoneLantern => {
            ex.paper_lantern && available_relics.contains(&RelicId::PaperLantern)
        }
        RelicId::SilkMoth => ex.silk_thread && available_relics.contains(&RelicId::SilkThread),
        RelicId::Taotie => ex.melting_ice && available_relics.contains(&RelicId::MeltingIce),
        RelicId::Geese => available_relics.contains(&RelicId::RustlingGooseEgg) && ex.xxxl_egg,
        RelicId::Rakuware => available_relics.contains(&RelicId::TeaCeremony) && ex.tea_ceremony,
        RelicId::MonarchButterfly => available_relics.contains(&RelicId::Chrysalis) && ex.chrysalis,
        _ => false,
    }
}

pub(crate) fn relic_eligible_for_shop_stock(
    id: RelicId,
    relics: &RelicState,
    available_relics: &[RelicId],
    ex: RelicShopPoolExtinction,
) -> bool {
    if relics.owns(id) {
        return false;
    }
    if crate::core::progression::is_transformation_successor_relic(id) {
        return transformation_successor_shop_eligible(id, available_relics, ex);
    }
    if !available_relics.contains(&id) {
        return false;
    }
    if id == RelicId::PaperLantern && ex.paper_lantern {
        return false;
    }
    if id == RelicId::SilkThread && ex.silk_thread {
        return false;
    }
    if id == RelicId::MeltingIce && ex.melting_ice {
        return false;
    }
    if id == RelicId::RustlingGooseEgg && ex.xxxl_egg {
        return false;
    }
    if id == RelicId::TeaCeremony && ex.tea_ceremony {
        return false;
    }
    if id == RelicId::Chrysalis && ex.chrysalis {
        return false;
    }
    true
}

#[derive(Clone, Copy)]
struct IndexedTile {
    hand_index: usize,
    tile: Tile,
}

fn enumerate_candidate_play_masks(hand: &[Tile], rules: &[RuleModifier]) -> Vec<u32> {
    let mut regular = Vec::with_capacity(hand.len());
    let mut flowers = Vec::new();
    for (hand_index, &tile) in hand.iter().enumerate() {
        let indexed = IndexedTile { hand_index, tile };
        if tile.is_flower() {
            flowers.push(indexed);
        } else {
            regular.push(indexed);
        }
    }
    regular.sort_by_key(|it| it.tile);
    flowers.sort_by_key(|it| it.tile);

    let subset_rules = PlayMaskRules {
        allow_wrap: rules.contains(&RuleModifier::SequenceWrap),
        no_sequences: rules.contains(&RuleModifier::NoSequences),
        require_honor: rules.contains(&RuleModifier::RequireHonor),
        must_play_five: rules.contains(&RuleModifier::MustPlayFive),
        no_flower_wildcards: rules.contains(&RuleModifier::NoFlowerWildcards),
    };

    let mut masks: rustc_hash::FxHashSet<u32> = rustc_hash::FxHashSet::default();
    enumerate_regular_subsets(&regular, &flowers, 0, subset_rules, 0, &mut masks);
    let mut out: Vec<u32> = masks.into_iter().collect();
    out.sort_unstable();
    out
}

#[derive(Clone, Copy)]
struct PlayMaskRules {
    allow_wrap: bool,
    no_sequences: bool,
    require_honor: bool,
    must_play_five: bool,
    no_flower_wildcards: bool,
}

fn enumerate_regular_subsets(
    remaining: &[IndexedTile],
    flowers: &[IndexedTile],
    current_mask: u32,
    rules: PlayMaskRules,
    current_tile_count: usize,
    out: &mut rustc_hash::FxHashSet<u32>,
) {
    let PlayMaskRules {
        allow_wrap,
        no_sequences,
        require_honor,
        must_play_five,
        no_flower_wildcards,
    } = rules;
    if current_tile_count > 14 || (must_play_five && current_tile_count > 5) {
        return;
    }

    if remaining.is_empty() {
        emit_leaf_masks(flowers, current_mask, current_tile_count, rules, out);
        return;
    }

    let first = remaining[0];
    enumerate_regular_subsets(
        &remaining[1..],
        flowers,
        current_mask,
        rules,
        current_tile_count,
        out,
    );

    if remaining.len() >= 2
        && same_face(first.tile, remaining[1].tile)
        && (!require_honor || tiles_have_honor(&[first.tile, remaining[1].tile]))
    {
        enumerate_regular_subsets(
            &remaining[2..],
            flowers,
            current_mask | (1 << first.hand_index) | (1 << remaining[1].hand_index),
            rules,
            current_tile_count + 2,
            out,
        );
    }

    if remaining.len() >= 3
        && same_face(first.tile, remaining[1].tile)
        && same_face(first.tile, remaining[2].tile)
        && (!require_honor || tiles_have_honor(&[first.tile, remaining[1].tile, remaining[2].tile]))
    {
        enumerate_regular_subsets(
            &remaining[3..],
            flowers,
            current_mask
                | (1 << first.hand_index)
                | (1 << remaining[1].hand_index)
                | (1 << remaining[2].hand_index),
            rules,
            current_tile_count + 3,
            out,
        );
    }

    if remaining.len() >= 4
        && same_face(first.tile, remaining[1].tile)
        && same_face(first.tile, remaining[2].tile)
        && same_face(first.tile, remaining[3].tile)
        && (!require_honor
            || tiles_have_honor(&[
                first.tile,
                remaining[1].tile,
                remaining[2].tile,
                remaining[3].tile,
            ]))
    {
        enumerate_regular_subsets(
            &remaining[4..],
            flowers,
            current_mask
                | (1 << first.hand_index)
                | (1 << remaining[1].hand_index)
                | (1 << remaining[2].hand_index)
                | (1 << remaining[3].hand_index),
            rules,
            current_tile_count + 4,
            out,
        );
    }

    let can_use_flower_wildcard = !flowers.is_empty() && !no_flower_wildcards;
    if can_use_flower_wildcard
        && remaining.len() >= 2
        && same_face(first.tile, remaining[1].tile)
        && (!require_honor || tiles_have_honor(&[first.tile, remaining[1].tile]))
    {
        for (flower_idx, flower) in flowers.iter().copied().enumerate() {
            enumerate_regular_subsets(
                &remaining[2..],
                &remove_flower(flowers, flower_idx),
                current_mask
                    | (1 << first.hand_index)
                    | (1 << remaining[1].hand_index)
                    | (1 << flower.hand_index),
                rules,
                current_tile_count + 3,
                out,
            );
        }
    }

    if !no_sequences && first.tile.is_number_tile() && !require_honor {
        for seq in sequence_candidates(remaining, allow_wrap, can_use_flower_wildcard, first) {
            let mut next_mask = current_mask | (1 << first.hand_index);
            let mut remove = vec![0usize];
            for idx in seq.regular_indices {
                next_mask |= 1 << remaining[idx].hand_index;
                remove.push(idx);
            }
            let rest = remove_indices(remaining, &remove);
            if seq.uses_flower {
                for (flower_idx, flower) in flowers.iter().copied().enumerate() {
                    enumerate_regular_subsets(
                        &rest,
                        &remove_flower(flowers, flower_idx),
                        next_mask | (1 << flower.hand_index),
                        rules,
                        current_tile_count + 3,
                        out,
                    );
                }
            } else {
                enumerate_regular_subsets(
                    &rest,
                    flowers,
                    next_mask,
                    rules,
                    current_tile_count + 3,
                    out,
                );
            }
        }
    }
}

fn emit_leaf_masks(
    flowers: &[IndexedTile],
    current_mask: u32,
    current_tile_count: usize,
    rules: PlayMaskRules,
    out: &mut rustc_hash::FxHashSet<u32>,
) {
    let must_play_five = rules.must_play_five;
    let round_rules = if rules.no_flower_wildcards {
        &[RuleModifier::NoFlowerWildcards][..]
    } else {
        &[][..]
    };
    for extra_mask in flower_meld_partition_masks(flowers, round_rules) {
        let total_mask = current_mask | extra_mask;
        let total_count = total_mask.count_ones() as usize;
        if total_count == 0 {
            continue;
        }
        if must_play_five {
            if total_count == 5 {
                out.insert(total_mask);
            }
        } else if total_count >= current_tile_count {
            out.insert(total_mask);
        }
    }
}

fn flower_meld_partition_masks(flowers: &[IndexedTile], rules: &[RuleModifier]) -> Vec<u32> {
    let indexed: Vec<(usize, u32)> = flowers.iter().map(|f| (f.hand_index, f.tile.id)).collect();
    crate::core::hand::decomposition::flower_meld_partition_masks(&indexed, rules)
}

fn remove_flower(flowers: &[IndexedTile], remove_idx: usize) -> Vec<IndexedTile> {
    flowers
        .iter()
        .enumerate()
        .filter_map(|(idx, flower)| (idx != remove_idx).then_some(*flower))
        .collect()
}

fn same_face(a: Tile, b: Tile) -> bool {
    a.suit == b.suit && a.rank == b.rank
}

fn tiles_have_honor(tiles: &[Tile]) -> bool {
    tiles
        .iter()
        .any(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
}

fn remove_indices(remaining: &[IndexedTile], remove: &[usize]) -> Vec<IndexedTile> {
    let mut remove_flags = vec![false; remaining.len()];
    for &idx in remove {
        remove_flags[idx] = true;
    }
    remaining
        .iter()
        .enumerate()
        .filter_map(|(idx, tile)| (!remove_flags[idx]).then_some(*tile))
        .collect()
}

#[derive(Clone, Copy)]
struct SequenceCandidate {
    regular_indices: [usize; 2],
    uses_flower: bool,
}

fn sequence_candidates(
    remaining: &[IndexedTile],
    allow_wrap: bool,
    can_use_flower: bool,
    first: IndexedTile,
) -> Vec<SequenceCandidate> {
    let mut out = Vec::new();
    push_sequence_candidate(
        remaining,
        first.tile.suit,
        [first.tile.rank + 1, first.tile.rank + 2],
        false,
        &mut out,
    );
    if can_use_flower {
        push_sequence_candidate(
            remaining,
            first.tile.suit,
            [first.tile.rank + 1],
            true,
            &mut out,
        );
        push_sequence_candidate(
            remaining,
            first.tile.suit,
            [first.tile.rank + 2],
            true,
            &mut out,
        );
    }
    if allow_wrap {
        for needs in wrap_sequence_needs(first.tile.rank) {
            push_sequence_candidate(remaining, first.tile.suit, *needs, false, &mut out);
        }
        if can_use_flower {
            for needs in wrap_sequence_needs(first.tile.rank) {
                push_sequence_candidate(remaining, first.tile.suit, [needs[0]], true, &mut out);
                push_sequence_candidate(remaining, first.tile.suit, [needs[1]], true, &mut out);
            }
        }
    }
    out
}

fn push_sequence_candidate(
    remaining: &[IndexedTile],
    suit: Suit,
    needed_ranks: impl AsRef<[u8]>,
    uses_flower: bool,
    out: &mut Vec<SequenceCandidate>,
) {
    let needed_ranks = needed_ranks.as_ref();
    let mut found = Vec::with_capacity(needed_ranks.len());
    for &rank in needed_ranks {
        let Some((idx, _)) = remaining.iter().enumerate().skip(1).find(|(_, tile)| {
            tile.tile.suit == suit && tile.tile.rank == rank && !found.contains(&tile.hand_index)
        }) else {
            return;
        };
        found.push(remaining[idx].hand_index);
    }

    let mut regular_indices = [0usize; 2];
    for (i, found_hand_index) in found.iter().enumerate() {
        let Some((remaining_idx, _)) = remaining
            .iter()
            .enumerate()
            .find(|(_, tile)| tile.hand_index == *found_hand_index)
        else {
            return;
        };
        regular_indices[i] = remaining_idx;
    }
    out.push(SequenceCandidate {
        regular_indices,
        uses_flower,
    });
}

fn wrap_sequence_needs(rank: u8) -> &'static [[u8; 2]] {
    match rank {
        1 => &[[2, 3], [9, 2], [8, 9]],
        2 => &[[3, 4], [1, 3], [9, 1]],
        3 => &[[4, 5], [2, 4], [1, 2]],
        4 => &[[5, 6], [3, 5], [2, 3]],
        5 => &[[6, 7], [4, 6], [3, 4]],
        6 => &[[7, 8], [5, 7], [4, 5]],
        7 => &[[8, 9], [6, 8], [5, 6]],
        8 => &[[9, 1], [7, 9], [6, 7]],
        9 => &[[8, 1], [1, 2], [7, 8]],
        _ => &[],
    }
}

fn structure_label_from_yaku(yaku: &[YakuKind]) -> String {
    if yaku.is_empty() {
        return "No Yaku".to_string();
    }
    yaku.iter()
        .map(|y| y.name())
        .collect::<Vec<_>>()
        .join(" + ")
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunState {
    pub wall: Wall,
    hand: Vec<Tile>,
    /// Which hand tiles are marked for discard (parallel with `hand`).
    selected: Vec<bool>,
    /// Melds committed from hand into the structure (deferred scoring until trigger).
    #[serde(default)]
    structure_sets: Vec<DetectedMeld>,
    /// Tile copies held in the structure (same ids as in `structure_sets`).
    #[serde(default)]
    structure_tiles: Vec<Tile>,
    pub round_score: u64,
    pub target_score: u32,
    pub base_target: u32,
    pub relics: RelicState,
    pub round_rules: Vec<RuleModifier>,
    pub run_number: u32,
    /// Current ante (1-indexed). Increments after defeating each Boss blind.
    pub ante: u32,
    pub plays_remaining: u32,
    /// Effective play peg row length for the current round after round-start
    /// modifiers (bosses, skip tags, relic taxes) have been applied.
    #[serde(default)]
    pub plays_max: u32,
    pub discards_remaining: u32,
    /// Effective discard peg row length for the current round after round-start
    /// modifiers (bosses, skip tags, relic taxes) have been applied.
    #[serde(default)]
    pub discards_max: u32,
    pub gold: i32,
    pub blind: BlindKind,
    /// Next blind the player will face in the Small→Big→Boss cycle.
    pub upcoming_blind: BlindKind,
    /// Last scoring breakdown for UI cascade display. Not persisted across
    /// quit/resume — the cascade is a transient UI artifact, not run state.
    #[serde(skip)]
    pub last_breakdown: Option<ScoreBreakdown>,
    /// Yaku available at the player's progression level.
    pub available_yaku: Vec<crate::core::yaku::YakuKind>,
    /// Rules available at the player's progression level.
    pub available_rules: Vec<RuleModifier>,
    /// Relics that were unlocked when this run started. Shop rolls and
    /// random relic rewards use this snapshot so new unlocks wait until
    /// the next game.
    #[serde(default = "default_available_relics")]
    pub available_relics: Vec<RelicId>,
    /// Whether the player scored on their last play (for ChainReaction relic).
    pub scored_last_turn: bool,
    /// Whether JokerTile was used this round.
    pub joker_used: bool,
    /// Whether the player has scored a FullHand yaku this round (e.g. Eight
    /// Treasures triggers on the first FullHand of the round).
    pub full_hand_played_this_round: bool,
    /// Per-yaku level (default 1). Incremented by Zodiac card use.
    pub yaku_levels: crate::core::zodiac::YakuLevels,
    /// Player's shared consumable inventory — holds both Zodiacs and Talismans
    /// in the same capped slot list. Capacity expands via the Brocade Pouch relic.
    pub consumables: crate::core::consumable::ConsumableInventory,
    /// Game mode preset used for this run (drives advance_round resets).
    pub mode: GameMode,
    /// When true, automatically cash in the structure as soon as it reaches
    /// a full valid shape.
    #[serde(default = "default_auto_cash_in_on_full_structure")]
    pub auto_cash_in_on_full_structure: bool,
    #[serde(default = "default_hints_enabled")]
    pub hints_enabled: bool,
    // ── Boss blind state ─────────────────────────────────────────────────
    #[serde(flatten)]
    pub boss: BossState,
    /// Yaku detected on prior plays in this round. Used by The Censor to
    /// halve repeat-yaku contributions. Reset on round start.
    #[serde(skip)]
    pub played_yaku_this_round: Vec<crate::core::yaku::YakuKind>,
    /// Round-scoped tile debuffs applied by the active boss.
    #[serde(default)]
    pub tile_debuffs: Vec<TileDebuff>,
    /// Set true the first time this round a scored hand contains a Wind
    /// or Dragon tile. Powers Green Luck's per-round payout — the relic
    /// awards its bonus at round clear iff this stays false. Reset by
    /// `advance_round` and `skip_to_next_blind`.
    #[serde(skip)]
    pub honors_scored_this_round: bool,
    /// Second round wind chosen by Windreader at blind start (1–4).
    #[serde(default)]
    pub windreader_bonus_wind: Option<u8>,
    /// Cumulative score earned across the entire run (metrics / Golden Engine).
    #[serde(default)]
    pub total_score_earned: u64,
    /// True once Paper Lantern has burned up this run. Prevents Paper from
    /// reappearing in shops and unlocks Stone Lantern in the shop pool.
    #[serde(default)]
    pub paper_lantern_extinct: bool,
    /// Silk Thread burned this run — slot emptied; Silk Moth can appear in shops.
    #[serde(default)]
    pub silk_thread_extinct: bool,
    /// Melting Ice burned this run — Taotie can appear in shops.
    #[serde(default)]
    pub melting_ice_extinct: bool,
    /// XXXL Egg burned this run — Geese returns to the shop pool (when the egg is meta-unlocked).
    #[serde(default)]
    pub xxxl_egg_extinct: bool,
    /// Tea Ceremony completed this run (became Rakuware) — Rakuware can appear in shops.
    #[serde(default, rename = "tea_to_raku_extinct")]
    pub tea_ceremony_extinct: bool,
    /// Chrysalis hatched this run — Monarch Butterfly can appear in shops.
    #[serde(default)]
    pub chrysalis_extinct: bool,
    /// Per-yaku cumulative play counter for the entire run. Powers the
    /// Yaku Journal overlay's "Played N×" line. Persisted across save/load
    /// (defaults to empty for old saves).
    #[serde(default)]
    pub yaku_times_played: rustc_hash::FxHashMap<crate::core::yaku::YakuKind, u32>,
    /// Cumulative tiles committed from hand into the structure bank (melds).
    #[serde(default)]
    pub tiles_played: u32,
    /// Cumulative tiles thrown away via the discard action.
    #[serde(default)]
    pub tiles_discarded: u32,
    /// Number of times the hand was replenished after spending tiles.
    #[serde(default)]
    pub times_restocked: u32,
    /// Highest chip total from a single structure cash-in.
    #[serde(default)]
    pub best_structure_score: u64,
    /// Display label for the highest-scoring structure.
    #[serde(default)]
    pub best_structure_name: String,
    /// Tiles from the highest-scoring structure cash-in (for chronicle display).
    #[serde(default)]
    pub best_hand_tiles: Vec<Tile>,
    /// Cumulative run score snapshot after each boss-cleared ante.
    #[serde(default)]
    pub score_after_ante: Vec<(u32, u64)>,
    /// Per-run tile enhancement map, keyed by tile id. Talismans stamp every
    /// hand tile's id into this map; whenever tiles are drawn (initial deal,
    /// post-play refill, mid-round draws, new-round redeals), we re-apply the
    /// stored enhancement so it persists for the rest of the run. Tile ids are
    /// stable across walls because `build_wall` assigns them deterministically
    /// (so id 5 is always the same suit+rank, even after a reshuffle).
    #[serde(default)]
    pub tile_enhancements: BTreeMap<u32, TileEnhancement>,
    /// When Brocade Pouch is owned, the last-used buff talisman's enhancement
    /// is recorded here and stamped onto every drawn tile (not just the 14
    /// in hand at use time). Per-tile entries in `tile_enhancements` still win
    /// so packs and prior-stamped tiles keep their specific enhancement.
    #[serde(default)]
    pub global_buff_enhancement: Option<TileEnhancement>,
    /// Tile IDs permanently removed from the wall (e.g. Taotie devour).
    /// Filtered out during wall construction each round.
    #[serde(default)]
    pub removed_tile_ids: rustc_hash::FxHashSet<u32>,
    /// Tile packs purchased from the shop. Each pack permanently injects
    /// extra tiles into the wall every round. Append-only.
    #[serde(default)]
    pub tile_packs: Vec<crate::core::tile_pack::TilePackKind>,

    // ── Skip-reward tags ──────────────────────────────────────────────
    /// Tag assigned to the Small blind this ante.
    #[serde(default)]
    pub small_blind_tag: Option<crate::core::tag::TagKind>,
    /// Tag assigned to the Big blind this ante.
    #[serde(default)]
    pub big_blind_tag: Option<crate::core::tag::TagKind>,
    /// Tag-granted: next shop's first reroll is free.
    #[serde(default)]
    pub tag_free_reroll: bool,
    /// Tag-granted: one random relic in the next shop costs 0.
    #[serde(default)]
    pub tag_patron_gift: bool,
    /// Tag-granted: next shop stocks 2 extra relics.
    #[serde(default)]
    pub tag_rich_stock: bool,
    /// Tag-granted bonus plays for the next round.
    #[serde(default)]
    pub tag_bonus_plays: u32,
    /// Tag-granted bonus discards for the next round.
    #[serde(default)]
    pub tag_bonus_discards: u32,
    /// Tag-granted bonus hand size for the next round.
    #[serde(default)]
    pub tag_bonus_hand_size: i32,
    /// Pending zodiac activations from a ZodiacBlessing skip-reward tag.
    /// Consumed one at a time by the pick-blind scene for celebration overlays.
    #[serde(skip)]
    pub pending_zodiac_celebrations: Vec<(
        crate::core::zodiac::ZodiacKind,
        crate::core::yaku::YakuKind,
        u32,
    )>,
    /// Data from a dismissed zodiac celebration overlay, consumed by the
    /// underlying scene (shop) to spawn a score popup + particle burst.
    #[serde(skip)]
    pub finished_zodiac_celebration: Option<(&'static str, u32)>,
    /// Set when a shop celebration overlay (tile pack or zodiac level-up) pops;
    /// [`ShopScene`] consumes it once to refocus the shelf so the player isn't
    /// stuck with focus pointing at the now-removed item.
    #[serde(skip)]
    pub pending_shop_focus_snap_after_celebration: bool,
    /// Per-relic mutable counters. Key is RelicId, value meaning depends
    /// on the relic:
    ///   Humility     → consecutive plays without honor tiles
    ///   Temperance   → permanent mult stack ×10 (+0.5 mult per unused play on blind clear)
    ///   Obsession    → rounds without most-used yaku
    ///   Bonfire      → relics sold this run
    ///   MeltingIce   → remaining chip bonus (starts 80, -8 per play)
    ///   SilkThread   → remaining mult ×10 (starts 40, -3 per discard)
    ///   NestEgg      → rounds held (sell value grows)
    ///   XXXL Egg → plays remaining before burning (Geese shop unlock)
    ///   TeaCeremony  → principle index 0–3 (four scored hands, then transforms)
    ///   Rakuware     → (no counter; all four Tea beats when conditions hold)
    ///   MonarchButterfly → cumulative absorbed excess (post-target); tiers for chip bonus
    ///   HungryGhost  → permanent mult bonus ×10
    ///   TilePolisher → accumulated +chip bonus (each scored tile +3)
    ///   RiverRunner  → accumulated +chip bonus (each scored sequence +20)
    ///   IGotAGuy     → shop restock waivers remaining (starts 3 on buy)
    #[serde(default)]
    pub relic_counters: std::collections::BTreeMap<RelicId, i32>,
    /// Curated onboarding-campaign state for the revamped first-time tutorial.
    #[serde(default)]
    pub onboarding: Option<OnboardingState>,

    /// Relics whose effects just fired this frame. Scenes drain this each
    /// frame to drive glow + wiggle animations. Populated by `run.rs`
    /// methods whenever a relic triggers (scoring, round-end, discard,
    /// draw, consumable interaction, etc.).
    #[serde(skip)]
    pub relic_activations: Vec<RelicId>,

    /// Per-run analytics for the Archive Chronicle ledger.
    #[serde(default)]
    pub chronicle: crate::core::run_chronicle::RunChronicle,

    /// Habits tracked for memorial remnant selection at defeat.
    #[serde(default)]
    pub defeat_journal: crate::core::memorial_talisman::RunDefeatJournal,
    /// True once the run-start memorial grant has been applied.
    #[serde(default)]
    pub memorial_granted: bool,
    /// Journal snapshot from the previous defeat (use-time scaling / flavor).
    #[serde(default)]
    pub memorial_snapshot: Option<crate::core::memorial_talisman::MemorialJournalSnapshot>,
    /// In-round bonuses from memorial use; cleared each blind.
    #[serde(skip)]
    pub memorial_round: crate::core::memorial_talisman::MemorialRoundState,
    /// Remnant chosen at defeat (for run history / debug).
    #[serde(skip)]
    pub defeat_memorial_kind: Option<crate::core::memorial_talisman::MemorialTalismanKind>,
}

impl RunState {
    /// Remaining gold-free shop restocks from [RelicId::IGotAGuy]. Zero if not owned.
    pub(crate) fn i_got_a_guy_restock_charges(&self) -> i32 {
        if !self.relics.has(RelicId::IGotAGuy) {
            return 0;
        }
        self.relic_counters
            .get(&RelicId::IGotAGuy)
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn can_afford_shop_reroll(&self, reroll_cost: u32) -> bool {
        reroll_cost == 0
            || self.gold >= reroll_cost as i32
            || self.i_got_a_guy_restock_charges() > 0
    }

    pub fn hand(&self) -> &[Tile] {
        &self.hand
    }

    pub(crate) fn hand_mut(&mut self) -> &mut Vec<Tile> {
        &mut self.hand
    }

    pub fn selected_slice(&self) -> &[bool] {
        &self.selected
    }

    /// Ghost Hand HUD / tooltip preview: the next cash-in scores all tiles still
    /// in hand as "unscored" (chips from tiles not yet committed to structure).
    pub fn ghost_hand_preview_chips(&self) -> i32 {
        let hand = self.hand();
        let debuffs = &self.tile_debuffs;
        hand.iter()
            .filter(|t| !debuffs.iter().any(|d| d.matches(t)))
            .map(|t| t.point_value() as i32)
            .sum()
    }

    pub(crate) fn selected_mut(&mut self) -> &mut Vec<bool> {
        &mut self.selected
    }

    pub fn structure_sets(&self) -> &[DetectedMeld] {
        &self.structure_sets
    }

    pub(crate) fn structure_sets_mut(&mut self) -> &mut Vec<DetectedMeld> {
        &mut self.structure_sets
    }

    pub fn structure_tiles(&self) -> &[Tile] {
        &self.structure_tiles
    }

    pub(crate) fn structure_tiles_mut(&mut self) -> &mut Vec<Tile> {
        &mut self.structure_tiles
    }

    pub(crate) fn set_gameplay_core_slice(
        &mut self,
        hand: Vec<Tile>,
        selected: Vec<bool>,
        structure_sets: Vec<DetectedMeld>,
        structure_tiles: Vec<Tile>,
    ) {
        self.hand = hand;
        self.selected = selected;
        self.structure_sets = structure_sets;
        self.structure_tiles = structure_tiles;
    }

    fn discard_cap_for(mode: &GameMode, relics: &RelicState) -> u32 {
        let mut d = mode.starting_discards;
        if relics.has(crate::core::relic::RelicId::BigHands) {
            d = d.saturating_sub(1);
        }
        if relics.has(crate::core::relic::RelicId::TinyHands) {
            d = d.saturating_add(2);
        }
        if relics.has(crate::core::relic::RelicId::Kindness) {
            d = d.saturating_add(1);
        }
        d
    }

    fn play_cap_for(mode: &GameMode, relics: &RelicState) -> u32 {
        let mut p = mode.starting_plays;
        if relics.has(crate::core::relic::RelicId::Diligence) {
            p = p.saturating_add(1);
        }
        p
    }

    fn round_play_cap(&self) -> u32 {
        Self::play_cap_for(&self.mode, &self.relics)
    }

    fn round_discard_cap(&self) -> u32 {
        Self::discard_cap_for(&self.mode, &self.relics)
    }

    pub fn relic_shop_pool_extinction(&self) -> RelicShopPoolExtinction {
        RelicShopPoolExtinction {
            paper_lantern: self.paper_lantern_extinct,
            silk_thread: self.silk_thread_extinct,
            melting_ice: self.melting_ice_extinct,
            xxxl_egg: self.xxxl_egg_extinct,
            tea_ceremony: self.tea_ceremony_extinct,
            chrysalis: self.chrysalis_extinct,
        }
    }

    /// Debug / cheat: act as if every fragile primary in the transform chains
    /// has already burned this run so successors become shop-eligible.
    pub fn cheat_force_all_transform_extinctions(&mut self) {
        self.paper_lantern_extinct = true;
        self.silk_thread_extinct = true;
        self.melting_ice_extinct = true;
        self.xxxl_egg_extinct = true;
        self.tea_ceremony_extinct = true;
        self.chrysalis_extinct = true;
    }

    /// Canonical *relic destroyed* trigger.
    ///
    /// The "destroyed" keyword is the player-facing name for permanent removal
    /// of a relic from a run; Kintsugi converts each invocation into a permanent
    /// +1 mult via its counter — skipping this after a qualifying removal breaks
    /// that synergy.
    ///
    /// Inventory teardown is handled by [`Self::destroy_relic_removed_from_run`],
    /// which clears debuffs/counters for `relic_id`, removes it from
    /// [`RelicState::active`], then calls this.
    ///
    /// Call this directly only when `relics.active` was already updated (in-slot
    /// transforms such as Tea Ceremony → Rakuware or Chrysalis → Monarch Butterfly,
    /// or the Hungry Ghost victim after `active.remove`) — still exactly once per
    /// qualifying destruction. Successors enter the shop pool via
    /// [`RelicShopPoolExtinction`] where applicable.
    pub(crate) fn note_relic_destroyed(&mut self) {
        if self.relics.has(crate::core::relic::RelicId::Kintsugi) {
            *self
                .relic_counters
                .entry(crate::core::relic::RelicId::Kintsugi)
                .or_insert(0) += 1;
            self.push_relic_activation(crate::core::relic::RelicId::Kintsugi);
        }
    }

    /// Permanent removal of `relic_id` from the run inventory (slot emptied).
    ///
    /// Clears [`RelicState::debuffed`] and [`RunState::relic_counters`] entries keyed
    /// by `relic_id`, removes all copies from [`RelicState::active`], then
    /// [`Self::note_relic_destroyed`] for Kintsugi.
    ///
    /// Does **not** push [`GameEvent`]s, set shop extinction flags, or append to
    /// [`RunState::relic_activations`] for `relic_id` — callers keep those semantics.
    ///
    /// In-slot transforms (Tea Ceremony → Rakuware, Chrysalis → Monarch Butterfly)
    /// must **not** use this; swap the active entry, then call [`Self::note_relic_destroyed`]
    /// alone.
    ///
    /// Returns whether at least one copy was present (and removed).
    pub(crate) fn destroy_relic_removed_from_run(&mut self, relic_id: RelicId) -> bool {
        if !self.relics.active.contains(&relic_id) {
            return false;
        }
        self.relics.active.retain(|&r| r != relic_id);
        self.relics.debuffed.remove(&relic_id);
        self.relic_counters.remove(&relic_id);
        self.note_relic_destroyed();
        true
    }

    /// Apply a signed gold change (shop spend, boss tax). Balance may go negative.
    pub(crate) fn apply_gold_delta(&mut self, delta: i32, bus: Option<&mut EventBus>) {
        if delta == 0 {
            return;
        }
        self.gold += delta;
        self.notify_run_gold_changed(delta, bus);
    }

    /// Apply a non-negative gold gain with saturation at `i32::MAX`.
    pub(crate) fn apply_gold_reward(&mut self, delta: i32, bus: Option<&mut EventBus>) {
        if delta <= 0 {
            if delta < 0 {
                self.apply_gold_delta(delta, bus);
            }
            return;
        }
        let old = self.gold;
        self.gold = self.gold.saturating_add(delta);
        let applied = self.gold - old;
        if applied != 0 {
            self.chronicle.note_gold_earned(applied);
            self.notify_run_gold_changed(applied, bus);
        }
    }

    /// Record a relic activation for animation and Chronicle totals.
    #[inline]
    pub(crate) fn push_relic_activation(&mut self, id: crate::core::relic::RelicId) {
        self.relic_activations.push(id);
        self.chronicle.note_relic_trigger();
    }

    /// Set run gold to an absolute value (debug / tooling). Emits the net delta.
    pub(crate) fn set_run_gold_direct(&mut self, new_gold: i32, bus: Option<&mut EventBus>) {
        let old = self.gold;
        if new_gold == old {
            return;
        }
        self.gold = new_gold;
        self.notify_run_gold_changed(new_gold - old, bus);
    }

    /// Push [`GameEvent::GoldChanged`] and run gold-reactive relic hooks.
    pub(crate) fn notify_run_gold_changed(&mut self, delta: i32, mut bus: Option<&mut EventBus>) {
        if delta == 0 {
            return;
        }
        if let Some(b) = bus.as_mut() {
            b.push(GameEvent::GoldChanged { delta });
        }
        self.relic_hooks_on_run_gold_changed(bus);
    }

    /// Extensible hook for relics that care about the bank after any gold mutation.
    fn relic_hooks_on_run_gold_changed(&mut self, bus: Option<&mut EventBus>) {
        self.turtle_shell_on_gold_broke(bus);
    }

    fn turtle_shell_on_gold_broke(&mut self, bus: Option<&mut EventBus>) {
        if self.gold > 0 {
            return;
        }
        if !self.relics.has(RelicId::TurtleShell) {
            return;
        }
        let _ = self.destroy_relic_removed_from_run(RelicId::TurtleShell);
        self.push_relic_activation(RelicId::TurtleShell);
        if let Some(bus) = bus {
            bus.push(GameEvent::RelicActivated(RelicId::TurtleShell));
        }
    }

    /// Bonus round wind from Windreader, if the relic is owned this run.
    pub fn bonus_round_wind_for_yaku(&self) -> Option<u8> {
        if self.relics.has(RelicId::WindReader) {
            self.windreader_bonus_wind
        } else {
            None
        }
    }

    pub(crate) fn refresh_windreader_bonus_wind(&mut self) {
        self.windreader_bonus_wind = if self.relics.has(RelicId::WindReader) {
            Some(crate::core::rules::BlindKind::roll_bonus_round_wind_for_ante(self.ante))
        } else {
            None
        };
    }

    /// Rebuild round-start plays/discards from current mode + permanent
    /// round modifiers. This keeps all "per round" bonuses sourced from one
    /// place so shop-bought relics and material bonuses apply on the next blind.
    fn reset_round_resources(&mut self) {
        self.plays_remaining = self.round_play_cap();
        self.discards_remaining = self.round_discard_cap();
        self.sync_round_resource_caps();
    }

    fn sync_round_resource_caps(&mut self) {
        self.plays_max = self.plays_remaining;
        self.discards_max = self.discards_remaining;
    }

    /// Consume skip-tag bonuses that apply to the next blind only.
    fn apply_pending_round_resource_bonuses(&mut self) {
        if self.tag_bonus_plays > 0 {
            self.plays_remaining += self.tag_bonus_plays;
            self.tag_bonus_plays = 0;
        }
        if self.tag_bonus_discards > 0 {
            self.discards_remaining += self.tag_bonus_discards;
            self.tag_bonus_discards = 0;
        }
    }

    /// Fresh run for the given mode.
    ///
    /// The wall is shuffled and the opening hand is drawn from it, and the first
    /// boss is picked with RNG — nothing about the deal is reproducible across
    /// calls, machines, or CI. For tests, screenshots, or anything that assumes a
    /// fixed hand, set `hand` (and any other fields you care about) explicitly
    /// after construction or use a dedicated test fixture.
    pub fn new(mode: GameMode) -> Self {
        let mut wall = Wall::from_standard_shuffled();

        let mut relics = RelicState::default();
        for &r in &mode.starting_relics {
            if !relics.is_full() {
                relics.active.push(r);
            }
        }

        let hand_size = boss::effective_hand_size_components(mode.hand_size, 0, &relics);
        let mut hand = Vec::with_capacity(hand_size);
        for _ in 0..hand_size {
            if let Some(t) = wall.draw() {
                hand.push(t);
            }
        }
        hand.sort();
        let selected = vec![false; hand.len()];

        let mut boss_pool_remaining = boss::regular_pool();
        let mut rng = rand::rng();
        let boss_floor = mode.stake.boss_min_ante_floor();
        let upcoming_boss =
            boss::pick_for_ante_with_floor(&mut boss_pool_remaining, 1, boss_floor, &mut rng);

        let starting_discards = Self::discard_cap_for(&mode, &relics);
        let starting_plays = Self::play_cap_for(&mode, &relics);
        let mut state = Self {
            wall,
            hand,
            selected,
            structure_sets: Vec::new(),
            structure_tiles: Vec::new(),
            round_score: 0,
            target_score: crate::core::blind_target::score_for(
                1,
                BlindKind::Small,
                mode.base_target,
            ),
            base_target: mode.base_target,
            relics,
            round_rules: mode.starting_rules.clone(),
            run_number: 1,
            ante: 1,
            plays_remaining: starting_plays,
            plays_max: starting_plays,
            discards_remaining: starting_discards,
            discards_max: starting_discards,
            gold: mode.starting_gold as i32,
            blind: BlindKind::Small,
            upcoming_blind: BlindKind::Small,
            last_breakdown: None,
            available_yaku: mode.starting_yaku.clone(),
            available_rules: mode.starting_rules.clone(),
            available_relics: default_available_relics(),
            scored_last_turn: false,
            joker_used: false,
            full_hand_played_this_round: false,
            yaku_levels: crate::core::zodiac::YakuLevels::default(),
            consumables: crate::core::consumable::ConsumableInventory {
                items: Vec::new(),
                capacity: mode.consumable_capacity,
            },
            mode,
            auto_cash_in_on_full_structure: true,
            hints_enabled: false,
            boss: BossState {
                pool_remaining: boss_pool_remaining,
                upcoming: upcoming_boss,
                effect: None,
                bonus_hand_size: 0,
                gold_cost_per_play: 0,
                tax_collector_cost: 0,
            },
            played_yaku_this_round: Vec::new(),
            tile_debuffs: Vec::new(),
            honors_scored_this_round: false,
            windreader_bonus_wind: None,
            total_score_earned: 0,
            paper_lantern_extinct: false,
            silk_thread_extinct: false,
            melting_ice_extinct: false,
            xxxl_egg_extinct: false,
            tea_ceremony_extinct: false,
            chrysalis_extinct: false,
            yaku_times_played: rustc_hash::FxHashMap::default(),
            tiles_played: 0,
            tiles_discarded: 0,
            times_restocked: 0,
            best_structure_score: 0,
            best_structure_name: String::new(),
            best_hand_tiles: Vec::new(),
            score_after_ante: Vec::new(),
            tile_enhancements: BTreeMap::new(),
            global_buff_enhancement: None,
            removed_tile_ids: rustc_hash::FxHashSet::default(),
            tile_packs: Vec::new(),
            small_blind_tag: None,
            big_blind_tag: None,
            tag_free_reroll: false,
            tag_patron_gift: false,
            tag_rich_stock: false,
            tag_bonus_plays: 0,
            tag_bonus_discards: 0,
            tag_bonus_hand_size: 0,
            pending_zodiac_celebrations: Vec::new(),
            finished_zodiac_celebration: None,
            pending_shop_focus_snap_after_celebration: false,
            relic_counters: std::collections::BTreeMap::new(),
            onboarding: None,
            relic_activations: Vec::new(),
            chronicle: {
                let seed = rand::random::<u64>();
                let started_unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                crate::core::run_chronicle::RunChronicle::new_run(seed, started_unix)
            },
            defeat_journal: crate::core::memorial_talisman::RunDefeatJournal::default(),
            memorial_granted: false,
            memorial_snapshot: None,
            memorial_round: crate::core::memorial_talisman::MemorialRoundState::default(),
            defeat_memorial_kind: None,
        };
        // Roll skip-reward tags for ante 1.
        state.roll_ante_tags();
        // Resolve the first ante's boss now so reactive variants are baked
        // in before pick_blind ever reads `upcoming_boss_effect`.
        state.resolve_upcoming_boss();
        // No-op for a fresh run (empty enhancement map), but kept here so the
        // invariant "hand always reflects tile_enhancements" holds uniformly.
        state.restamp_hand_enhancements();
        state
    }

    pub fn set_auto_cash_in_on_full_structure(&mut self, enabled: bool) {
        self.auto_cash_in_on_full_structure = enabled;
    }

    pub fn set_hints_enabled(&mut self, enabled: bool) {
        self.hints_enabled = enabled;
    }

    pub fn apply_progression(&mut self, progress: &crate::core::progression::PlayerProgress) {
        self.available_yaku = progress.available_yaku();
        self.available_rules = progress.available_rules();
        self.available_relics = progress.available_relics();
    }

    /// Grant the memorial remnant carried over from the last defeat (once per run).
    pub fn grant_pending_memorial(&mut self, progress: &mut crate::core::progression::PlayerProgress) {
        if self.memorial_granted {
            return;
        }
        let Some(kind) = progress.pending_memorial.take() else {
            return;
        };
        self.memorial_snapshot = progress.pending_memorial_journal.take();
        let _ = self
            .consumables
            .try_push(crate::core::consumable::Consumable::Memorial(kind));
        self.memorial_granted = true;
        progress.memorials_discovered.insert(kind);
    }

    /// Build the `ResolvedBossEffect` for the current `upcoming_boss`. For
    /// static bosses this is a thin wrap of `BossDef::effect`. For reactive
    /// bosses (those with an `on_reveal` hook), the hook runs against the
    /// current `RunState` and produces a tailored effect that's locked in
    /// for the rest of the ante. Idempotent — safe to call from
    /// `RunState::new`, `advance_round`, and the save-load rehydrate path.
    pub fn resolve_upcoming_boss(&mut self) {
        use crate::core::boss::ResolvedBossEffect;
        // Reset any reactive scratch — the new boss may not need it.
        self.boss.tax_collector_cost = 0;
        let Some(kind) = self.boss.upcoming else {
            self.boss.effect = None;
            return;
        };
        let def = kind.def();
        // Two-step to keep borrows clean: pull `on_reveal` out as a
        // function pointer (Copy), then call it with `&mut *self`.
        let effect = match def.on_reveal {
            Some(hook) => hook(self),
            None => ResolvedBossEffect::from_static(&def.effect),
        };
        self.boss.effect = Some(effect);
    }

    /// [`Self::new`] with [`GameMode::standard`](crate::game::game_mode::GameMode::standard).
    /// The initial hand is **not** deterministic; see [`Self::new`].
    pub fn new_demo() -> Self {
        Self::new(GameMode::standard())
    }

    /// Start a new run with the given tile material set.
    pub fn new_with_material(material: crate::persistence::TileMaterial) -> Self {
        Self::new(GameMode::with_material(material))
    }

    /// Factory that threads a difficulty stake into the game mode at run
    /// start. Spring produces the same result as `new_with_material`.
    pub fn new_with_material_and_stake(
        material: crate::persistence::TileMaterial,
        stake: crate::core::stake::Stake,
    ) -> Self {
        Self::new(GameMode::with_material_and_stake(material, stake))
    }

    /// Score target for a blind at the current ante (see `core::blind_target`).
    pub fn blind_score_target(&self, blind: crate::core::rules::BlindKind) -> u32 {
        crate::core::blind_target::score_for(self.ante, blind, self.base_target)
    }

    /// Whether a run is in progress (not a fresh/default state).
    pub fn is_in_progress(&self) -> bool {
        self.round_score > 0 || self.run_number > 1 || self.gold != self.mode.starting_gold as i32
    }

    /// True once the player has defeated the Boss of the final ante.
    pub fn is_run_complete(&self) -> bool {
        self.ante > FINAL_ANTE
    }

    /// Set Magnet: after any draw phase, for each face with exactly 3 copies
    /// in hand, pull the 4th matching tile from the wall.
    fn set_magnet_draw_fourths(&mut self, bus: &mut EventBus) {
        if !self.relics.has(RelicId::SetMagnet) {
            return;
        }
        // Count copies of each (suit, rank) face currently in hand.
        let mut counts: rustc_hash::FxHashMap<(crate::core::tile::Suit, u8), u32> =
            rustc_hash::FxHashMap::default();
        for t in &self.hand {
            *counts.entry((t.suit, t.rank)).or_insert(0) += 1;
        }
        for ((suit, rank), count) in counts {
            if count == 3
                && let Some(matching) = self.wall.draw_matching(suit, rank)
            {
                self.hand.push(matching);
                self.selected.push(false);
                bus.push(GameEvent::TileDrawn);
                self.push_relic_activation(RelicId::SetMagnet);
                self.chronicle.note_tiles_drawn(1);
            }
        }
    }
}

/// All tile faces for substitution attempts.
const ALL_FACES: [(Suit, u8); 34] = {
    let mut faces = [(Suit::Manzu, 0u8); 34];
    let mut i = 0;
    let suits = [Suit::Manzu, Suit::Souzu, Suit::Pinzu];
    let mut si = 0;
    while si < 3 {
        let mut r = 1u8;
        while r <= 9 {
            faces[i] = (suits[si], r);
            i += 1;
            r += 1;
        }
        si += 1;
    }
    let mut r = 1u8;
    while r <= 4 {
        faces[i] = (Suit::Wind, r);
        i += 1;
        r += 1;
    }
    r = 1;
    while r <= 3 {
        faces[i] = (Suit::Dragon, r);
        i += 1;
        r += 1;
    }
    faces
};

/// Try substituting one tile with every possible face (JokerTile).
fn try_joker_substitution(
    tiles: &[Tile],
    rules: &[RuleModifier],
) -> Option<(Vec<DetectedMeld>, Vec<Tile>)> {
    for (idx, _) in tiles.iter().enumerate() {
        for &(suit, rank) in &ALL_FACES {
            let mut modified = tiles.to_vec();
            modified[idx] = Tile::new(suit, rank, modified[idx].id);
            if let Some(sets) = validate_selection_with_rules(&modified, rules) {
                return Some((sets, modified));
            }
        }
    }
    None
}

/// Build the set of faces a wild wind tile could usefully become:
/// - Any face already in `tiles` (for pairs/triplets)
/// - Any numbered face within ±2 rank of a same-suit numbered tile (for sequences)
fn wind_candidate_faces(tiles: &[Tile]) -> Vec<(Suit, u8)> {
    use std::collections::BTreeSet;
    let mut candidates = BTreeSet::new();
    let number_suits = [Suit::Manzu, Suit::Souzu, Suit::Pinzu];
    for t in tiles {
        // Exact face: could pair/triplet with existing tiles.
        candidates.insert((t.suit, t.rank));
        // Nearby ranks in numbered suits: could form a sequence.
        if number_suits.contains(&t.suit) {
            for delta in [-2i8, -1, 1, 2] {
                let r = t.rank as i8 + delta;
                if (1..=9).contains(&r) {
                    candidates.insert((t.suit, r as u8));
                }
            }
        }
    }
    // Remove wind/dragon faces that don't already appear — honor tiles can only
    // pair/triplet, so only faces already present are useful.
    candidates.retain(|&(s, _)| number_suits.contains(&s) || tiles.iter().any(|t| t.suit == s));
    candidates.into_iter().collect()
}

/// Try substituting wind tiles with other faces (WildWinds).
/// Recursively substitutes all wind tiles, pruning to only faces that could
/// participate in a meld with the other tiles in the hand.
fn try_wind_substitution(
    tiles: &[Tile],
    rules: &[RuleModifier],
) -> Option<(Vec<DetectedMeld>, Vec<Tile>)> {
    try_wind_substitution_excluding(tiles, &[], rules)
}

/// Like [`try_wind_substitution`] but leaves the wind tiles whose indices
/// appear in `frozen` untouched. Used by Disgust + WildWinds chaining: Disgust
/// freezes the East/West tiles it consumed so WildWinds can play with the
/// remaining winds without breaking the synthetic pair/triplet/kong.
fn try_wind_substitution_excluding(
    tiles: &[Tile],
    frozen: &[usize],
    rules: &[RuleModifier],
) -> Option<(Vec<DetectedMeld>, Vec<Tile>)> {
    let wind_indices: Vec<usize> = tiles
        .iter()
        .enumerate()
        .filter(|(i, t)| t.suit == Suit::Wind && !frozen.contains(i))
        .map(|(i, _)| i)
        .collect();
    if wind_indices.is_empty() {
        return None;
    }
    let candidates = wind_candidate_faces(tiles);
    if candidates.is_empty() {
        return None;
    }
    fn substitute_recursive(
        tiles: &mut Vec<Tile>,
        wind_indices: &[usize],
        pos: usize,
        candidates: &[(Suit, u8)],
        rules: &[RuleModifier],
    ) -> Option<(Vec<DetectedMeld>, Vec<Tile>)> {
        if pos == wind_indices.len() {
            return validate_selection_with_rules(tiles, rules).map(|sets| (sets, tiles.clone()));
        }
        let idx = wind_indices[pos];
        let original = tiles[idx];
        for &(suit, rank) in candidates {
            tiles[idx] = Tile::new(suit, rank, original.id);
            if let Some(result) =
                substitute_recursive(tiles, wind_indices, pos + 1, candidates, rules)
            {
                return Some(result);
            }
        }
        tiles[idx] = original;
        None
    }
    let mut modified = tiles.to_vec();
    substitute_recursive(&mut modified, &wind_indices, 0, &candidates, rules)
}

/// Disgust relic: relabel West tiles as East before validation so that
/// E+W = pair, E+W+W = triplet, E+W+W+W = kong. Only fires when the
/// selection contains at least one East and one West tile.
///
/// When `chain_winds` is true (player also owns WildWinds), any *other* wind
/// tiles in the selection are then treated as wildcards via
/// [`try_wind_substitution_excluding`], with the East/relabeled-West tiles
/// frozen so the synthetic pair/triplet/kong stays intact.
fn try_disgust_substitution(
    tiles: &[Tile],
    rules: &[RuleModifier],
    chain_winds: bool,
) -> Option<(Vec<DetectedMeld>, Vec<Tile>)> {
    let has_east = tiles.iter().any(|t| t.suit == Suit::Wind && t.rank == 1);
    let has_west = tiles.iter().any(|t| t.suit == Suit::Wind && t.rank == 3);
    if !has_east || !has_west {
        return None;
    }
    // Indices of every tile that contributes to the synthetic Disgust set —
    // the original Easts plus the Wests we're about to relabel.
    let frozen: Vec<usize> = tiles
        .iter()
        .enumerate()
        .filter(|(_, t)| t.suit == Suit::Wind && (t.rank == 1 || t.rank == 3))
        .map(|(i, _)| i)
        .collect();
    let modified: Vec<Tile> = tiles
        .iter()
        .map(|t| {
            if t.suit == Suit::Wind && t.rank == 3 {
                let mut east = Tile::new(Suit::Wind, 1, t.id);
                east.enhancement = t.enhancement;
                east.debuffed_visual = t.debuffed_visual;
                east
            } else {
                *t
            }
        })
        .collect();
    if let Some(sets) = validate_selection_with_rules(&modified, rules) {
        return Some((sets, modified));
    }
    if chain_winds {
        return try_wind_substitution_excluding(&modified, &frozen, rules);
    }
    None
}
