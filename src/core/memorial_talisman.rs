//! Memorial (remnant) talismans — one per run, granted from the previous defeat.
//!
//! Shop talismans live in [`crate::core::talisman`]. Memorials are a parallel
//! series: not bought in the shop (granted after defeat), sellable for
//! [`MemorialTalismanKind::SHOP_SELL_PRICE`]. Selected from [`RunDefeatJournal`]
//! at game over and carried into the next run in a normal consumable slot.
//! Effects are **in-round**
//! only (current blind), applied on dish use.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::core::json_asset::load_json_asset;
use crate::core::rules::BlindKind;
use crate::core::talisman::TalismanKind;
use crate::core::tile::{Suit, TileEnhancement};
use crate::core::yaku::YakuKind;
use crate::game::event_bus::GameOverReason;

#[derive(Deserialize)]
struct MemorialPresentationRaw {
    id: MemorialTalismanKind,
    name: String,
    description: String,
    accent: [f32; 4],
}

struct MemorialPresentation {
    name: &'static str,
    description: &'static str,
    accent: [f32; 4],
}

fn memorial_presentations() -> &'static HashMap<MemorialTalismanKind, MemorialPresentation> {
    static MAP: OnceLock<HashMap<MemorialTalismanKind, MemorialPresentation>> = OnceLock::new();
    MAP.get_or_init(|| {
        const PATH: &str = "data/memorial_talismans.json";
        let raw: Vec<MemorialPresentationRaw> = load_json_asset(PATH, "memorial talisman data");
        raw.into_iter()
            .map(|r| {
                (
                    r.id,
                    MemorialPresentation {
                        name: Box::leak(r.name.into_boxed_str()),
                        description: Box::leak(r.description.into_boxed_str()),
                        accent: r.accent,
                    },
                )
            })
            .collect()
    })
}

fn presentation(kind: MemorialTalismanKind) -> &'static MemorialPresentation {
    memorial_presentations()
        .get(&kind)
        .unwrap_or_else(|| panic!("memorial talisman data missing for {kind:?}"))
}

/// Fixed roster of remnants left behind after defeat.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorialTalismanKind {
    Exhausted,
    FrozenHand,
    Skipper,
    Hoarder,
    FullDish,
    Discarded,
    BossMark,
    BuffSaint,
    Transformer,
    TagBearer,
    MeldMason,
    DeepWalker,
}

impl MemorialTalismanKind {
    pub fn all() -> &'static [MemorialTalismanKind] {
        &[
            Self::Exhausted,
            Self::FrozenHand,
            Self::Skipper,
            Self::Hoarder,
            Self::FullDish,
            Self::Discarded,
            Self::BossMark,
            Self::BuffSaint,
            Self::Transformer,
            Self::TagBearer,
            Self::MeldMason,
            Self::DeepWalker,
        ]
    }

    /// Flat sell price when the player discards a remnant from the shop inventory.
    pub const SHOP_SELL_PRICE: u32 = 4;

    pub fn name(self) -> &'static str {
        presentation(self).name
    }

    pub fn description(self) -> &'static str {
        presentation(self).description
    }

    pub fn accent_color(self) -> [f32; 4] {
        presentation(self).accent
    }

    /// JSON / asset stem (`exhausted`, `frozen_hand`, …).
    pub fn asset_slug(self) -> &'static str {
        match self {
            Self::Exhausted => "exhausted",
            Self::FrozenHand => "frozen_hand",
            Self::Skipper => "skipper",
            Self::Hoarder => "hoarder",
            Self::FullDish => "full_dish",
            Self::Discarded => "discarded",
            Self::BossMark => "boss_mark",
            Self::BuffSaint => "buff_saint",
            Self::Transformer => "transformer",
            Self::TagBearer => "tag_bearer",
            Self::MeldMason => "meld_mason",
            Self::DeepWalker => "deep_walker",
        }
    }

    /// Grayscale relief heightmap for the octagonal tablet mesh (`lit_mesh` chitin).
    pub fn heightmap_asset_path(self) -> &'static str {
        match self {
            Self::Exhausted => "textures/talismans/memorial_exhausted.png",
            Self::FrozenHand => "textures/talismans/memorial_frozen_hand.png",
            Self::Skipper => "textures/talismans/memorial_skipper.png",
            Self::Hoarder => "textures/talismans/memorial_hoarder.png",
            Self::FullDish => "textures/talismans/memorial_full_dish.png",
            Self::Discarded => "textures/talismans/memorial_discarded.png",
            Self::BossMark => "textures/talismans/memorial_boss_mark.png",
            Self::BuffSaint => "textures/talismans/memorial_buff_saint.png",
            Self::Transformer => "textures/talismans/memorial_transformer.png",
            Self::TagBearer => "textures/talismans/memorial_tag_bearer.png",
            Self::MeldMason => "textures/talismans/memorial_meld_mason.png",
            Self::DeepWalker => "textures/talismans/memorial_deep_walker.png",
        }
    }

    /// `(path, gpu label)` pairs in [`Self::all()`] order — see `build.rs` loader.
    pub fn heightmap_paths() -> &'static [(&'static str, &'static str)] {
        &[
            ("textures/talismans/memorial_exhausted.png", "memorial-exhausted-hm"),
            (
                "textures/talismans/memorial_frozen_hand.png",
                "memorial-frozen-hand-hm",
            ),
            ("textures/talismans/memorial_skipper.png", "memorial-skipper-hm"),
            ("textures/talismans/memorial_hoarder.png", "memorial-hoarder-hm"),
            (
                "textures/talismans/memorial_full_dish.png",
                "memorial-full-dish-hm",
            ),
            (
                "textures/talismans/memorial_discarded.png",
                "memorial-discarded-hm",
            ),
            (
                "textures/talismans/memorial_boss_mark.png",
                "memorial-boss-mark-hm",
            ),
            (
                "textures/talismans/memorial_buff_saint.png",
                "memorial-buff-saint-hm",
            ),
            (
                "textures/talismans/memorial_transformer.png",
                "memorial-transformer-hm",
            ),
            (
                "textures/talismans/memorial_tag_bearer.png",
                "memorial-tag-bearer-hm",
            ),
            (
                "textures/talismans/memorial_meld_mason.png",
                "memorial-meld-mason-hm",
            ),
            (
                "textures/talismans/memorial_deep_walker.png",
                "memorial-deep-walker-hm",
            ),
        ]
    }

    /// Octagon silhouette mask (white = tablet, black = void) for chitin discard.
    pub fn mask_asset_path(self) -> &'static str {
        match self {
            Self::Exhausted => "textures/talismans/memorial_exhausted_mask.png",
            Self::FrozenHand => "textures/talismans/memorial_frozen_hand_mask.png",
            Self::Skipper => "textures/talismans/memorial_skipper_mask.png",
            Self::Hoarder => "textures/talismans/memorial_hoarder_mask.png",
            Self::FullDish => "textures/talismans/memorial_full_dish_mask.png",
            Self::Discarded => "textures/talismans/memorial_discarded_mask.png",
            Self::BossMark => "textures/talismans/memorial_boss_mark_mask.png",
            Self::BuffSaint => "textures/talismans/memorial_buff_saint_mask.png",
            Self::Transformer => "textures/talismans/memorial_transformer_mask.png",
            Self::TagBearer => "textures/talismans/memorial_tag_bearer_mask.png",
            Self::MeldMason => "textures/talismans/memorial_meld_mason_mask.png",
            Self::DeepWalker => "textures/talismans/memorial_deep_walker_mask.png",
        }
    }

    pub fn mask_paths() -> &'static [(&'static str, &'static str)] {
        &[
            (
                "textures/talismans/memorial_exhausted_mask.png",
                "memorial-exhausted-mask",
            ),
            (
                "textures/talismans/memorial_frozen_hand_mask.png",
                "memorial-frozen-hand-mask",
            ),
            (
                "textures/talismans/memorial_skipper_mask.png",
                "memorial-skipper-mask",
            ),
            (
                "textures/talismans/memorial_hoarder_mask.png",
                "memorial-hoarder-mask",
            ),
            (
                "textures/talismans/memorial_full_dish_mask.png",
                "memorial-full-dish-mask",
            ),
            (
                "textures/talismans/memorial_discarded_mask.png",
                "memorial-discarded-mask",
            ),
            (
                "textures/talismans/memorial_boss_mark_mask.png",
                "memorial-boss-mark-mask",
            ),
            (
                "textures/talismans/memorial_buff_saint_mask.png",
                "memorial-buff-saint-mask",
            ),
            (
                "textures/talismans/memorial_transformer_mask.png",
                "memorial-transformer-mask",
            ),
            (
                "textures/talismans/memorial_tag_bearer_mask.png",
                "memorial-tag-bearer-mask",
            ),
            (
                "textures/talismans/memorial_meld_mason_mask.png",
                "memorial-meld-mason-mask",
            ),
            (
                "textures/talismans/memorial_deep_walker_mask.png",
                "memorial-deep-walker-mask",
            ),
        ]
    }

    /// One-line flavor for the defeat screen from journal habits.
    pub fn defeat_subtitle(self, _journal: &MemorialJournalSnapshot) -> &'static str {
        match self {
            Self::Exhausted => "You ran out of plays.",
            Self::FrozenHand => "No legal move remained.",
            Self::Skipper => "You kept walking past the blinds.",
            Self::Hoarder => "Credits stayed in your purse.",
            Self::FullDish => "You never used your items.",
            Self::Discarded => "Perhaps you discarded too many tiles.",
            Self::BossMark => "You met your end at the hands of a boss.",
            Self::BuffSaint => "You buffed every tile you could.",
            Self::Transformer => "You reshaped the hand again and again.",
            Self::TagBearer => "You took every token the House offered.",
            Self::MeldMason => "One pattern ruled your run.",
            Self::DeepWalker => "Your reach exceeded your grasp.",
        }
    }
}

/// Habits recorded during a run; used to pick the next remnant.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunDefeatJournal {
    #[serde(default)]
    pub blinds_skipped: u32,
    #[serde(default)]
    pub shop_talisman_buys: u32,
    #[serde(default)]
    pub shop_relic_buys: u32,
    #[serde(default)]
    pub shop_pack_buys: u32,
    #[serde(default)]
    pub talisman_uses: u32,
    #[serde(default)]
    pub zodiac_uses: u32,
    #[serde(default)]
    pub tags_taken: u32,
    #[serde(default)]
    pub buff_talisman_uses: HashMap<TalismanKind, u32>,
    #[serde(default)]
    pub transform_talisman_uses: HashMap<TalismanKind, u32>,
}

impl RunDefeatJournal {
    pub fn record_talisman_use(&mut self, kind: TalismanKind) {
        self.talisman_uses = self.talisman_uses.saturating_add(1);
        if kind.enhancement().is_some() {
            *self.buff_talisman_uses.entry(kind).or_insert(0) += 1;
        } else {
            *self.transform_talisman_uses.entry(kind).or_insert(0) += 1;
        }
    }

    pub fn dominant_buff_talisman(&self) -> Option<TalismanKind> {
        self.buff_talisman_uses
            .iter()
            .max_by_key(|(_, c)| *c)
            .map(|(k, _)| *k)
    }

    pub fn dominant_transform_talisman(&self) -> Option<TalismanKind> {
        self.transform_talisman_uses
            .iter()
            .max_by_key(|(_, c)| *c)
            .map(|(k, _)| *k)
    }
}

/// Snapshot frozen at defeat for remnant selection and use-time scaling.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemorialJournalSnapshot {
    pub journal: RunDefeatJournal,
    pub loss_reason: GameOverReason,
    pub final_ante: u32,
    pub final_blind: BlindKind,
    pub final_gold: i32,
    pub tiles_played: u32,
    pub tiles_discarded: u32,
    pub consumables_unused: u32,
    pub dominant_yaku: Option<YakuKind>,
}

/// Per-blind state from memorial use; cleared when a new blind starts.
#[derive(Clone, Debug, Default)]
pub struct MemorialRoundState {
    /// Extra gold added on blind clear (Skipper).
    pub clear_gold_bonus: u32,
    /// Bonus chips on next structure cash-in.
    pub next_cashin_bonus_chips: u64,
    /// Only apply cash-in bonus if this yaku is in the hand (Meld Mason).
    pub next_cashin_yaku: Option<YakuKind>,
}

impl MemorialRoundState {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Pick which remnant the player becomes / receives next run.
///
/// Priority: boss death, then dominant habits, then loss reason fallback.
pub fn select_memorial(snapshot: &MemorialJournalSnapshot) -> MemorialTalismanKind {
    let j = &snapshot.journal;
    if snapshot.final_blind == BlindKind::Boss {
        return MemorialTalismanKind::BossMark;
    }
    if j.blinds_skipped >= 2 {
        return MemorialTalismanKind::Skipper;
    }
    if snapshot.final_gold >= 20
        && j.shop_talisman_buys + j.shop_relic_buys + j.shop_pack_buys >= 3
    {
        return MemorialTalismanKind::Hoarder;
    }
    if snapshot.consumables_unused > 0 && j.talisman_uses == 0 && j.zodiac_uses == 0 {
        return MemorialTalismanKind::FullDish;
    }
    if snapshot.tiles_discarded > snapshot.tiles_played.saturating_mul(3) / 2
        && snapshot.tiles_discarded >= 8
    {
        return MemorialTalismanKind::Discarded;
    }
    if j.dominant_buff_talisman().is_some() {
        return MemorialTalismanKind::BuffSaint;
    }
    if j.dominant_transform_talisman().is_some() {
        return MemorialTalismanKind::Transformer;
    }
    if j.tags_taken >= 1 {
        return MemorialTalismanKind::TagBearer;
    }
    if snapshot.dominant_yaku.is_some() {
        return MemorialTalismanKind::MeldMason;
    }
    if snapshot.final_ante >= 4 {
        return MemorialTalismanKind::DeepWalker;
    }
    match snapshot.loss_reason {
        GameOverReason::OutOfPlays => MemorialTalismanKind::Exhausted,
        GameOverReason::NoActionsRemaining => MemorialTalismanKind::FrozenHand,
    }
}

/// Dominant yaku from run counters.
pub fn dominant_yaku_from_run(
    yaku_times: &rustc_hash::FxHashMap<YakuKind, u32>,
) -> Option<YakuKind> {
    yaku_times
        .iter()
        .max_by_key(|(_, c)| *c)
        .filter(|(_, c)| **c > 0)
        .map(|(y, _)| *y)
}

/// Build snapshot at defeat from live run state.
pub fn snapshot_from_run(
    journal: &RunDefeatJournal,
    reason: GameOverReason,
    run: &crate::game::run::RunState,
) -> MemorialJournalSnapshot {
    let consumables_unused = run.consumables.items.len() as u32;
    MemorialJournalSnapshot {
        journal: journal.clone(),
        loss_reason: reason,
        final_ante: run.ante,
        final_blind: run.blind,
        final_gold: run.gold,
        tiles_played: run.tiles_played,
        tiles_discarded: run.tiles_discarded,
        consumables_unused,
        dominant_yaku: dominant_yaku_from_run(&run.yaku_times_played),
    }
}

/// Enhancement echoed by Buff Saint from the frozen journal.
pub fn buff_saint_enhancement(snapshot: &MemorialJournalSnapshot) -> TileEnhancement {
    snapshot
        .journal
        .dominant_buff_talisman()
        .and_then(|k| k.enhancement())
        .unwrap_or(TileEnhancement::Pearl)
}

/// Suit bias for Transformer remnant.
pub fn transformer_target_suit(snapshot: &MemorialJournalSnapshot) -> Suit {
    match snapshot.journal.dominant_transform_talisman() {
        Some(TalismanKind::Souzu) => Suit::Souzu,
        Some(TalismanKind::Pinzu) => Suit::Pinzu,
        Some(TalismanKind::Manzu) => Suit::Manzu,
        _ => Suit::Souzu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::rules::BlindKind;

    fn snapshot_with(journal: RunDefeatJournal, reason: GameOverReason) -> MemorialJournalSnapshot {
        MemorialJournalSnapshot {
            journal,
            loss_reason: reason,
            final_ante: 1,
            final_blind: BlindKind::Small,
            final_gold: 0,
            tiles_played: 0,
            tiles_discarded: 0,
            consumables_unused: 0,
            dominant_yaku: None,
        }
    }

    #[test]
    fn boss_death_selects_boss_mark() {
        let mut s = snapshot_with(RunDefeatJournal::default(), GameOverReason::OutOfPlays);
        s.final_blind = BlindKind::Boss;
        assert_eq!(select_memorial(&s), MemorialTalismanKind::BossMark);
    }

    #[test]
    fn out_of_plays_fallback_exhausted() {
        let s = snapshot_with(RunDefeatJournal::default(), GameOverReason::OutOfPlays);
        assert_eq!(select_memorial(&s), MemorialTalismanKind::Exhausted);
    }

    #[test]
    fn no_actions_fallback_frozen_hand() {
        let s = snapshot_with(
            RunDefeatJournal::default(),
            GameOverReason::NoActionsRemaining,
        );
        assert_eq!(select_memorial(&s), MemorialTalismanKind::FrozenHand);
    }

    #[test]
    fn skipper_when_many_skips() {
        let mut j = RunDefeatJournal::default();
        j.blinds_skipped = 3;
        let s = snapshot_with(j, GameOverReason::NoActionsRemaining);
        assert_eq!(select_memorial(&s), MemorialTalismanKind::Skipper);
    }

    #[test]
    fn heightmap_and_mask_paths_match_all_kinds() {
        let all = MemorialTalismanKind::all();
        assert_eq!(MemorialTalismanKind::heightmap_paths().len(), all.len());
        assert_eq!(MemorialTalismanKind::mask_paths().len(), all.len());
        for (i, &k) in all.iter().enumerate() {
            let (hm, _) = MemorialTalismanKind::heightmap_paths()[i];
            let (mask, _) = MemorialTalismanKind::mask_paths()[i];
            assert!(
                hm.ends_with(&format!("memorial_{}.png", k.asset_slug())),
                "height path order mismatch for {k:?}"
            );
            assert!(
                mask.ends_with(&format!("memorial_{}_mask.png", k.asset_slug())),
                "mask path order mismatch for {k:?}"
            );
        }
    }

    #[test]
    fn every_memorial_variant_has_one_data_entry() {
        let map = memorial_presentations();
        assert_eq!(map.len(), MemorialTalismanKind::all().len());
        for &k in MemorialTalismanKind::all() {
            let _ = presentation(k);
        }
    }
}
