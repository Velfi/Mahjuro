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

use rand::RngExt;
use serde::{Deserialize, Serialize};

use crate::core::json_asset::load_json_asset;
use crate::core::rules::ChamberKind;
use crate::core::talisman::TalismanKind;
use crate::core::tile::{Suit, TileEnhancement};
use crate::core::yaku::YakuKind;
use mahjuro_types::GameOverReason;

#[derive(Deserialize)]
struct MemorialPresentationRaw {
    id: MemorialTalismanKind,
    name: String,
    description: String,
    accent: [f32; 4],
}

struct MemorialPresentation {
    name: &'static str,
    description_template: &'static str,
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
                        description_template: Box::leak(r.description.into_boxed_str()),
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
    /// Defeat in the first two chambers of a run (`run_number` ≤ 2).
    DeadOnArrival,
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
            Self::DeadOnArrival,
        ]
    }

    /// Default sell price when the player discards a remnant from the shop inventory.
    pub const SHOP_SELL_PRICE: u32 = 4;

    /// Immediate yen from using The Hoarder (also its shop sell price).
    pub const HOARDER_YEN: u32 = 6;

    /// Sell price for this remnant in the shop inventory.
    pub fn shop_sell_price(self) -> u32 {
        match self {
            Self::Hoarder => Self::HOARDER_YEN,
            _ => Self::SHOP_SELL_PRICE,
        }
    }

    pub fn name(self) -> &'static str {
        presentation(self).name
    }

    /// Tooltip / inspect copy; pass the defeat journal snapshot when available.
    pub fn description(self, snapshot: Option<&MemorialJournalSnapshot>) -> String {
        crate::core::memorial_desc_template::expand_memorial_description_templates(
            self,
            presentation(self).description_template,
            snapshot,
        )
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
            Self::DeadOnArrival => "dead_on_arrival",
        }
    }

    /// Grayscale relief heightmap for the carved pendant mesh (`lit_mesh` chitin).
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
            Self::DeadOnArrival => "textures/talismans/memorial_dead_on_arrival.png",
        }
    }

    /// `(path, gpu label)` pairs in [`Self::all()`] order — see `build.rs` loader.
    pub fn heightmap_paths() -> &'static [(&'static str, &'static str)] {
        &[
            (
                "textures/talismans/memorial_exhausted.png",
                "memorial-exhausted-hm",
            ),
            (
                "textures/talismans/memorial_frozen_hand.png",
                "memorial-frozen-hand-hm",
            ),
            (
                "textures/talismans/memorial_skipper.png",
                "memorial-skipper-hm",
            ),
            (
                "textures/talismans/memorial_hoarder.png",
                "memorial-hoarder-hm",
            ),
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
            (
                "textures/talismans/memorial_dead_on_arrival.png",
                "memorial-dead-on-arrival-hm",
            ),
        ]
    }

    /// Organic pendant silhouette mask (white = jade, black = void) for chitin discard.
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
            Self::DeadOnArrival => "textures/talismans/memorial_dead_on_arrival_mask.png",
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
            (
                "textures/talismans/memorial_dead_on_arrival_mask.png",
                "memorial-dead-on-arrival-mask",
            ),
        ]
    }

    /// Memorial talismans that can avert the given defeat reason, in preferred
    /// auto-use order when the round would otherwise end.
    pub fn salvage_candidates(reason: GameOverReason) -> &'static [MemorialTalismanKind] {
        match reason {
            GameOverReason::OutOfPlays => &[
                Self::DeadOnArrival,
                Self::Exhausted,
                Self::BossMark,
                Self::TagBearer,
            ],
            GameOverReason::NoActionsRemaining => &[
                Self::DeadOnArrival,
                Self::FrozenHand,
                Self::FullDish,
                Self::Discarded,
                Self::TagBearer,
            ],
        }
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
            Self::DeadOnArrival => "You fell in the first halls.",
        }
    }
}

/// Habits recorded during a run; used to pick the next remnant.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunDefeatJournal {
    #[serde(default)]
    pub chambers_skipped: u32,
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
    pub final_wing: u32,
    pub final_chamber: ChamberKind,
    pub final_yen: i32,
    pub tiles_played: u32,
    pub tiles_discarded: u32,
    pub consumables_unused: u32,
    pub dominant_yaku: Option<YakuKind>,
    /// Chamber index within the run at defeat (1 = first chamber faced).
    #[serde(default)]
    pub run_number: Option<u32>,
}

/// Yen added on blind clear when using The Skipper (¥4 base + ¥1 per blind skipped last run).
pub const SKIPPER_CLEAR_YEN_BASE: u32 = 4;
pub const SKIPPER_CLEAR_YEN_CAP: u32 = 12;

/// Bonus yen on blind clear from The Skipper.
pub fn skipper_clear_yen_bonus(snapshot: Option<&MemorialJournalSnapshot>) -> u32 {
    let skipped = snapshot.map(|s| s.journal.chambers_skipped).unwrap_or(0);
    SKIPPER_CLEAR_YEN_BASE
        .saturating_add(skipped)
        .min(SKIPPER_CLEAR_YEN_CAP)
}

/// Extra discards from The Discarded (`tiles_discarded / 10`, clamped 1–3).
pub fn discarded_extra_discards(snapshot: Option<&MemorialJournalSnapshot>) -> u32 {
    let journal_discards = snapshot.map(|s| s.tiles_discarded).unwrap_or(0);
    (journal_discards / 10).clamp(1, 3)
}

/// Per-blind state from memorial use; cleared when a new blind starts.
#[derive(Clone, Debug, Default)]
pub struct MemorialRoundState {
    /// Extra yen added on blind clear (Skipper).
    pub clear_yen_bonus: u32,
    /// Bonus Fu on next structure cash-in.
    pub next_cashin_bonus_fu: u64,
    /// Only apply cash-in bonus if this yaku is in the hand (Meld Mason).
    pub next_cashin_yaku: Option<YakuKind>,
}

impl MemorialRoundState {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Whether defeat happened in the first two chambers of the run.
pub fn is_first_two_chambers(run_number: u32) -> bool {
    run_number <= 2
}

/// All remnants the player qualifies for from the frozen defeat snapshot.
pub fn qualified_memorials(snapshot: &MemorialJournalSnapshot) -> Vec<MemorialTalismanKind> {
    let j = &snapshot.journal;
    let mut qualified = Vec::new();
    if snapshot.final_chamber == ChamberKind::Ordeal {
        qualified.push(MemorialTalismanKind::BossMark);
    }
    if snapshot
        .run_number
        .is_some_and(|n| is_first_two_chambers(n))
    {
        qualified.push(MemorialTalismanKind::DeadOnArrival);
    }
    if j.chambers_skipped >= 2 {
        qualified.push(MemorialTalismanKind::Skipper);
    }
    if snapshot.final_yen >= 20 && j.shop_talisman_buys + j.shop_relic_buys + j.shop_pack_buys >= 3
    {
        qualified.push(MemorialTalismanKind::Hoarder);
    }
    if snapshot.consumables_unused > 0 && j.talisman_uses == 0 && j.zodiac_uses == 0 {
        qualified.push(MemorialTalismanKind::FullDish);
    }
    if snapshot.tiles_discarded > snapshot.tiles_played.saturating_mul(3) / 2
        && snapshot.tiles_discarded >= 8
    {
        qualified.push(MemorialTalismanKind::Discarded);
    }
    if j.dominant_buff_talisman().is_some() {
        qualified.push(MemorialTalismanKind::BuffSaint);
    }
    if j.dominant_transform_talisman().is_some() {
        qualified.push(MemorialTalismanKind::Transformer);
    }
    if j.tags_taken >= 1 {
        qualified.push(MemorialTalismanKind::TagBearer);
    }
    if snapshot.dominant_yaku.is_some() {
        qualified.push(MemorialTalismanKind::MeldMason);
    }
    if snapshot.final_wing >= 4 {
        qualified.push(MemorialTalismanKind::DeepWalker);
    }
    qualified.push(match snapshot.loss_reason {
        GameOverReason::OutOfPlays => MemorialTalismanKind::Exhausted,
        GameOverReason::NoActionsRemaining => MemorialTalismanKind::FrozenHand,
    });
    qualified
}

/// Pick which remnant the player becomes / receives next run.
pub fn select_memorial(snapshot: &MemorialJournalSnapshot) -> MemorialTalismanKind {
    select_memorial_with_rng(snapshot, &mut rand::rng())
}

/// Pick one qualified remnant with the provided RNG.
pub fn select_memorial_with_rng(
    snapshot: &MemorialJournalSnapshot,
    rng: &mut impl rand::Rng,
) -> MemorialTalismanKind {
    let qualified = qualified_memorials(snapshot);
    let idx = rng.random_range(0..qualified.len());
    qualified[idx]
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
    use crate::core::rules::ChamberKind;

    fn snapshot_with(journal: RunDefeatJournal, reason: GameOverReason) -> MemorialJournalSnapshot {
        MemorialJournalSnapshot {
            journal,
            loss_reason: reason,
            final_wing: 1,
            final_chamber: ChamberKind::Small,
            final_yen: 0,
            tiles_played: 0,
            tiles_discarded: 0,
            consumables_unused: 0,
            dominant_yaku: None,
            run_number: Some(3),
        }
    }

    #[test]
    fn dead_on_arrival_when_first_two_chambers() {
        let mut s = snapshot_with(RunDefeatJournal::default(), GameOverReason::OutOfPlays);
        s.run_number = Some(1);
        assert_eq!(
            qualified_memorials(&s),
            vec![
                MemorialTalismanKind::DeadOnArrival,
                MemorialTalismanKind::Exhausted
            ]
        );
        s.run_number = Some(2);
        assert_eq!(
            qualified_memorials(&s),
            vec![
                MemorialTalismanKind::DeadOnArrival,
                MemorialTalismanKind::Exhausted
            ]
        );
        s.run_number = Some(3);
        assert_eq!(
            qualified_memorials(&s),
            vec![MemorialTalismanKind::Exhausted]
        );
    }

    #[test]
    fn boss_death_qualifies_boss_mark() {
        let mut s = snapshot_with(RunDefeatJournal::default(), GameOverReason::OutOfPlays);
        s.final_chamber = ChamberKind::Ordeal;
        assert_eq!(
            qualified_memorials(&s),
            vec![
                MemorialTalismanKind::BossMark,
                MemorialTalismanKind::Exhausted
            ]
        );
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
        j.chambers_skipped = 3;
        let s = snapshot_with(j, GameOverReason::NoActionsRemaining);
        assert_eq!(
            qualified_memorials(&s),
            vec![
                MemorialTalismanKind::Skipper,
                MemorialTalismanKind::FrozenHand
            ]
        );
    }

    #[test]
    fn multiple_actions_all_qualify() {
        let mut j = RunDefeatJournal {
            chambers_skipped: 2,
            tags_taken: 1,
            ..Default::default()
        };
        j.record_talisman_use(TalismanKind::Pearl);
        let mut s = snapshot_with(j, GameOverReason::OutOfPlays);
        s.final_chamber = ChamberKind::Ordeal;
        s.run_number = Some(2);
        s.final_wing = 4;
        s.tiles_played = 8;
        s.tiles_discarded = 13;
        s.dominant_yaku = Some(YakuKind::Tanyao);
        assert_eq!(
            qualified_memorials(&s),
            vec![
                MemorialTalismanKind::BossMark,
                MemorialTalismanKind::DeadOnArrival,
                MemorialTalismanKind::Skipper,
                MemorialTalismanKind::Discarded,
                MemorialTalismanKind::BuffSaint,
                MemorialTalismanKind::TagBearer,
                MemorialTalismanKind::MeldMason,
                MemorialTalismanKind::DeepWalker,
                MemorialTalismanKind::Exhausted,
            ]
        );
    }

    #[test]
    fn rng_selects_from_qualified_set() {
        use rand::SeedableRng;

        let mut j = RunDefeatJournal {
            chambers_skipped: 2,
            ..Default::default()
        };
        j.record_talisman_use(TalismanKind::Souzu);
        let s = snapshot_with(j, GameOverReason::NoActionsRemaining);
        let qualified = qualified_memorials(&s);
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        for _ in 0..32 {
            assert!(qualified.contains(&select_memorial_with_rng(&s, &mut rng)));
        }
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
