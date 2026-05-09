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
mod round_flow;
mod scoring_flow;
mod tags;
mod tutorial;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::boss::{self, BossKind};
use crate::core::debuff::TileDebuff;
use crate::core::deck::Wall;
use crate::core::hand::{
    DetectedSet, SetKind, enumerate_decompositions, validate_selection_with_rules,
};
use crate::core::hand_intent::{
    DecompositionBias, decomposition_affinity, infer_decomposition_bias,
};
use crate::core::structure::{
    StructureTriggerKind, StructureTriggerMeta, banked_meld_chips, can_trigger_structure,
    is_winning_structure_shape,
};

use crate::audio::SfxId;
use crate::core::relic::{RelicId, RelicState, ScoreContext};
use crate::core::rules::{BlindKind, RuleModifier};
use crate::core::scoring::{ScoreBreakdown, score_sets_with_original};
use crate::core::tile::{Suit, Tile, TileEnhancement};
use crate::core::yaku::YakuKind;
use crate::game::event_bus::{EventBus, GameEvent, GameOverReason};
use crate::game::game_mode::GameMode;
use crate::game::onboarding::{OnboardingPhase, OnboardingState, TUTORIAL_BOSS, tutorial_yaku};
use crate::game::tutorial::TutorialState;
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
}

pub const HAND_SIZE: usize = 14;
/// Defeating the Boss of this ante completes the run (Balatro-style).
pub const FINAL_ANTE: u32 = 7;
/// Max number of QuickDraw bonus draws per round.
pub const QUICKDRAW_USES_PER_ROUND: u8 = 3;

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
    pub rustling_goose_egg: bool,
    pub tea_ceremony: bool,
    pub chrysalis: bool,
}

/// Silk Moth / Taotie / Geese / Silver Filigree: shop-only after the primary
/// burns **this run**; never carried in meta `available_relics`.
fn transformation_successor_shop_eligible(
    id: RelicId,
    available_relics: &[RelicId],
    ex: RelicShopPoolExtinction,
) -> bool {
    match id {
        RelicId::SilverFiligreeLantern => {
            ex.paper_lantern && available_relics.contains(&RelicId::PaperLantern)
        }
        RelicId::SilkMoth => ex.silk_thread && available_relics.contains(&RelicId::SilkThread),
        RelicId::Taotie => ex.melting_ice && available_relics.contains(&RelicId::MeltingIce),
        RelicId::Geese => {
            available_relics.contains(&RelicId::RustlingGooseEgg) && ex.rustling_goose_egg
        }
        RelicId::Rakuware => {
            available_relics.contains(&RelicId::TeaCeremony) && ex.tea_ceremony
        }
        RelicId::MonarchButterfly => {
            available_relics.contains(&RelicId::Chrysalis) && ex.chrysalis
        }
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
    if id == RelicId::PhantomRelic {
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
    if id == RelicId::RustlingGooseEgg && ex.rustling_goose_egg {
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

    let allow_wrap = rules.contains(&RuleModifier::SequenceWrap);
    let no_sequences = rules.contains(&RuleModifier::NoSequences);
    let require_honor = rules.contains(&RuleModifier::RequireHonor);
    let must_play_five = rules.contains(&RuleModifier::MustPlayFive);

    let mut masks = std::collections::HashSet::new();
    enumerate_regular_subsets(
        &regular,
        &flowers,
        0,
        allow_wrap,
        no_sequences,
        require_honor,
        must_play_five,
        0,
        &mut masks,
    );
    let mut out: Vec<u32> = masks.into_iter().collect();
    out.sort_unstable();
    out
}

#[allow(clippy::too_many_arguments)]
fn enumerate_regular_subsets(
    remaining: &[IndexedTile],
    flowers: &[IndexedTile],
    current_mask: u32,
    allow_wrap: bool,
    no_sequences: bool,
    require_honor: bool,
    must_play_five: bool,
    current_tile_count: usize,
    out: &mut std::collections::HashSet<u32>,
) {
    if current_tile_count > 14 || (must_play_five && current_tile_count > 5) {
        return;
    }

    if remaining.is_empty() {
        emit_leaf_masks(
            flowers,
            current_mask,
            current_tile_count,
            must_play_five,
            out,
        );
        return;
    }

    let first = remaining[0];
    enumerate_regular_subsets(
        &remaining[1..],
        flowers,
        current_mask,
        allow_wrap,
        no_sequences,
        require_honor,
        must_play_five,
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
            allow_wrap,
            no_sequences,
            require_honor,
            must_play_five,
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
            allow_wrap,
            no_sequences,
            require_honor,
            must_play_five,
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
            allow_wrap,
            no_sequences,
            require_honor,
            must_play_five,
            current_tile_count + 4,
            out,
        );
    }

    if !flowers.is_empty()
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
                allow_wrap,
                no_sequences,
                require_honor,
                must_play_five,
                current_tile_count + 3,
                out,
            );
        }
    }

    if !no_sequences && first.tile.is_number_tile() && !require_honor {
        for seq in sequence_candidates(remaining, allow_wrap, !flowers.is_empty(), first) {
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
                        allow_wrap,
                        no_sequences,
                        require_honor,
                        must_play_five,
                        current_tile_count + 3,
                        out,
                    );
                }
            } else {
                enumerate_regular_subsets(
                    &rest,
                    flowers,
                    next_mask,
                    allow_wrap,
                    no_sequences,
                    require_honor,
                    must_play_five,
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
    must_play_five: bool,
    out: &mut std::collections::HashSet<u32>,
) {
    for extra_mask in flower_only_masks(flowers) {
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

fn flower_only_masks(flowers: &[IndexedTile]) -> Vec<u32> {
    let mut masks = vec![0];

    for select_count in 2..=flowers.len().min(4) {
        collect_flower_masks(flowers, select_count, 0, 0, &mut masks);
    }

    masks
}

fn collect_flower_masks(
    flowers: &[IndexedTile],
    select_count: usize,
    start: usize,
    current_mask: u32,
    out: &mut Vec<u32>,
) {
    if select_count == 0 {
        out.push(current_mask);
        return;
    }

    for idx in start..=flowers.len() - select_count {
        collect_flower_masks(
            flowers,
            select_count - 1,
            idx + 1,
            current_mask | (1 << flowers[idx].hand_index),
            out,
        );
    }
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
    structure_sets: Vec<DetectedSet>,
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
    /// Remaining QuickDraw extra-tile draws this round. Set to 3 at round
    /// start; each play that draws decrements it. Once at zero, QuickDraw is
    /// inert until the next round.
    #[serde(default)]
    pub quickdraw_uses_remaining: u8,
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
    /// Cumulative score earned across the entire run (for Snowball relic).
    #[serde(default)]
    pub total_score_earned: u64,
    /// True once Paper Lantern has burned up this run. Prevents Paper from
    /// reappearing in shops and unlocks Silver Filigree Lantern in the shop pool.
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
    pub rustling_goose_egg_extinct: bool,
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
    pub yaku_times_played: std::collections::HashMap<crate::core::yaku::YakuKind, u32>,
    /// Cumulative tiles committed from hand into scored plays / structure.
    #[serde(default)]
    pub tiles_played: u32,
    /// Cumulative tiles thrown away via the discard action.
    #[serde(default)]
    pub tiles_discarded: u32,
    /// Number of times the hand was replenished after spending tiles.
    #[serde(default)]
    pub times_restocked: u32,
    /// Highest score earned by a single scored hand / structure cash-in.
    #[serde(default)]
    pub best_structure_score: u64,
    /// Display label for the highest-scoring structure.
    #[serde(default)]
    pub best_structure_name: String,
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
    /// Tile IDs permanently removed from the wall via the Kiln talisman.
    /// Filtered out during wall construction each round.
    #[serde(default)]
    pub removed_tile_ids: std::collections::HashSet<u32>,
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
    /// Pending zodiac activation from a ZodiacBlessing skip-reward tag.
    /// Consumed by the pick-blind scene to trigger the celebration overlay.
    #[serde(skip)]
    pub pending_zodiac_celebration: Option<(
        crate::core::zodiac::ZodiacKind,
        crate::core::yaku::YakuKind,
        u32,
    )>,
    /// Data from a dismissed zodiac celebration overlay, consumed by the
    /// underlying scene (shop) to spawn a score popup + particle burst.
    #[serde(skip)]
    pub finished_zodiac_celebration: Option<(&'static str, u32)>,
    /// Set when the tile-pack celebration overlay pops; [`ShopScene`] consumes
    /// it once to refocus the shelf (same as the old in-shop celebration path).
    #[serde(skip)]
    pub pending_shop_focus_snap_after_pack_celebration: bool,
    /// Per-relic mutable counters. Key is RelicId, value meaning depends
    /// on the relic:
    ///   Humility     → consecutive plays without honor tiles
    ///   Obsession    → rounds without most-used yaku
    ///   Bonfire      → relics sold this run
    ///   MeltingIce   → remaining chip bonus (starts 80, -8 per play)
    ///   SilkThread   → remaining mult ×10 (starts 40, -3 per discard)
    ///   NestEgg      → rounds held (sell value grows)
    ///   RustlingGooseEgg (XXXL Egg) → plays remaining before burning (Geese shop unlock)
    ///   TeaCeremony  → principle index 0–3 (four scored hands, then transforms)
    ///   Rakuware     → (no counter; all four Tea beats when conditions hold)
    ///   MonarchButterfly → cumulative absorbed excess (post-target); tiers for chip bonus
    ///   PhantomRelic → rounds held
    ///   HungryGhost  → permanent mult bonus ×10
    ///   TilePolisher → accumulated +chip bonus (each scored tile +3)
    ///   RiverRunner  → accumulated +chip bonus (each scored sequence +20)
    ///   IGotAGuy     → shop restock waivers remaining (starts 3 on buy)
    #[serde(default)]
    pub relic_counters: std::collections::BTreeMap<RelicId, i32>,
    /// Tutorial state. `None` for normal (non-tutorial) runs. Present
    /// during the player's very first run to gate mechanics by lesson.
    #[serde(default)]
    pub tutorial: Option<TutorialState>,
    /// Curated onboarding-campaign state for the revamped first-time tutorial.
    #[serde(default)]
    pub onboarding: Option<OnboardingState>,

    /// Relics whose effects just fired this frame. Scenes drain this each
    /// frame to drive glow + wiggle animations. Populated by `run.rs`
    /// methods whenever a relic triggers (scoring, round-end, discard,
    /// draw, consumable interaction, etc.).
    #[serde(skip)]
    pub relic_activations: Vec<RelicId>,
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

    /// Ghost Hand HUD / tooltip preview: in structure-bank mode the next cash-in
    /// scores all tiles still in hand as "unscored"; in classic mode, if any tile
    /// is selected the preview is the sum of **un**selected hand tiles (what stays
    /// out of the meld), otherwise the sum of the whole hand before you choose a meld.
    pub fn ghost_hand_preview_chips(&self) -> i32 {
        let hand = self.hand();
        let debuffs = &self.tile_debuffs;
        let sum_points = |tiles: &[Tile]| -> i32 {
            tiles
                .iter()
                .filter(|t| !debuffs.iter().any(|d| d.matches(t)))
                .map(|t| t.point_value() as i32)
                .sum()
        };
        if self.mode.structure_bank {
            return sum_points(hand);
        }
        let sel = self.selected_slice();
        if sel.iter().any(|&x| x) {
            let unselected: Vec<Tile> = hand
                .iter()
                .enumerate()
                .filter(|(i, _)| !sel.get(*i).copied().unwrap_or(false))
                .map(|(_, t)| *t)
                .collect();
            return sum_points(&unselected);
        }
        sum_points(hand)
    }

    pub(crate) fn selected_mut(&mut self) -> &mut Vec<bool> {
        &mut self.selected
    }

    pub fn structure_sets(&self) -> &[DetectedSet] {
        &self.structure_sets
    }

    pub(crate) fn structure_sets_mut(&mut self) -> &mut Vec<DetectedSet> {
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
        structure_sets: Vec<DetectedSet>,
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
        d
    }

    fn round_play_cap(&self) -> u32 {
        self.mode.starting_plays
    }

    fn round_discard_cap(&self) -> u32 {
        Self::discard_cap_for(&self.mode, &self.relics)
    }

    pub fn relic_shop_pool_extinction(&self) -> RelicShopPoolExtinction {
        RelicShopPoolExtinction {
            paper_lantern: self.paper_lantern_extinct,
            silk_thread: self.silk_thread_extinct,
            melting_ice: self.melting_ice_extinct,
            rustling_goose_egg: self.rustling_goose_egg_extinct,
            tea_ceremony: self.tea_ceremony_extinct,
            chrysalis: self.chrysalis_extinct,
        }
    }

    /// Canonical *relic destroyed* trigger.
    ///
    /// The "destroyed" keyword is the
    /// player-facing name for permanent removal of a relic from a run; this
    /// function is the single code-side anchor the keyword refers to. Every
    /// path that destroys a relic should call this *after* removing the
    /// victim from `relics.active`. Kintsugi converts each invocation into
    /// a permanent +1 mult via its counter — adding a new destruction site
    /// without going through here will silently break that synergy.
    ///
    /// Gros-Michel-style relic burns (Paper Lantern, Silk Thread, Melting Ice,
    /// XXXL Egg, Glass Cannon after its scoring use) remove the relic from inventory and call this so Kintsugi
    /// can react; successors enter the shop pool via [`RelicShopPoolExtinction`].
    /// Tea Ceremony instead transforms into Rakuware in-slot and also invokes this
    /// so Kintsugi can count the finished ritual. Chrysalis transforms into
    /// Monarch Butterfly the same way when excess crosses the hatch threshold.
    fn note_relic_destroyed(&mut self) {
        if self.relics.has(crate::core::relic::RelicId::Kintsugi) {
            *self
                .relic_counters
                .entry(crate::core::relic::RelicId::Kintsugi)
                .or_insert(0) += 1;
            self.relic_activations
                .push(crate::core::relic::RelicId::Kintsugi);
        }
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
        let mut state = Self {
            wall,
            hand,
            selected,
            structure_sets: Vec::new(),
            structure_tiles: Vec::new(),
            round_score: 0,
            target_score: mode.base_target,
            base_target: mode.base_target,
            relics,
            round_rules: mode.starting_rules.clone(),
            run_number: 1,
            ante: 1,
            plays_remaining: mode.starting_plays,
            plays_max: mode.starting_plays,
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
            quickdraw_uses_remaining: 0,
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
            total_score_earned: 0,
            paper_lantern_extinct: false,
            silk_thread_extinct: false,
            melting_ice_extinct: false,
            rustling_goose_egg_extinct: false,
            tea_ceremony_extinct: false,
            chrysalis_extinct: false,
            yaku_times_played: std::collections::HashMap::new(),
            tiles_played: 0,
            tiles_discarded: 0,
            times_restocked: 0,
            best_structure_score: 0,
            best_structure_name: String::new(),
            tile_enhancements: BTreeMap::new(),
            global_buff_enhancement: None,
            removed_tile_ids: std::collections::HashSet::new(),
            tile_packs: Vec::new(),
            small_blind_tag: None,
            big_blind_tag: None,
            tag_free_reroll: false,
            tag_patron_gift: false,
            tag_rich_stock: false,
            tag_bonus_plays: 0,
            tag_bonus_discards: 0,
            tag_bonus_hand_size: 0,
            pending_zodiac_celebration: None,
            finished_zodiac_celebration: None,
            pending_shop_focus_snap_after_pack_celebration: false,
            relic_counters: std::collections::BTreeMap::new(),
            tutorial: None,
            onboarding: None,
            relic_activations: Vec::new(),
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

    /// Convenience constructor using the standard game mode.
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
        let mut counts: std::collections::HashMap<(crate::core::tile::Suit, u8), u32> =
            std::collections::HashMap::new();
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
                self.relic_activations.push(RelicId::SetMagnet);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::deck::build_wall;
    use crate::core::hand::DetectedSet;
    use crate::core::relic::RelicId;

    /// Standard mode starting plays (Bamboo: 4 base + 1 bonus).
    const STARTING_PLAYS: u32 = 5;
    /// Standard mode starting discards (Bamboo: 4 base + 0 bonus).
    const STARTING_DISCARDS: u32 = 4;

    // Create a RunState with a deterministic (unshuffled) wall for predictable tests.
    fn test_run() -> RunState {
        let tiles = build_wall(); // deterministic order: Char 1-9, Bam 1-9, Cir 1-9, Winds, Dragons
        let mut wall = Wall::from_unshuffled(tiles);
        let mut hand = Vec::with_capacity(HAND_SIZE);
        for _ in 0..HAND_SIZE {
            if let Some(t) = wall.draw() {
                hand.push(t);
            }
        }
        let selected = vec![false; hand.len()];
        let mode = GameMode {
            starting_gold: 0,
            starting_rules: vec![],
            starting_yaku: vec![],
            ..GameMode::standard()
        };
        RunState {
            ante: 1,
            available_rules: vec![],
            available_yaku: vec![],
            available_relics: default_available_relics(),
            base_target: mode.base_target,
            blind: BlindKind::Small,
            boss: BossState::default(),
            consumables: crate::core::consumable::ConsumableInventory::default(),
            discards_remaining: mode.starting_discards,
            discards_max: mode.starting_discards,
            full_hand_played_this_round: false,
            gold: mode.starting_gold as i32,
            hand,
            structure_sets: vec![],
            structure_tiles: vec![],
            joker_used: false,
            last_breakdown: None,
            mode: mode.clone(),
            auto_cash_in_on_full_structure: true,
            hints_enabled: false,
            played_yaku_this_round: vec![],
            tile_debuffs: vec![],
            honors_scored_this_round: false,
            yaku_times_played: std::collections::HashMap::new(),
            tiles_played: 0,
            tiles_discarded: 0,
            times_restocked: 0,
            best_structure_score: 0,
            best_structure_name: String::new(),
            plays_remaining: mode.starting_plays,
            plays_max: mode.starting_plays,
            quickdraw_uses_remaining: 0,
            relics: RelicState::default(),
            round_rules: vec![],
            round_score: 0,
            run_number: 1,
            scored_last_turn: false,
            selected,
            target_score: mode.base_target,
            tile_enhancements: BTreeMap::new(),
            global_buff_enhancement: None,
            removed_tile_ids: std::collections::HashSet::new(),
            upcoming_blind: BlindKind::Small,
            wall,
            yaku_levels: crate::core::zodiac::YakuLevels::default(),
            tile_packs: vec![],
            total_score_earned: 0,
            paper_lantern_extinct: false,
            silk_thread_extinct: false,
            melting_ice_extinct: false,
            rustling_goose_egg_extinct: false,
            tea_ceremony_extinct: false,
            chrysalis_extinct: false,
            small_blind_tag: None,
            big_blind_tag: None,
            tag_free_reroll: false,
            tag_patron_gift: false,
            tag_rich_stock: false,
            tag_bonus_plays: 0,
            tag_bonus_discards: 0,
            tag_bonus_hand_size: 0,
            pending_zodiac_celebration: None,
            finished_zodiac_celebration: None,
            pending_shop_focus_snap_after_pack_celebration: false,
            relic_counters: BTreeMap::new(),
            tutorial: None,
            onboarding: None,
            relic_activations: Vec::new(),
        }
    }

    fn bus() -> EventBus {
        EventBus::default()
    }

    fn winning_structure() -> (Vec<Tile>, Vec<DetectedSet>) {
        let tiles = vec![
            Tile::new(Suit::Characters, 1, 1),
            Tile::new(Suit::Characters, 1, 2),
            Tile::new(Suit::Characters, 2, 3),
            Tile::new(Suit::Characters, 3, 4),
            Tile::new(Suit::Characters, 4, 5),
            Tile::new(Suit::Circles, 2, 6),
            Tile::new(Suit::Circles, 3, 7),
            Tile::new(Suit::Circles, 4, 8),
            Tile::new(Suit::Bamboos, 5, 9),
            Tile::new(Suit::Bamboos, 6, 10),
            Tile::new(Suit::Bamboos, 7, 11),
            Tile::new(Suit::Wind, 1, 12),
            Tile::new(Suit::Wind, 1, 13),
            Tile::new(Suit::Wind, 1, 14),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![1, 2],
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
                kind: SetKind::Sequence,
                tile_ids: vec![9, 10, 11],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![12, 13, 14],
            },
        ];
        (tiles, sets)
    }

    // ── toggle_select ───────────────────────────────────────────────

    #[test]
    fn toggle_select_marks_tile() {
        let mut run = test_run();
        assert!(!run.selected[0]);
        run.toggle_select(0);
        assert!(run.selected[0]);
    }

    #[test]
    fn toggle_select_unmarks_tile() {
        let mut run = test_run();
        run.toggle_select(3);
        assert!(run.selected[3]);
        run.toggle_select(3);
        assert!(!run.selected[3]);
    }

    #[test]
    fn toggle_select_out_of_bounds_is_noop() {
        let mut run = test_run();
        run.toggle_select(999); // should not panic
        assert_eq!(run.selected_count(), 0);
    }

    #[test]
    fn toggle_select_multiple_tiles() {
        let mut run = test_run();
        run.toggle_select(0);
        run.toggle_select(5);
        run.toggle_select(13);
        assert_eq!(run.selected_count(), 3);
    }

    // ── clear_selection ─────────────────────────────────────────────

    #[test]
    fn clear_selection_resets_all() {
        let mut run = test_run();
        run.toggle_select(0);
        run.toggle_select(7);
        run.toggle_select(12);
        assert_eq!(run.selected_count(), 3);
        run.clear_selection();
        assert_eq!(run.selected_count(), 0);
        assert!(run.selected.iter().all(|&s| !s));
    }

    #[test]
    fn clear_selection_on_empty_is_noop() {
        let mut run = test_run();
        run.clear_selection(); // should not panic
        assert_eq!(run.selected_count(), 0);
    }

    // ── selected_count ──────────────────────────────────────────────

    #[test]
    fn selected_count_starts_at_zero() {
        let run = test_run();
        assert_eq!(run.selected_count(), 0);
    }

    #[test]
    fn selected_count_tracks_toggles() {
        let mut run = test_run();
        run.toggle_select(0);
        assert_eq!(run.selected_count(), 1);
        run.toggle_select(1);
        assert_eq!(run.selected_count(), 2);
        run.toggle_select(0);
        assert_eq!(run.selected_count(), 1);
    }

    // ── discard_selected ────────────────────────────────────────────

    #[test]
    fn discard_selected_removes_tiles_and_redraws() {
        let mut run = test_run();
        let mut bus = bus();
        let original_hand = run.hand.clone();

        run.toggle_select(0);
        run.toggle_select(1);
        let discarded = run.discard_selected(&mut bus);

        assert_eq!(discarded, 2);
        assert_eq!(run.hand.len(), HAND_SIZE); // auto-drew back to full
        // The first two tiles should be gone.
        assert!(!run.hand.contains(&original_hand[0]));
        assert!(!run.hand.contains(&original_hand[1]));
    }

    #[test]
    fn discard_selected_costs_one_discard() {
        let mut run = test_run();
        let mut bus = bus();
        assert_eq!(run.discards_remaining, STARTING_DISCARDS);

        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        run.discard_selected(&mut bus);

        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 1);
    }

    #[test]
    fn discard_selected_clears_selection_after() {
        let mut run = test_run();
        let mut bus = bus();

        run.toggle_select(0);
        run.toggle_select(5);
        run.discard_selected(&mut bus);

        assert_eq!(run.selected_count(), 0);
        assert_eq!(run.selected.len(), run.hand.len());
    }

    #[test]
    fn discard_selected_returns_zero_when_none_selected() {
        let mut run = test_run();
        let mut bus = bus();
        let discarded = run.discard_selected(&mut bus);
        assert_eq!(discarded, 0);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS); // not decremented
    }

    #[test]
    fn discard_selected_returns_zero_when_no_discards_left() {
        let mut run = test_run();
        let mut bus = bus();
        run.discards_remaining = 0;

        run.toggle_select(0);
        let discarded = run.discard_selected(&mut bus);

        assert_eq!(discarded, 0);
        assert_eq!(run.hand.len(), HAND_SIZE); // hand unchanged
    }

    #[test]
    fn discard_selected_emits_events() {
        let mut run = test_run();
        let mut bus = bus();

        run.toggle_select(2);
        run.toggle_select(4);
        run.discard_selected(&mut bus);

        let events: Vec<_> = bus.drain().collect();
        // Should have TileDiscarded events + TileDrawn events for the redraws.
        let discards: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, GameEvent::TileDiscarded))
            .collect();
        let draws: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, GameEvent::TileDrawn))
            .collect();
        assert_eq!(discards.len(), 2);
        assert_eq!(draws.len(), 2); // drew 2 to replace the 2 discarded
    }

    #[test]
    fn discard_selected_preserves_non_selected_tiles() {
        let mut run = test_run();
        let mut bus = bus();

        // Remember non-selected tile ids.
        let kept_ids: Vec<u32> = run
            .hand
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 3 && *i != 7)
            .map(|(_, t)| t.id)
            .collect();

        run.toggle_select(3);
        run.toggle_select(7);
        run.discard_selected(&mut bus);

        // All originally-kept tiles should still be in hand.
        for id in &kept_ids {
            assert!(
                run.hand.iter().any(|t| t.id == *id),
                "tile id {} was lost",
                id
            );
        }
    }

    #[test]
    fn multiple_discard_rounds() {
        let mut run = test_run();
        let mut bus = bus();

        // First discard: remove 3 tiles.
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        run.discard_selected(&mut bus);
        assert_eq!(run.hand.len(), HAND_SIZE);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 1);

        // Second discard: remove 1 tile.
        run.toggle_select(0);
        run.discard_selected(&mut bus);
        assert_eq!(run.hand.len(), HAND_SIZE);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 2);

        // Third discard: remove 5 tiles.
        for i in 0..5 {
            run.toggle_select(i);
        }
        run.discard_selected(&mut bus);
        assert_eq!(run.hand.len(), HAND_SIZE);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 3);

        // Fourth discard: removes the last allowance.
        run.toggle_select(0);
        run.discard_selected(&mut bus);
        assert_eq!(run.discards_remaining, 0);

        // Fifth attempt: should fail (no discards left).
        run.toggle_select(0);
        let result = run.discard_selected(&mut bus);
        assert_eq!(result, 0);
        assert_eq!(run.discards_remaining, 0);
    }

    #[test]
    fn discard_all_14_tiles_redraws_full_hand() {
        let mut run = test_run();
        let mut bus = bus();

        for i in 0..HAND_SIZE {
            run.toggle_select(i);
        }
        let discarded = run.discard_selected(&mut bus);
        assert_eq!(discarded, HAND_SIZE);
        assert_eq!(run.hand.len(), HAND_SIZE); // wall has 136 - 14 = 122 tiles, plenty to redraw
    }

    // ── auto-draw with depleted wall ────────────────────────────────

    #[test]
    fn discard_with_depleted_wall_draws_what_it_can() {
        let mut run = test_run();
        let mut bus = bus();

        // Drain the wall almost completely: wall started with 140, 14 already drawn.
        // Draw remaining 126 tiles to exhaust the wall.
        for _ in 0..126 {
            run.wall.draw();
        }
        assert!(run.wall.draw().is_none()); // wall is empty

        run.toggle_select(0);
        run.toggle_select(1);
        run.discard_selected(&mut bus);

        // Can't redraw, so hand is now 12.
        assert_eq!(run.hand.len(), HAND_SIZE - 2);
        assert_eq!(run.selected.len(), run.hand.len());
    }

    // ── selected vec stays in sync ──────────────────────────────────

    #[test]
    fn selected_vec_length_matches_hand_after_discard() {
        let mut run = test_run();
        let mut bus = bus();

        run.toggle_select(5);
        run.discard_selected(&mut bus);

        assert_eq!(run.selected.len(), run.hand.len());
        // All should be false after discard.
        assert!(run.selected.iter().all(|&s| !s));
    }

    #[test]
    fn selected_vec_length_matches_hand_at_init() {
        let run = test_run();
        assert_eq!(run.selected.len(), run.hand.len());
        assert_eq!(run.selected.len(), HAND_SIZE);
    }

    // ── advance_round resets selection ───────────────────────────────

    #[test]
    fn advance_round_resets_selection() {
        let mut run = test_run();
        let mut bus = bus();
        run.toggle_select(0);
        run.toggle_select(5);
        assert_eq!(run.selected_count(), 2);

        run.advance_round(&mut bus);

        assert_eq!(run.selected_count(), 0);
        assert_eq!(run.selected.len(), run.hand.len());
        assert_eq!(run.discards_remaining, STARTING_DISCARDS);
    }

    #[test]
    fn apply_blind_rebuilds_round_resources_from_current_bonuses() {
        let mut run = test_run();
        run.plays_remaining = 1;
        run.discards_remaining = 0;
        run.tag_bonus_plays = 1;
        run.tag_bonus_discards = 1;

        run.apply_blind(BlindKind::Small);

        assert_eq!(run.plays_remaining, STARTING_PLAYS + 1);
        assert_eq!(run.plays_max, STARTING_PLAYS + 1);
        assert_eq!(run.discards_remaining, STARTING_DISCARDS + 1);
        assert_eq!(run.discards_max, STARTING_DISCARDS + 1);
        assert_eq!(run.tag_bonus_plays, 0);
        assert_eq!(run.tag_bonus_discards, 0);
    }

    #[test]
    fn second_wind_salvages_round_instead_of_game_over() {
        let mut run = test_run();
        let mut bus = bus();
        run.relics.active.push(RelicId::SecondWind);
        run.hand = vec![
            Tile::new(Suit::Characters, 1, 1),
            Tile::new(Suit::Characters, 3, 2),
            Tile::new(Suit::Characters, 5, 3),
            Tile::new(Suit::Characters, 7, 4),
            Tile::new(Suit::Characters, 9, 5),
            Tile::new(Suit::Bamboos, 2, 6),
            Tile::new(Suit::Bamboos, 4, 7),
            Tile::new(Suit::Bamboos, 6, 8),
            Tile::new(Suit::Bamboos, 8, 9),
            Tile::new(Suit::Circles, 1, 10),
            Tile::new(Suit::Circles, 3, 11),
            Tile::new(Suit::Circles, 5, 12),
            Tile::new(Suit::Wind, 1, 13),
            Tile::new(Suit::Dragon, 1, 14),
        ];
        run.selected = vec![false; run.hand.len()];
        run.discards_remaining = 0;
        run.plays_remaining = 3;
        run.structure_sets.clear();
        run.structure_tiles.clear();

        run.refill_hand(&mut bus);

        assert!(
            !bus.queue.iter().any(|ev| matches!(ev, GameEvent::GameOver { .. })),
            "Second Wind should prevent GameOver"
        );
        assert!(
            bus.queue.iter().any(|ev| {
                matches!(
                    ev,
                    GameEvent::RoundComplete {
                        reached_target: false,
                        ..
                    }
                )
            }),
            "Second Wind should enqueue a zero-payout RoundComplete"
        );
        assert!(
            !run.relics.has(RelicId::SecondWind),
            "Second Wind should be destroyed"
        );
        run.forfeit_current_blind_second_wind(&mut bus);
        assert_eq!(run.upcoming_blind, BlindKind::Big);
        assert_eq!(run.run_number, 2);
    }

    #[test]
    fn second_wind_plays_used_uses_effective_round_cap() {
        let mut run = test_run();
        run.apply_blind(BlindKind::Small);
        run.plays_remaining -= 2;

        let rw = Some(BlindKind::round_wind_for_ante(run.ante));
        let ctx = ScoreContext {
            relics: &run.relics,
            tile_debuffs: &[],
            scored_last_turn: run.scored_last_turn,
            dora_faces: run.wall.dora_faces(),
            available_yaku: run.available_yaku.clone(),
            round_wind: rw,
            plays_used: run.round_play_cap().saturating_sub(run.plays_remaining),
            yaku_levels: Some(run.yaku_levels.clone()),
            played_yaku_this_round: run.played_yaku_this_round.clone(),
            gold: run.gold,
            total_score: run.total_score_earned,
            is_final_play: run.plays_remaining == 0,
            relic_counters: run.relic_counters.clone(),
            hand_for_ghost: run.hand(),
            structure: None,
        };

        assert_eq!(ctx.plays_used, 2);
    }

    #[test]
    fn apply_blind_uses_material_starting_discards_before_skip_bonus() {
        let mut run = RunState::new(GameMode::with_material(
            crate::persistence::TileMaterial::Plastic,
        ));
        run.discards_remaining = 0;
        run.tag_bonus_discards = 1;

        run.apply_blind(BlindKind::Small);

        assert_eq!(run.discards_remaining, 6);
        assert_eq!(run.discards_max, 6);
        assert_eq!(run.tag_bonus_discards, 0);
    }

    #[test]
    fn apply_blind_tracks_reduced_round_caps_for_boss_taxes() {
        let mut run = test_run();
        run.boss.effect = Some(crate::core::boss::ResolvedBossEffect::from_static(
            &crate::core::boss::BossKind::Drought.def().effect,
        ));

        run.apply_blind(BlindKind::Boss);

        assert_eq!(run.discards_remaining, STARTING_DISCARDS / 2);
        assert_eq!(run.discards_max, STARTING_DISCARDS / 2);
    }

    #[test]
    fn big_hands_increases_effective_hand_and_reduces_discard_cap() {
        let mut run = test_run();
        run.relics.active.push(RelicId::BigHands);
        assert_eq!(boss::effective_hand_size(&run), HAND_SIZE + 2);
        run.reset_round_resources();
        assert_eq!(run.discards_remaining, STARTING_DISCARDS - 1);
        assert_eq!(run.discards_max, STARTING_DISCARDS - 1);
    }

    #[test]
    fn tiny_hands_decreases_effective_hand_and_adds_discard_cap() {
        let mut run = test_run();
        run.relics.active.push(RelicId::TinyHands);
        assert_eq!(boss::effective_hand_size(&run), HAND_SIZE - 2);
        run.reset_round_resources();
        assert_eq!(run.discards_remaining, STARTING_DISCARDS + 2);
        assert_eq!(run.discards_max, STARTING_DISCARDS + 2);
    }

    #[test]
    fn big_hands_and_tiny_hands_cancel_hand_delta() {
        let mut run = test_run();
        run.relics.active.push(RelicId::BigHands);
        run.relics.active.push(RelicId::TinyHands);
        assert_eq!(boss::effective_hand_size(&run), HAND_SIZE);
    }

    #[test]
    fn refill_hand_reaches_big_hands_target_from_undersized_hand() {
        let mut run = test_run();
        run.relics.active.push(RelicId::BigHands);
        assert_eq!(run.hand.len(), HAND_SIZE);
        let mut bus = bus();
        run.refill_hand(&mut bus);
        assert_eq!(run.hand.len(), HAND_SIZE + 2);
    }

    #[test]
    fn apply_blind_promotes_wide_hand_bonus_to_round_hand_size() {
        let mut run = test_run();
        run.apply_tag(crate::core::tag::TagKind::WideHand);

        run.apply_blind(BlindKind::Small);

        assert_eq!(run.hand.len(), HAND_SIZE + 2);
        assert_eq!(boss::effective_hand_size(&run), HAND_SIZE + 2);
        assert_eq!(run.tag_bonus_hand_size, 0);
    }

    #[test]
    fn skipping_with_wide_hand_carries_bonus_into_next_blind() {
        let mut run = test_run();

        run.apply_tag(crate::core::tag::TagKind::WideHand);
        run.skip_to_next_blind();

        assert_eq!(run.tag_bonus_hand_size, 2);

        run.apply_blind(BlindKind::Big);

        assert_eq!(run.hand.len(), HAND_SIZE + 2);
        assert_eq!(boss::effective_hand_size(&run), HAND_SIZE + 2);
        assert_eq!(run.tag_bonus_hand_size, 0);
    }

    #[test]
    fn advance_round_after_boss_preserves_pending_shop_skip_rewards() {
        let mut run = test_run();
        let mut bus = bus();
        run.blind = BlindKind::Boss;
        run.upcoming_blind = BlindKind::Boss;
        run.tag_free_reroll = true;
        run.tag_patron_gift = true;
        run.tag_rich_stock = true;

        run.advance_round(&mut bus);

        assert!(run.tag_free_reroll);
        assert!(run.tag_patron_gift);
        assert!(run.tag_rich_stock);
    }

    #[test]
    fn advance_round_after_boss_clears_unconsumed_next_blind_skip_bonuses() {
        let mut run = test_run();
        let mut bus = bus();
        run.blind = BlindKind::Boss;
        run.upcoming_blind = BlindKind::Boss;
        run.tag_bonus_plays = 1;
        run.tag_bonus_discards = 1;
        run.tag_bonus_hand_size = 2;

        run.advance_round(&mut bus);

        assert_eq!(run.tag_bonus_plays, 0);
        assert_eq!(run.tag_bonus_discards, 0);
        assert_eq!(run.tag_bonus_hand_size, 0);
    }

    // ── score_selected_tiles ──────────────────────────────────────

    #[test]
    fn score_selected_valid_triplet() {
        let mut run = test_run();
        let mut bus = bus();
        // Deterministic hand (sorted): 1m×4, 2m×4, 3m×4, 4m×2
        // Select first 3 tiles (1m, 1m, 1m) — a triplet.
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        let pts = run.score_selected_tiles(&mut bus);
        assert!(pts > 0, "valid triplet should score");
        assert_eq!(run.plays_remaining, STARTING_PLAYS - 1);
        // Scored tiles removed and redrawn.
        assert_eq!(run.hand.len(), HAND_SIZE);
        assert_eq!(run.selected_count(), 0);
    }

    #[test]
    fn glass_cannon_destroys_after_first_scoring_hand() {
        let mut run = test_run();
        run.mode.structure_bank = false;
        run.relics.active.push(RelicId::GlassCannon);
        let mut bus = bus();
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        let _ = run.score_selected_tiles(&mut bus);
        assert!(!run.relics.active.contains(&RelicId::GlassCannon));
    }

    #[test]
    fn glass_cannon_does_not_reduce_starting_plays_cap() {
        let mut run = test_run();
        assert_eq!(run.round_play_cap(), STARTING_PLAYS);
        run.relics.active.push(RelicId::GlassCannon);
        assert_eq!(run.round_play_cap(), STARTING_PLAYS);
    }

    #[test]
    fn classic_mode_scores_on_commit_without_structure_bank() {
        let mut run = test_run();
        run.mode.structure_bank = false;
        let mut bus = bus();
        let score_before = run.round_score;
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        let pts = run.score_selected_tiles(&mut bus);
        assert!(pts > 0);
        assert!(
            run.round_score > score_before,
            "classic play adds to round score immediately"
        );
        assert!(run.structure_sets.is_empty());
        assert!(run.structure_tiles.is_empty());
    }

    #[test]
    fn dragon_allows_non_honor_play_but_debuffs_its_score() {
        let mut baseline = test_run();
        baseline.mode.structure_bank = false;
        baseline.blind = BlindKind::Boss;
        baseline.upcoming_blind = BlindKind::Boss;
        let mut baseline_bus = bus();
        baseline.toggle_select(0);
        baseline.toggle_select(1);
        baseline.toggle_select(2);
        let baseline_pts = baseline.score_selected_tiles(&mut baseline_bus);
        assert!(baseline_pts > 0);
        let baseline_score = baseline.round_score;

        let mut dragon = test_run();
        dragon.mode.structure_bank = false;
        dragon.blind = BlindKind::Boss;
        dragon.upcoming_blind = BlindKind::Boss;
        dragon.boss.upcoming = Some(BossKind::Dragon);
        let mut dragon_bus = bus();
        dragon.toggle_select(0);
        dragon.toggle_select(1);
        dragon.toggle_select(2);
        let dragon_pts = dragon.score_selected_tiles(&mut dragon_bus);
        assert!(dragon_pts > 0, "Dragon should still allow cycling plays");
        assert!(
            dragon.round_score > 0,
            "debuffed Dragon plays should still score something"
        );
        assert!(
            dragon.round_score < baseline_score,
            "Dragon should weaken non-honor plays instead of hard-locking them"
        );
    }

    #[test]
    fn full_structure_autocash_can_be_disabled() {
        let mut run = test_run();
        run.set_auto_cash_in_on_full_structure(false);
        let mut bus = bus();
        let (tiles, sets) = winning_structure();
        run.structure_tiles = tiles;
        run.structure_sets = sets;

        run.try_autotrigger_structure_full(&mut bus);

        assert_eq!(run.structure_sets.len(), 5);
        assert_eq!(run.structure_tiles.len(), 14);
        assert_eq!(run.round_score, 0);
    }

    #[test]
    fn full_structure_autocash_defaults_on() {
        let mut run = test_run();
        let mut bus = bus();
        let (tiles, sets) = winning_structure();
        run.structure_tiles = tiles;
        run.structure_sets = sets;

        run.try_autotrigger_structure_full(&mut bus);

        assert!(run.structure_sets.is_empty());
        assert!(run.structure_tiles.is_empty());
        assert!(run.round_score > 0);
    }

    #[test]
    fn score_selected_invalid_returns_zero() {
        let mut run = test_run();
        let mut bus = bus();
        // Select 4 tiles: triplet + 1 leftover → invalid.
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        run.toggle_select(4); // 2m — leftover
        let pts = run.score_selected_tiles(&mut bus);
        assert_eq!(pts, 0, "invalid selection should score 0");
        assert_eq!(run.plays_remaining, STARTING_PLAYS, "no play consumed");
        assert_eq!(run.hand.len(), HAND_SIZE, "hand unchanged");
    }

    #[test]
    fn score_selected_nothing_returns_zero() {
        let mut run = test_run();
        let mut bus = bus();
        let pts = run.score_selected_tiles(&mut bus);
        assert_eq!(pts, 0);
        assert_eq!(run.plays_remaining, STARTING_PLAYS);
    }

    #[test]
    fn refill_hand_ends_round_when_no_actions_remain() {
        let mut run = test_run();
        let mut bus = bus();
        run.hand = vec![
            Tile::new(Suit::Characters, 1, 1),
            Tile::new(Suit::Characters, 3, 2),
            Tile::new(Suit::Characters, 5, 3),
            Tile::new(Suit::Characters, 7, 4),
            Tile::new(Suit::Characters, 9, 5),
            Tile::new(Suit::Bamboos, 2, 6),
            Tile::new(Suit::Bamboos, 4, 7),
            Tile::new(Suit::Bamboos, 6, 8),
            Tile::new(Suit::Bamboos, 8, 9),
            Tile::new(Suit::Circles, 1, 10),
            Tile::new(Suit::Circles, 3, 11),
            Tile::new(Suit::Circles, 5, 12),
            Tile::new(Suit::Wind, 1, 13),
            Tile::new(Suit::Dragon, 1, 14),
        ];
        run.selected = vec![false; run.hand.len()];
        run.discards_remaining = 0;
        run.plays_remaining = 3;
        run.structure_sets.clear();
        run.structure_tiles.clear();

        run.refill_hand(&mut bus);

        assert!(matches!(
            bus.queue.last(),
            Some(GameEvent::GameOver {
                reason: GameOverReason::NoActionsRemaining,
            })
        ));
    }

    #[test]
    fn out_of_plays_loss_takes_precedence_over_dead_round_reason() {
        let mut run = test_run();
        let mut bus = bus();
        run.hand = vec![
            Tile::new(Suit::Characters, 1, 1),
            Tile::new(Suit::Characters, 3, 2),
            Tile::new(Suit::Characters, 5, 3),
            Tile::new(Suit::Characters, 7, 4),
            Tile::new(Suit::Characters, 9, 5),
            Tile::new(Suit::Bamboos, 2, 6),
            Tile::new(Suit::Bamboos, 4, 7),
            Tile::new(Suit::Bamboos, 6, 8),
            Tile::new(Suit::Bamboos, 8, 9),
            Tile::new(Suit::Circles, 1, 10),
            Tile::new(Suit::Circles, 3, 11),
            Tile::new(Suit::Circles, 5, 12),
            Tile::new(Suit::Wind, 1, 13),
            Tile::new(Suit::Dragon, 1, 14),
        ];
        run.selected = vec![false; run.hand.len()];
        run.discards_remaining = 0;
        run.plays_remaining = 0;

        run.refill_hand(&mut bus);

        assert!(matches!(
            bus.queue.last(),
            Some(GameEvent::GameOver {
                reason: GameOverReason::OutOfPlays,
            })
        ));
    }

    #[test]
    fn score_selected_removes_tiles_from_hand() {
        let mut run = test_run();
        let mut bus = bus();
        // Select a pair: indices 0 and 1 (1m, 1m).
        let tile0 = run.hand[0];
        let tile1 = run.hand[1];
        run.toggle_select(0);
        run.toggle_select(1);
        run.score_selected_tiles(&mut bus);
        // Those specific tiles should be gone.
        assert!(!run.hand.iter().any(|t| t.id == tile0.id));
        assert!(!run.hand.iter().any(|t| t.id == tile1.id));
    }

    #[test]
    fn is_selection_valid_reflects_state() {
        let mut run = test_run();
        assert!(!run.is_selection_valid(), "empty selection is invalid");
        // Select a triplet.
        run.toggle_select(0);
        run.toggle_select(1);
        run.toggle_select(2);
        assert!(run.is_selection_valid(), "triplet should be valid");
        // Add a leftover.
        run.toggle_select(4);
        assert!(!run.is_selection_valid(), "triplet + leftover is invalid");
    }

    // ── discard indices are correct (reverse removal) ───────────────

    #[test]
    fn discard_removes_correct_tiles_by_index() {
        let mut run = test_run();
        let mut bus = bus();

        let tile_at_2 = run.hand[2];
        let tile_at_10 = run.hand[10];

        run.toggle_select(2);
        run.toggle_select(10);
        run.discard_selected(&mut bus);

        // These specific tiles should no longer be in hand.
        assert!(!run.hand.iter().any(|t| t.id == tile_at_2.id));
        assert!(!run.hand.iter().any(|t| t.id == tile_at_10.id));
    }

    // ── Brocade Pouch: global-buff enhancement ──────────────────────────

    #[test]
    fn brocade_pouch_stamps_tiles_drawn_after_talisman_use() {
        use crate::core::consumable::Consumable;
        use crate::core::talisman::TalismanKind;
        use crate::core::tile::TileEnhancement;

        let mut run = test_run();
        let mut bus = bus();

        run.relics.active.push(RelicId::BrocadePouch);
        run.recompute_capacities();
        run.consumables
            .try_push(Consumable::Talisman(TalismanKind::Pearl));

        // Remember which tile ids are in hand *before* use; ids drawn later
        // should still pick up the enhancement via the global fallback.
        let original_ids: std::collections::HashSet<u32> = run.hand.iter().map(|t| t.id).collect();
        run.use_consumable(0, &mut bus);
        assert_eq!(run.global_buff_enhancement, Some(TileEnhancement::Pearl));

        // Discard all original-hand tiles to force the wall to hand out new ids.
        for i in 0..run.hand.len() {
            run.toggle_select(i);
        }
        run.discard_selected(&mut bus);

        // Freshly-drawn tiles (different ids) should now carry Pearl via the
        // global fallback in restamp_hand_enhancements.
        let replaced = run
            .hand
            .iter()
            .filter(|t| !original_ids.contains(&t.id))
            .count();
        assert!(replaced > 0, "wall should have handed out new ids");
        assert!(
            run.hand
                .iter()
                .filter(|t| !original_ids.contains(&t.id))
                .all(|t| t.enhancement == Some(TileEnhancement::Pearl)),
            "new tiles should inherit Pearl from global buff"
        );
    }

    #[test]
    fn brocade_pouch_does_not_apply_without_talisman_use() {
        let mut run = test_run();
        run.relics.active.push(RelicId::BrocadePouch);
        run.recompute_capacities();

        assert_eq!(run.global_buff_enhancement, None);
        assert!(run.hand.iter().all(|t| t.enhancement.is_none()));
    }

    #[test]
    fn brocade_pouch_adds_consumable_slot() {
        let mut run = test_run();
        let base = run.consumables.capacity;
        run.relics.active.push(RelicId::BrocadePouch);
        run.recompute_capacities();
        assert_eq!(run.consumables.capacity, base + 1);
    }

    #[test]
    fn buff_talisman_without_pouch_does_not_set_global() {
        use crate::core::consumable::Consumable;
        use crate::core::talisman::TalismanKind;

        let mut run = test_run();
        let mut bus = bus();
        run.consumables
            .try_push(Consumable::Talisman(TalismanKind::Pearl));
        run.use_consumable(0, &mut bus);
        assert_eq!(run.global_buff_enhancement, None);
    }
}

/// All tile faces for substitution attempts.
const ALL_FACES: [(Suit, u8); 34] = {
    let mut faces = [(Suit::Characters, 0u8); 34];
    let mut i = 0;
    let suits = [Suit::Characters, Suit::Bamboos, Suit::Circles];
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
) -> Option<(Vec<DetectedSet>, Vec<Tile>)> {
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
    let number_suits = [Suit::Characters, Suit::Bamboos, Suit::Circles];
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
) -> Option<(Vec<DetectedSet>, Vec<Tile>)> {
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
) -> Option<(Vec<DetectedSet>, Vec<Tile>)> {
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
    ) -> Option<(Vec<DetectedSet>, Vec<Tile>)> {
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
) -> Option<(Vec<DetectedSet>, Vec<Tile>)> {
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

#[cfg(test)]
mod joker_tile_tests {
    use super::*;

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn joker_completes_sequence() {
        // 1m 2m 5s — joker should turn 5s into 3m
        let tiles = vec![
            tile(Suit::Characters, 1, 0),
            tile(Suit::Characters, 2, 1),
            tile(Suit::Bamboos, 5, 2),
        ];
        let result = try_joker_substitution(&tiles, &[]);
        assert!(result.is_some(), "joker should complete the sequence");
        let (sets, modified) = result.unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Sequence);
        // The modified tile should now be 3m
        assert_eq!(modified[2].suit, Suit::Characters);
        assert_eq!(modified[2].rank, 3);
    }

    #[test]
    fn joker_completes_triplet() {
        // 7p 7p 1s — joker should turn 1s into 7p
        let tiles = vec![
            tile(Suit::Circles, 7, 0),
            tile(Suit::Circles, 7, 1),
            tile(Suit::Bamboos, 1, 2),
        ];
        let result = try_joker_substitution(&tiles, &[]);
        assert!(result.is_some());
        let (sets, _) = result.unwrap();
        assert_eq!(sets[0].kind, SetKind::Triplet);
    }

    #[test]
    fn joker_makes_pair_from_two_tiles() {
        // 1m 5s — joker turns 5s into 1m for a pair
        let tiles = vec![tile(Suit::Characters, 1, 0), tile(Suit::Bamboos, 5, 1)];
        let result = try_joker_substitution(&tiles, &[]);
        assert!(result.is_some());
        let (sets, _) = result.unwrap();
        assert_eq!(sets[0].kind, SetKind::Pair);
    }

    #[test]
    fn joker_only_substitutes_one_tile() {
        // 1m 5s 9p — all different, need 2 subs to make a meld, joker can only do 1
        let tiles = vec![
            tile(Suit::Characters, 1, 0),
            tile(Suit::Bamboos, 5, 1),
            tile(Suit::Circles, 9, 2),
        ];
        assert!(try_joker_substitution(&tiles, &[]).is_none());
    }

    #[test]
    fn joker_respects_no_sequences_rule() {
        // 1m 2m 5s — would be a sequence with joker, but NoSequences blocks it
        let tiles = vec![
            tile(Suit::Characters, 1, 0),
            tile(Suit::Characters, 2, 1),
            tile(Suit::Bamboos, 5, 2),
        ];
        let result = try_joker_substitution(&tiles, &[RuleModifier::NoSequences]);
        // Could still work if joker turns 5s into 1m or 2m for a triplet — but
        // we only have 2 of those, so a triplet needs the joker tile to match one.
        // 1m 2m 1m → not a valid decomposition (pair 1m + leftover 2m).
        // 1m 2m 2m → pair 2m + leftover 1m. Also invalid.
        // No triplet possible, so should be None.
        assert!(result.is_none());
    }
}

#[cfg(test)]
mod wild_wind_tests {
    use super::*;

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn two_winds_substitute_into_sequences() {
        // Hand: 2m W 4m | 7m 8m 9m | 4s 5s 6s | 7p 8p W
        // With Wild Winds, W->3m and W->9p (or 6p) should yield 4 sequences.
        let tiles = vec![
            tile(Suit::Characters, 2, 1),
            tile(Suit::Wind, 3, 2), // West, should become 3m
            tile(Suit::Characters, 4, 3),
            tile(Suit::Characters, 7, 4),
            tile(Suit::Characters, 8, 5),
            tile(Suit::Characters, 9, 6),
            tile(Suit::Bamboos, 4, 7),
            tile(Suit::Bamboos, 5, 8),
            tile(Suit::Bamboos, 6, 9),
            tile(Suit::Circles, 7, 10),
            tile(Suit::Circles, 8, 11),
            tile(Suit::Wind, 3, 12), // West, should become 9p (or 6p)
        ];
        let result = try_wind_substitution(&tiles, &[]);
        assert!(
            result.is_some(),
            "two-wind substitution should find a valid hand"
        );
        let (sets, _) = result.unwrap();
        assert_eq!(sets.len(), 4);
        assert!(sets.iter().all(|s| s.kind == SetKind::Sequence));
    }

    #[test]
    fn single_wind_substitutes_into_sequence() {
        // 1m 2m W -> W becomes 3m
        let tiles = vec![
            tile(Suit::Characters, 1, 1),
            tile(Suit::Characters, 2, 2),
            tile(Suit::Wind, 1, 3), // East
        ];
        let result = try_wind_substitution(&tiles, &[]);
        assert!(result.is_some());
        let (sets, _) = result.unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Sequence);
    }

    #[test]
    fn wind_substitutes_into_triplet() {
        // 5s 5s W -> W becomes 5s for a triplet
        let tiles = vec![
            tile(Suit::Bamboos, 5, 1),
            tile(Suit::Bamboos, 5, 2),
            tile(Suit::Wind, 2, 3),
        ];
        let result = try_wind_substitution(&tiles, &[]);
        assert!(result.is_some());
        let (sets, _) = result.unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Triplet);
    }

    #[test]
    fn no_winds_returns_none() {
        let tiles = vec![
            tile(Suit::Characters, 1, 1),
            tile(Suit::Characters, 2, 2),
            tile(Suit::Characters, 3, 3),
        ];
        assert!(try_wind_substitution(&tiles, &[]).is_none());
    }

    #[test]
    fn impossible_hand_returns_none() {
        // W alone can't form any meld
        let tiles = vec![tile(Suit::Wind, 1, 1)];
        assert!(try_wind_substitution(&tiles, &[]).is_none());
    }

    #[test]
    fn candidates_include_nearby_ranks() {
        let tiles = vec![tile(Suit::Characters, 5, 1), tile(Suit::Wind, 3, 2)];
        let candidates = wind_candidate_faces(&tiles);
        // Should include 3m-7m (5 ± 2) and 5m itself
        for r in 3..=7 {
            assert!(
                candidates.contains(&(Suit::Characters, r)),
                "candidates should include {}m",
                r
            );
        }
        // Should NOT include 1m (too far)
        assert!(!candidates.contains(&(Suit::Characters, 1)));
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        const NUMBER_SUITS: [Suit; 3] = [Suit::Characters, Suit::Bamboos, Suit::Circles];

        fn arb_number_tile(id: u32) -> BoxedStrategy<Tile> {
            (0..3usize, 1..=9u8)
                .prop_map(move |(si, rank)| Tile::new(NUMBER_SUITS[si], rank, id))
                .boxed()
        }

        fn arb_wind_tile(id: u32) -> BoxedStrategy<Tile> {
            (1..=4u8)
                .prop_map(move |rank| Tile::new(Suit::Wind, rank, id))
                .boxed()
        }

        fn arb_dragon_tile(id: u32) -> BoxedStrategy<Tile> {
            (1..=3u8)
                .prop_map(move |rank| Tile::new(Suit::Dragon, rank, id))
                .boxed()
        }

        /// Mixed hand with at least one wind tile, 3..=9 tiles total.
        ///
        /// The upper bound is deliberately below 14. `try_wind_substitution`
        /// is combinatorial in the number of winds × candidate faces, so the
        /// legacy 4-winds + 10-other worst case could take tens of seconds
        /// per proptest case. The shape we actually want to cover —
        /// substitution behaves consistently across small, medium, and larger
        /// wind-heavy hands — is still exercised here without making the
        /// test suite unusable.
        fn arb_wind_hand() -> BoxedStrategy<Vec<Tile>> {
            (1usize..=3, 2usize..=6)
                .prop_flat_map(|(n_winds, n_other)| {
                    let wind_strats: Vec<BoxedStrategy<Tile>> =
                        (0..n_winds).map(|i| arb_wind_tile(i as u32)).collect();
                    let other_strats: Vec<BoxedStrategy<Tile>> = (0..n_other)
                        .map(|i| {
                            let id = (n_winds + i) as u32;
                            prop_oneof![
                                arb_number_tile(id),
                                arb_wind_tile(id),
                                arb_dragon_tile(id),
                            ]
                            .boxed()
                        })
                        .collect();
                    (wind_strats, other_strats).prop_map(|(mut w, o)| {
                        w.extend(o);
                        w
                    })
                })
                .boxed()
        }

        /// Extract the multiset of (suit, rank) faces assigned to the wind tiles
        /// in `original` after substitution into `modified`.
        fn wind_face_multiset(original: &[Tile], modified: &[Tile]) -> Vec<(Suit, u8)> {
            let wind_ids: std::collections::HashSet<u32> = original
                .iter()
                .filter(|t| t.suit == Suit::Wind)
                .map(|t| t.id)
                .collect();
            let mut faces: Vec<(Suit, u8)> = modified
                .iter()
                .filter(|m| wind_ids.contains(&m.id))
                .map(|m| (m.suit, m.rank))
                .collect();
            faces.sort();
            faces
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

            // ── Property: permutation invariance (multiset) ───────────
            //
            // Reordering the input tiles must not change the *multiset* of faces
            // assigned to wind tiles. The per-id assignment can still vary when
            // multiple valid substitutions exist (e.g. three East winds with
            // 2m could become {1m,1m,2m,2m} with either wind taking which rank),
            // but the set of faces produced should be invariant. The old
            // HashSet-backed candidate list could pick structurally different
            // substitutions based on hash iteration order — this property would
            // catch that regression.
            #[test]
            fn permutation_invariance(
                tiles in arb_wind_hand(),
                perm_seed in any::<u64>(),
            ) {
                use rand::SeedableRng;
                use rand::seq::SliceRandom;

                let Some((_sets_a, modified_a)) = try_wind_substitution(&tiles, &[]) else {
                    return Ok(());
                };

                let mut shuffled = tiles.clone();
                let mut rng = rand::rngs::StdRng::seed_from_u64(perm_seed);
                shuffled.shuffle(&mut rng);

                let Some((_sets_b, modified_b)) = try_wind_substitution(&shuffled, &[]) else {
                    prop_assert!(
                        false,
                        "permuted hand rejected while original accepted: {:?} -> {:?}",
                        tiles,
                        shuffled
                    );
                    return Ok(());
                };

                let faces_a = wind_face_multiset(&tiles, &modified_a);
                let faces_b = wind_face_multiset(&shuffled, &modified_b);
                prop_assert_eq!(
                    faces_a,
                    faces_b,
                    "wind face multiset depends on input order (tiles={:?})",
                    tiles
                );
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

            // ── Property: substitution output re-validates ────────────
            //
            // If substitution returns (sets, modified), the modified tile list
            // must itself validate without any further wildcard magic — otherwise
            // the scorer is handed melds that don't actually match the tiles.
            #[test]
            fn substitution_output_revalidates(tiles in arb_wind_hand()) {
                if let Some((sets, modified)) = try_wind_substitution(&tiles, &[]) {
                    let revalidated = crate::core::hand::validate_selection_with_rules(&modified, &[]);
                    prop_assert!(
                        revalidated.is_some(),
                        "substitution output failed to revalidate: modified={:?}",
                        modified
                    );
                    let revalidated = revalidated.unwrap();
                    prop_assert_eq!(sets.len(), revalidated.len(), "set count mismatch");
                }
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

            // ── Property: tile IDs preserved exactly ──────────────────
            //
            // Substitution rewrites face (suit, rank) but must never drop,
            // duplicate, or invent tile IDs.
            #[test]
            fn ids_preserved(tiles in arb_wind_hand()) {
                if let Some((_sets, modified)) = try_wind_substitution(&tiles, &[]) {
                    let mut input_ids: Vec<u32> = tiles.iter().map(|t| t.id).collect();
                    let mut output_ids: Vec<u32> = modified.iter().map(|t| t.id).collect();
                    input_ids.sort();
                    output_ids.sort();
                    prop_assert_eq!(input_ids, output_ids);
                }
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

            // ── Property: non-wind tiles unchanged ────────────────────
            //
            // Only wind tiles may be rewritten. A bug that substitutes the wrong
            // index would show up here.
            #[test]
            fn non_winds_unchanged(tiles in arb_wind_hand()) {
                if let Some((_sets, modified)) = try_wind_substitution(&tiles, &[]) {
                    for orig in &tiles {
                        if orig.suit != Suit::Wind {
                            let m = modified.iter().find(|m| m.id == orig.id).unwrap();
                            prop_assert_eq!(
                                (m.suit, m.rank),
                                (orig.suit, orig.rank),
                                "non-wind tile id={} was rewritten",
                                orig.id
                            );
                        }
                    }
                }
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

            // ── Property: no panics on arbitrary input ────────────────
            #[test]
            fn no_panic_on_arbitrary_hand(tiles in arb_wind_hand()) {
                let _ = try_wind_substitution(&tiles, &[]);
            }
        }
    }
}

#[cfg(test)]
mod progression_snapshot_tests {
    use super::*;

    #[test]
    fn run_relic_unlocks_only_change_when_a_new_run_applies_progression() {
        let mut progress = crate::core::progression::PlayerProgress::new();
        progress.runs_completed = 6;
        progress.check_level_up();

        let mut current_run = RunState::new_demo();
        current_run.apply_progression(&progress);
        assert!(!current_run.available_relics.contains(&RelicId::JokerTile));

        progress.runs_completed = 10;
        let result = progress
            .check_level_up()
            .expect("level 5 should unlock relics");
        assert!(result.relics.contains(&RelicId::JokerTile));
        assert!(!current_run.available_relics.contains(&RelicId::JokerTile));

        let mut next_run = RunState::new_demo();
        next_run.apply_progression(&progress);
        assert!(next_run.available_relics.contains(&RelicId::JokerTile));
    }
}

#[cfg(test)]
mod disgust_tests {
    use super::*;

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn ew_validates_as_pair() {
        let tiles = vec![tile(Suit::Wind, 1, 0), tile(Suit::Wind, 3, 1)];
        let (sets, _) = try_disgust_substitution(&tiles, &[], false).expect("EW should be a pair");
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Pair);
    }

    #[test]
    fn eww_validates_as_triplet() {
        let tiles = vec![
            tile(Suit::Wind, 1, 0),
            tile(Suit::Wind, 3, 1),
            tile(Suit::Wind, 3, 2),
        ];
        let (sets, _) =
            try_disgust_substitution(&tiles, &[], false).expect("EWW should be a triplet");
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Triplet);
    }

    #[test]
    fn ewww_validates_as_kong() {
        let tiles = vec![
            tile(Suit::Wind, 1, 0),
            tile(Suit::Wind, 3, 1),
            tile(Suit::Wind, 3, 2),
            tile(Suit::Wind, 3, 3),
        ];
        let (sets, _) =
            try_disgust_substitution(&tiles, &[], false).expect("EWWW should be a kong");
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Kong);
    }

    #[test]
    fn requires_both_east_and_west() {
        // No East: should not fire.
        let tiles = vec![tile(Suit::Wind, 3, 0), tile(Suit::Wind, 3, 1)];
        assert!(try_disgust_substitution(&tiles, &[], false).is_none());
        // No West: should not fire.
        let tiles = vec![tile(Suit::Wind, 1, 0), tile(Suit::Wind, 1, 1)];
        assert!(try_disgust_substitution(&tiles, &[], false).is_none());
    }

    #[test]
    fn nonsense_selection_still_invalid() {
        // EW + a stray bamboo cannot decompose even after relabel.
        let tiles = vec![
            tile(Suit::Wind, 1, 0),
            tile(Suit::Wind, 3, 1),
            tile(Suit::Bamboos, 5, 2),
        ];
        assert!(try_disgust_substitution(&tiles, &[], false).is_none());
    }
}
