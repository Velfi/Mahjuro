//! Incremental gameplay-engine boundary for scene-facing gameplay mutations.
//!
//! This module is the first slice of the larger gameplay rewrite: scenes
//! dispatch typed commands and receive semantic outcomes, while the current
//! `RunState` still acts as the legacy mutation backend underneath.
//!
//! Gameplay mutations to hand, selection, structure bank, and the resource
//! fields owned by [`GameplayCoreState`](crate::game::engine_state::GameplayCoreState)
//! should go through [`GameEngine`] commands or
//! [`GameplayCoreState::with_run_mut`](crate::game::engine_state::GameplayCoreState::with_run_mut)
//! so parallel vectors stay aligned.

use crate::core::boss::BossKind;
use crate::core::consumable::Consumable;
use crate::core::hand::DetectedMeld;
use crate::core::progression::PlayerProgress;
use crate::core::relic::{RelicId, RelicState, apply_merchants_eye_discount};
use crate::core::rules::BlindKind;
use crate::core::scoring::ScoreBreakdown;
use crate::core::structure::is_winning_structure_shape;
use crate::core::tag::TagKind;
use crate::core::talisman::TalismanKind;
use crate::core::tile::{Suit, Tile};
use crate::core::tile_pack::{PACK_ID_STRIDE, PACK_TILE_ID_BASE, TilePackKind};
use crate::core::yaku::YakuKind;
use crate::game::engine_state::GameplayCoreState;
use crate::game::event_bus::{EventBus, GameEvent, GameOverReason};
use crate::game::onboarding::OnboardingPhase;
use crate::game::run::{ConsumableUseResult, RunState};
use crate::game::tutorial::TutorialMilestone;
use crate::persistence::{AppSettings, TileMaterial};
use crate::ui::input::MarqueeSelect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameCommand {
    CommitSelection,
    TriggerStructure,
    DiscardSelectionNoRefill,
    RefillHand,
    UseConsumable { index: usize },
    SortHandBySuit,
    SortHandByRank,
    ApplyBlind { blind: BlindKind },
    SkipUpcomingBlindWithTag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandRejection {
    EmptySelection,
    InvalidSelection,
    NoPlaysRemaining,
    NoDiscardsRemaining,
    TriggerUnavailable,
    ConsumableUnavailable,
    TutorialLocked,
    NoEffect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiHint {
    Hand,
    Selection,
    Score,
    Structure,
    Consumables,
    Resources,
    Blind,
    Round,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineEvent {
    InvalidAction,
    ScoreChanged { delta: u64 },
    GoldChanged { delta: i32 },
    TilesDrawn { count: usize },
    TilesDiscarded { count: usize },
    StructureCommitted,
    StructureTriggered { earned: u64 },
    ConsumableUsed { result: ConsumableUseResult },
    BlindApplied { blind: BlindKind },
    BlindSkipped { next_blind: BlindKind },
    TagApplied { tag: TagKind },
    RoundComplete { reached_target: bool },
    GameOver { reason: GameOverReason },
    TutorialMilestone(TutorialMilestone),
    RelicActivated(RelicId),
    TalismanUsed(TalismanKind),
    ZodiacLevelUp,
    YakuScored(YakuKind),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EngineSnapshot {
    pub hand_len: usize,
    pub selected_count: usize,
    pub structure_tiles: usize,
    pub structure_sets: usize,
    pub consumable_count: usize,
    pub round_score: u64,
    pub gold: i32,
    pub plays_remaining: u32,
    pub discards_remaining: u32,
}

impl EngineSnapshot {
    fn capture(run: &RunState) -> Self {
        Self {
            hand_len: run.hand().len(),
            selected_count: run.selected_count(),
            structure_tiles: run.structure_tiles().len(),
            structure_sets: run.structure_sets().len(),
            consumable_count: run.consumables.items.len(),
            round_score: run.round_score,
            gold: run.gold,
            plays_remaining: run.plays_remaining,
            discards_remaining: run.discards_remaining,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandData {
    None,
    CommitSelection { step: u64 },
    TriggerStructure { earned: u64 },
    DiscardSelection { count: usize },
    RefillHand,
    UseConsumable { result: ConsumableUseResult },
    ApplyBlind { blind: BlindKind },
    SkipBlind { tag: Option<TagKind> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutcome {
    pub command: GameCommand,
    pub data: CommandData,
    pub rejection: Option<CommandRejection>,
    pub before: EngineSnapshot,
    pub after: EngineSnapshot,
    pub events: Vec<EngineEvent>,
    pub ui_hints: Vec<UiHint>,
}

impl CommandOutcome {
    fn rejected(command: GameCommand, before: EngineSnapshot, rejection: CommandRejection) -> Self {
        Self {
            command,
            data: CommandData::None,
            rejection: Some(rejection),
            before,
            after: before,
            events: Vec::new(),
            ui_hints: Vec::new(),
        }
    }
}

pub struct GameEngine<'a> {
    run: &'a mut RunState,
    bus: &'a mut EventBus,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConsumedShopTags {
    pub free_reroll: bool,
    pub patron_gift: bool,
    pub rich_stock: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShopCommand {
    BuyRelic {
        relic: RelicId,
        price: u32,
    },
    SellRelic {
        index: usize,
    },
    MoveRelicLeft {
        index: usize,
    },
    MoveRelicRight {
        index: usize,
    },
    BuyZodiac {
        zodiac: crate::core::zodiac::ZodiacKind,
        price: u32,
    },
    BuyTalisman {
        kind: TalismanKind,
        price: u32,
    },
    SellConsumable {
        index: usize,
    },
    UseConsumable {
        index: usize,
    },
    BuyPack {
        kind: TilePackKind,
        price: u32,
    },
    RerollShop {
        cost: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShopCommandRejection {
    InsufficientGold,
    InvalidTarget,
    InventoryFull,
    NoEffect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShopCommandData {
    None,
    ZodiacApplied {
        zodiac_kind: crate::core::zodiac::ZodiacKind,
        yaku_name: &'static str,
        new_level: u32,
    },
    PackBought {
        tiles: Vec<Tile>,
        pack_name: &'static str,
        pack_kind: TilePackKind,
    },
    Rerolled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShopCommandOutcome {
    pub command: ShopCommand,
    pub data: ShopCommandData,
    pub rejection: Option<ShopCommandRejection>,
    pub before: EngineSnapshot,
    pub after: EngineSnapshot,
    pub events: Vec<EngineEvent>,
    pub ui_hints: Vec<UiHint>,
}

impl ShopCommandOutcome {
    fn rejected(
        command: ShopCommand,
        before: EngineSnapshot,
        rejection: ShopCommandRejection,
    ) -> Self {
        Self {
            command,
            data: ShopCommandData::None,
            rejection: Some(rejection),
            before,
            after: before,
            events: Vec::new(),
            ui_hints: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GameplayReadModel {
    pub blind: BlindKind,
    pub blind_label: String,
    pub boss_ofuda_title: String,
    pub boss_ofuda_rule_text: String,
    pub run_number: u32,
    pub round_wind_rank: u8,
    pub round_wind_label: &'static str,
    pub tiles_left: usize,
    pub dora_faces: Vec<(Suit, u8)>,
    pub dora_indicator_tiles: Vec<Tile>,
    pub has_structure: bool,
    pub structure_complete: bool,
    pub structure_tiles: Vec<Tile>,
    pub structure_sets: Vec<DetectedMeld>,
    pub trigger_enabled: bool,
    pub trigger_preview_total: u64,
    pub selected_count: usize,
    pub hand_len: usize,
    pub round_score: u64,
    pub target_score: u32,
    pub plays_remaining: u32,
    pub plays_max: u32,
    pub discards_remaining: u32,
    pub discards_max: u32,
    pub gold: i32,
    pub available_yaku: Vec<YakuKind>,
    pub has_dora_crown: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GameplayInteractionReadModel {
    pub hand: Vec<Tile>,
    pub hand_ids: Vec<u32>,
    pub hand_len: usize,
    pub selected: Vec<bool>,
    pub selected_indices: Vec<usize>,
    pub consumables: Vec<Consumable>,
    pub consumable_capacity: usize,
    pub consumable_count: usize,
    pub relic_count: usize,
    pub tutorial_active: bool,
    pub hints_enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TutorialLessonReadModel {
    pub current_lesson: u32,
    pub flavor_text: &'static str,
    pub intro_text: &'static str,
    pub step_prompts: &'static [&'static str],
    pub meld_guide_opened: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShopOwnedConsumable {
    pub inventory_index: usize,
    pub consumable: Consumable,
}

#[derive(Clone, Debug)]
pub struct ShopReadModel {
    pub gold: i32,
    pub display_gold: u32,
    pub relics_full: bool,
    pub consumables_full: bool,
    pub relic_state: RelicState,
    pub owned_relics: Vec<RelicId>,
    pub available_relics: Vec<RelicId>,
    pub owned_zodiacs: Vec<ShopOwnedConsumable>,
    pub owned_talismans: Vec<ShopOwnedConsumable>,
    pub relic_counters: std::collections::BTreeMap<RelicId, i32>,
    pub total_score_earned: u64,
    /// Score target for the next fight (`base_target * run_number`), for relic tooltips.
    pub next_blind_target: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PickBlindReadModel {
    pub upcoming_blind: BlindKind,
    pub ante: u32,
    pub run_number: u32,
    pub base_target: u32,
    pub skip_tag: Option<TagKind>,
    pub boss_kind: Option<BossKind>,
    pub boss_name: Option<String>,
    pub boss_description: Option<String>,
    pub boss_tier_label: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TutorialOverlayReadModel {
    pub selected_count: usize,
    pub discards_remaining: u32,
    pub round_score: u64,
    pub has_structure: bool,
    pub blind: BlindKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YakuProgressReadModel {
    pub levels: rustc_hash::FxHashMap<YakuKind, u32>,
    pub played_this_run: rustc_hash::FxHashMap<YakuKind, u32>,
}

impl YakuProgressReadModel {
    pub fn level_of(&self, yaku: YakuKind) -> u32 {
        self.levels.get(&yaku).copied().unwrap_or(1)
    }

    pub fn played_this_run(&self, yaku: YakuKind) -> u32 {
        self.played_this_run.get(&yaku).copied().unwrap_or(0)
    }
}

impl<'a> GameEngine<'a> {
    pub fn new(run: &'a mut RunState, bus: &'a mut EventBus) -> Self {
        Self { run, bus }
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot::capture(self.run)
    }

    pub fn apply_progression_and_settings(
        run: &mut RunState,
        progress: &PlayerProgress,
        settings: &AppSettings,
    ) {
        run.apply_progression(progress);
        run.set_auto_cash_in_on_full_structure(settings.auto_cash_in_on_full_structure);
        run.set_hints_enabled(settings.hints_enabled);
    }

    pub fn reset_to_demo(run: &mut RunState, progress: &PlayerProgress, settings: &AppSettings) {
        *run = RunState::new_demo();
        Self::apply_progression_and_settings(run, progress, settings);
    }

    pub fn start_run_with_material(
        run: &mut RunState,
        material: TileMaterial,
        progress: &PlayerProgress,
        settings: &AppSettings,
    ) {
        *run = RunState::new_with_material(material);
        Self::apply_progression_and_settings(run, progress, settings);
    }

    /// Stake-aware variant used by the tile-select modal's stake picker.
    /// Spring behaves identically to `start_run_with_material`.
    pub fn start_run_with_material_and_stake(
        run: &mut RunState,
        material: TileMaterial,
        stake: crate::core::stake::Stake,
        progress: &PlayerProgress,
        settings: &AppSettings,
    ) {
        *run = RunState::new_with_material_and_stake(material, stake);
        Self::apply_progression_and_settings(run, progress, settings);
    }

    pub fn start_onboarding_run(
        run: &mut RunState,
        progress: &PlayerProgress,
        settings: &AppSettings,
    ) {
        *run = RunState::new_onboarding();
        Self::apply_progression_and_settings(run, progress, settings);
    }

    pub fn transition_to_onboarding_finale(run: &mut RunState) {
        if let Some(ref mut onboarding) = run.onboarding {
            onboarding.phase = crate::game::onboarding::OnboardingPhase::Finale;
        }
        run.begin_onboarding_finale();
    }

    pub fn set_onboarding_shop_phase(run: &mut RunState) {
        if let Some(ref mut onboarding) = run.onboarding {
            onboarding.phase = OnboardingPhase::Shop;
        }
    }

    pub fn consume_shop_tags(run: &mut RunState) -> ConsumedShopTags {
        let tags = ConsumedShopTags {
            free_reroll: run.tag_free_reroll,
            patron_gift: run.tag_patron_gift,
            rich_stock: run.tag_rich_stock,
        };
        run.tag_free_reroll = false;
        run.tag_patron_gift = false;
        run.tag_rich_stock = false;
        tags
    }

    pub fn prepare_pending_blind(run: &mut RunState) {
        GameplayCoreState::with_run_mut(run, |core| {
            core.clear_hand_structure_bank();
        });
    }

    pub fn take_pending_zodiac_celebration(
        run: &mut RunState,
    ) -> Option<(crate::core::zodiac::ZodiacKind, YakuKind, u32)> {
        run.pending_zodiac_celebration.take()
    }

    pub fn set_finished_zodiac_celebration(
        run: &mut RunState,
        yaku_name: &'static str,
        new_level: u32,
    ) {
        run.finished_zodiac_celebration = Some((yaku_name, new_level));
    }

    pub fn take_finished_zodiac_celebration(run: &mut RunState) -> Option<(&'static str, u32)> {
        run.finished_zodiac_celebration.take()
    }

    pub fn drain_relic_activations(run: &mut RunState) -> Vec<RelicId> {
        run.relic_activations.drain(..).collect()
    }

    pub fn debug_add_pack(run: &mut RunState, kind: TilePackKind) -> Vec<Tile> {
        let pack_idx = run.tile_packs.len();
        let start_id = PACK_TILE_ID_BASE + (pack_idx as u32) * PACK_ID_STRIDE;
        let mut tiles = kind.generate_tiles(start_id);
        if let Some(enh) = kind.pre_enhancement() {
            for t in &mut tiles {
                run.tile_enhancements.insert(t.id, enh);
                t.enhancement = Some(enh);
            }
        }
        run.tile_packs.push(kind);
        tiles
    }

    pub fn begin_marquee_selection(
        run: &mut RunState,
        index: usize,
    ) -> Option<(MarqueeSelect, (u32, u32))> {
        GameplayCoreState::with_run_mut(run, |core| core.begin_marquee_selection(index))
    }

    pub fn apply_marquee_selection(
        run: &mut RunState,
        marquee: &mut MarqueeSelect,
        index: usize,
    ) -> Option<(u32, u32)> {
        GameplayCoreState::with_run_mut(run, |core| core.apply_marquee_selection(marquee, index))
    }

    pub fn swap_active_relics(run: &mut RunState, from_idx: usize, to_idx: usize) -> bool {
        if from_idx == to_idx
            || to_idx >= run.relics.active.len()
            || from_idx >= run.relics.active.len()
        {
            return false;
        }
        run.relics.swap_relics(from_idx, to_idx);
        true
    }

    pub fn read_shop(run: &RunState) -> ShopReadModel {
        let mut owned_zodiacs = Vec::new();
        let mut owned_talismans = Vec::new();
        for (inventory_index, consumable) in run.consumables.items.iter().copied().enumerate() {
            let owned = ShopOwnedConsumable {
                inventory_index,
                consumable,
            };
            match consumable {
                Consumable::Zodiac(_) => owned_zodiacs.push(owned),
                Consumable::Talisman(_) => owned_talismans.push(owned),
            }
        }
        ShopReadModel {
            gold: run.gold,
            display_gold: run.gold.max(0) as u32,
            relics_full: run.relics.is_full(),
            consumables_full: run.consumables.is_full(),
            relic_state: run.relics.clone(),
            owned_relics: run.relics.active.clone(),
            available_relics: run.available_relics.clone(),
            owned_zodiacs,
            owned_talismans,
            relic_counters: run.relic_counters.clone(),
            total_score_earned: run.total_score_earned,
            next_blind_target: run.base_target.saturating_mul(run.run_number),
        }
    }

    pub fn shop_extra_relic_stock(run: &RunState) -> usize {
        let extra_relics: usize = if run.tag_rich_stock { 2 } else { 0 };
        if run.tag_patron_gift {
            extra_relics.max(1)
        } else {
            extra_relics
        }
    }

    pub fn shop_has_patron_gift(run: &RunState) -> bool {
        run.tag_patron_gift
    }

    pub fn read_pick_blind(run: &RunState) -> PickBlindReadModel {
        let boss_kind = run.boss.upcoming;
        let (boss_name, boss_description, boss_tier_label) = if let Some(kind) = boss_kind {
            let def = kind.def();
            let description = run
                .boss
                .effect
                .as_ref()
                .and_then(|effect| effect.description_override.clone())
                .unwrap_or_else(|| def.description.to_string());
            (
                Some(def.name.to_string()),
                Some(description),
                Some(def.tier.label()),
            )
        } else {
            (None, None, None)
        };
        PickBlindReadModel {
            upcoming_blind: run.upcoming_blind,
            ante: run.ante,
            run_number: run.run_number,
            base_target: run.base_target,
            skip_tag: run.tag_for_blind(run.upcoming_blind),
            boss_kind,
            boss_name,
            boss_description,
            boss_tier_label,
        }
    }

    pub fn read_tutorial_overlay(run: &RunState) -> TutorialOverlayReadModel {
        TutorialOverlayReadModel {
            selected_count: run.selected_count(),
            discards_remaining: run.discards_remaining,
            round_score: run.round_score,
            has_structure: !run.structure_sets().is_empty(),
            blind: run.blind,
        }
    }

    pub fn read_yaku_progress(run: &RunState) -> YakuProgressReadModel {
        YakuProgressReadModel {
            levels: run.yaku_levels.levels.clone(),
            played_this_run: run.yaku_times_played.clone(),
        }
    }

    pub fn run_in_progress(run: &RunState) -> bool {
        run.is_in_progress()
    }

    pub fn read_interaction(run: &RunState) -> GameplayInteractionReadModel {
        let core = GameplayCoreState::from_run(run);
        let selected = core.selected.clone();
        let selected_indices = core.selected_indices();
        GameplayInteractionReadModel {
            hand: core.hand.clone(),
            hand_ids: core.hand.iter().map(|tile| tile.id).collect(),
            hand_len: core.hand_len(),
            selected,
            selected_indices,
            consumables: core.consumables.items.clone(),
            consumable_capacity: core.consumables.capacity,
            consumable_count: core.consumables.items.len(),
            relic_count: run.relics.len(),
            tutorial_active: run
                .tutorial
                .as_ref()
                .is_some_and(|tutorial| tutorial.is_active()),
            hints_enabled: run.hints_enabled,
        }
    }

    pub fn tutorial_affinity_glow(run: &RunState) -> bool {
        run.tutorial_affinity_glow()
    }

    pub fn tutorial_annotated_cascade(run: &RunState) -> bool {
        run.tutorial_annotated_cascade()
    }

    pub fn structure_banked_meld_chips(run: &RunState) -> i32 {
        run.structure_banked_meld_chips()
    }

    pub fn preview_manual_trigger_breakdown(run: &RunState) -> Option<ScoreBreakdown> {
        run.preview_manual_trigger_breakdown()
    }

    pub fn selection_is_valid(run: &RunState) -> bool {
        run.is_selection_valid()
    }

    pub fn validate_with_wildcards(
        run: &RunState,
        tiles: &[Tile],
    ) -> Option<(Vec<DetectedMeld>, Vec<Tile>)> {
        run.try_validate_with_wildcards(tiles)
    }

    pub fn display_tile(tile: Tile, run: &RunState) -> Tile {
        let mut tile = tile;
        tile.debuffed_visual = run.tile_debuffs.iter().any(|debuff| debuff.matches(&tile));
        tile
    }

    pub fn display_tiles(tiles: impl IntoIterator<Item = Tile>, run: &RunState) -> Vec<Tile> {
        tiles
            .into_iter()
            .map(|tile| Self::display_tile(tile, run))
            .collect()
    }

    pub fn read(run: &RunState) -> GameplayReadModel {
        let core = GameplayCoreState::from_run(run);
        let blind_label = if run.blind == BlindKind::Boss {
            run.boss
                .upcoming
                .map(|k| k.def().name.to_string())
                .unwrap_or_else(|| run.blind.name().to_string())
        } else {
            run.blind.name().to_string()
        };
        let (boss_ofuda_title, boss_ofuda_rule_text) = if run.blind == BlindKind::Boss {
            if let Some(kind) = run.boss.upcoming {
                let def = kind.def();
                let desc = run
                    .boss
                    .effect
                    .as_ref()
                    .and_then(|effect| effect.description_override.as_deref())
                    .unwrap_or(def.description);
                (def.name.to_string(), desc.to_string())
            } else {
                (String::new(), String::new())
            }
        } else {
            (String::new(), String::new())
        };
        let round_wind = BlindKind::round_wind_for_ante(run.ante);
        GameplayReadModel {
            blind: run.blind,
            blind_label,
            boss_ofuda_title,
            boss_ofuda_rule_text,
            run_number: run.run_number,
            round_wind_rank: round_wind,
            round_wind_label: BlindKind::wind_name(round_wind),
            tiles_left: run.wall.remaining(),
            dora_faces: run.wall.dora_faces(),
            dora_indicator_tiles: run.wall.dora_indicator_tiles().to_vec(),
            has_structure: !core.structure_sets.is_empty(),
            structure_complete: is_winning_structure_shape(
                &core.structure_tiles,
                &core.structure_sets,
            ),
            structure_tiles: core.structure_tiles.clone(),
            structure_sets: core.structure_sets.clone(),
            trigger_enabled: run.can_trigger_structure_now(),
            trigger_preview_total: run.preview_manual_trigger_total(),
            selected_count: core.selected_count(),
            hand_len: core.hand_len(),
            round_score: core.round_score,
            target_score: core.target_score,
            plays_remaining: core.plays_remaining,
            plays_max: core.plays_max,
            discards_remaining: core.discards_remaining,
            discards_max: core.discards_max,
            gold: core.gold,
            available_yaku: core.available_yaku.clone(),
            has_dora_crown: run.relics.has(RelicId::DoraCrown),
        }
    }

    pub fn tutorial_affinity_indices(run: &RunState) -> Vec<usize> {
        let Some(tutorial) = run
            .tutorial
            .as_ref()
            .filter(|tutorial| tutorial.is_active())
        else {
            return Vec::new();
        };
        let lesson = tutorial.current_lesson_def();
        crate::game::tutorial::affinity_tile_indices(
            run.hand(),
            run.selected_slice(),
            lesson.allowed_melds,
        )
    }

    pub fn tutorial_lesson(run: &RunState) -> Option<TutorialLessonReadModel> {
        let tutorial = run
            .tutorial
            .as_ref()
            .filter(|tutorial| tutorial.is_active())?;
        let lesson = tutorial.current_lesson_def();
        Some(TutorialLessonReadModel {
            current_lesson: tutorial.current_lesson,
            flavor_text: lesson.flavor_text,
            intro_text: lesson.intro_text,
            step_prompts: lesson.step_prompts,
            meld_guide_opened: tutorial.meld_guide_opened,
        })
    }

    pub fn mark_tutorial_cascade_annotated(run: &mut RunState) {
        if let Some(ref mut tutorial) = run.tutorial {
            tutorial.cascade_annotated = true;
        }
    }

    pub fn mark_tutorial_meld_guide_opened(run: &mut RunState) {
        if let Some(ref mut tutorial) = run.tutorial {
            tutorial.meld_guide_opened = true;
        }
    }

    pub fn celebrate_tutorial_milestone(
        run: &mut RunState,
        bus: &mut EventBus,
        milestone: TutorialMilestone,
    ) -> bool {
        let Some(tutorial) = run.tutorial.as_mut() else {
            return false;
        };
        if !tutorial.celebrate(milestone) {
            return false;
        }
        bus.push(GameEvent::TutorialMilestone(milestone));
        true
    }

    pub fn last_breakdown(run: &RunState) -> Option<ScoreBreakdown> {
        run.last_breakdown.clone()
    }

    pub fn active_relics(run: &RunState) -> Vec<RelicId> {
        run.relics.active.clone()
    }

    pub fn auto_cash_in_on_full_structure(run: &RunState) -> bool {
        run.auto_cash_in_on_full_structure
    }

    pub fn dora_matching_hand_indices(run: &RunState, dora_faces: &[(Suit, u8)]) -> Vec<usize> {
        run.hand()
            .iter()
            .enumerate()
            .filter_map(|(i, &tile)| {
                let display = Self::display_tile(tile, run);
                dora_faces
                    .contains(&(display.suit, display.rank))
                    .then_some(i)
            })
            .collect()
    }

    pub fn resumes_to_tutorial_shop(run: &RunState) -> bool {
        run.onboarding
            .as_ref()
            .is_some_and(|o| matches!(o.phase, OnboardingPhase::Shop))
    }

    pub fn current_run_number(run: &RunState) -> u32 {
        run.run_number
    }

    pub fn current_upcoming_blind(run: &RunState) -> BlindKind {
        run.upcoming_blind
    }

    pub fn dispatch(&mut self, command: GameCommand) -> CommandOutcome {
        let before = self.snapshot();
        let queue_start = self.bus.queue.len();
        let data = match command {
            GameCommand::CommitSelection => {
                if self.run.selected_count() == 0 {
                    return CommandOutcome::rejected(
                        command,
                        before,
                        CommandRejection::EmptySelection,
                    );
                }
                if self.run.plays_remaining == 0 {
                    return CommandOutcome::rejected(
                        command,
                        before,
                        CommandRejection::NoPlaysRemaining,
                    );
                }
                let step = self.run.commit_selection_to_structure(self.bus);
                if step == 0 {
                    return self.finish_outcome(
                        command,
                        before,
                        queue_start,
                        CommandData::CommitSelection { step },
                        Some(CommandRejection::InvalidSelection),
                    );
                }
                CommandData::CommitSelection { step }
            }
            GameCommand::TriggerStructure => {
                if !self.run.can_trigger_structure_now() {
                    return CommandOutcome::rejected(
                        command,
                        before,
                        CommandRejection::TriggerUnavailable,
                    );
                }
                let earned = self.run.trigger_structure_manual(self.bus);
                if earned == 0 {
                    return self.finish_outcome(
                        command,
                        before,
                        queue_start,
                        CommandData::TriggerStructure { earned },
                        Some(CommandRejection::TriggerUnavailable),
                    );
                }
                CommandData::TriggerStructure { earned }
            }
            GameCommand::DiscardSelectionNoRefill => {
                if !self.run.tutorial_discard_allowed() {
                    return CommandOutcome::rejected(
                        command,
                        before,
                        CommandRejection::TutorialLocked,
                    );
                }
                if self.run.discards_remaining == 0 {
                    return CommandOutcome::rejected(
                        command,
                        before,
                        CommandRejection::NoDiscardsRemaining,
                    );
                }
                if self.run.selected_count() == 0 {
                    return CommandOutcome::rejected(
                        command,
                        before,
                        CommandRejection::EmptySelection,
                    );
                }
                let count = self.run.discard_selected_no_refill(self.bus);
                if count == 0 {
                    return self.finish_outcome(
                        command,
                        before,
                        queue_start,
                        CommandData::DiscardSelection { count },
                        Some(CommandRejection::NoEffect),
                    );
                }
                CommandData::DiscardSelection { count }
            }
            GameCommand::RefillHand => {
                let hand_before = self.run.hand().len();
                self.run.refill_hand(self.bus);
                if self.run.hand().len() == hand_before {
                    return self.finish_outcome(
                        command,
                        before,
                        queue_start,
                        CommandData::RefillHand,
                        Some(CommandRejection::NoEffect),
                    );
                }
                CommandData::RefillHand
            }
            GameCommand::UseConsumable { index } => {
                if self.run.consumables.items.get(index).is_none() {
                    return CommandOutcome::rejected(
                        command,
                        before,
                        CommandRejection::ConsumableUnavailable,
                    );
                }
                let Some(result) = self.run.use_consumable(index, self.bus) else {
                    return self.finish_outcome(
                        command,
                        before,
                        queue_start,
                        CommandData::None,
                        Some(CommandRejection::ConsumableUnavailable),
                    );
                };
                CommandData::UseConsumable { result }
            }
            GameCommand::SortHandBySuit => {
                GameplayCoreState::with_run_mut(self.run, |core| {
                    core.sort_hand_by_suit();
                });
                CommandData::None
            }
            GameCommand::SortHandByRank => {
                GameplayCoreState::with_run_mut(self.run, |core| {
                    core.sort_hand_by_rank();
                });
                CommandData::None
            }
            GameCommand::ApplyBlind { blind } => {
                self.run.apply_blind(blind, Some(&mut self.bus));
                CommandData::ApplyBlind { blind }
            }
            GameCommand::SkipUpcomingBlindWithTag => {
                let tag = self.run.tag_for_blind(self.run.upcoming_blind);
                if let Some(tag) = tag {
                    self.run.apply_tag(tag, Some(&mut self.bus));
                }
                self.run.skip_to_next_blind();
                CommandData::SkipBlind { tag }
            }
        };

        self.finish_outcome(command, before, queue_start, data, None)
    }

    pub fn dispatch_shop(&mut self, command: ShopCommand) -> ShopCommandOutcome {
        let before = self.snapshot();
        let queue_start = self.bus.queue.len();
        let data = match command {
            ShopCommand::BuyRelic { relic, price } => {
                if self.run.gold < price as i32 {
                    return ShopCommandOutcome::rejected(
                        command,
                        before,
                        ShopCommandRejection::InsufficientGold,
                    );
                }
                if self.run.relics.is_full() {
                    return ShopCommandOutcome::rejected(
                        command,
                        before,
                        ShopCommandRejection::InventoryFull,
                    );
                }
                self.run
                    .apply_gold_delta(-(price as i32), Some(&mut self.bus));
                self.run.relics.active.push(relic);
                self.bus
                    .push(GameEvent::UiSound(crate::audio::SfxId::Purchase));
                self.bus.push(GameEvent::PlayRelicStinger(relic));
                match relic {
                    RelicId::MeltingIce => {
                        self.run.relic_counters.insert(RelicId::MeltingIce, 80);
                    }
                    RelicId::Taotie => {
                        self.run.relic_counters.insert(RelicId::Taotie, 0);
                    }
                    RelicId::SilkThread => {
                        self.run.relic_counters.insert(RelicId::SilkThread, 40);
                    }
                    RelicId::SilkMoth => {
                        self.run.relic_counters.insert(RelicId::SilkMoth, 0);
                    }
                    RelicId::RustlingGooseEgg => {
                        self.run.relic_counters.insert(RelicId::RustlingGooseEgg, 3);
                    }
                    RelicId::TeaCeremony => {
                        self.run.relic_counters.insert(RelicId::TeaCeremony, 0);
                    }
                    RelicId::Chrysalis => {
                        self.run.relic_counters.insert(RelicId::MonarchButterfly, 0);
                    }
                    RelicId::MonarchButterfly => {
                        self.run.relic_counters.insert(RelicId::MonarchButterfly, 0);
                    }
                    RelicId::IGotAGuy => {
                        self.run.relic_counters.insert(RelicId::IGotAGuy, 3);
                    }
                    RelicId::Rakuware => {}
                    _ => {}
                }
                self.run.recompute_capacities();
                if let Some(ref mut tut) = self.run.tutorial
                    && tut.celebrate(crate::game::tutorial::TutorialMilestone::FirstShopBuy)
                {
                    self.bus.push(GameEvent::TutorialMilestone(
                        crate::game::tutorial::TutorialMilestone::FirstShopBuy,
                    ));
                }
                ShopCommandData::None
            }
            ShopCommand::SellRelic { index } => {
                if index >= self.run.relics.active.len() {
                    return ShopCommandOutcome::rejected(
                        command,
                        before,
                        ShopCommandRejection::InvalidTarget,
                    );
                }
                let rid = self.run.relics.active[index];
                let mut refund = crate::core::relic::relic_sell_price(rid);
                if rid == RelicId::NestEgg {
                    let rounds = self
                        .run
                        .relic_counters
                        .get(&RelicId::NestEgg)
                        .copied()
                        .unwrap_or(0);
                    refund = refund.saturating_add(2 * rounds as u32);
                }
                if rid == RelicId::HungryGhost && index + 1 < self.run.relics.active.len() {
                    let victim_id = self.run.relics.active[index + 1];
                    let victim_value = crate::core::relic::relic_sell_price(victim_id) as i32;
                    self.run.relics.active.remove(index + 1);
                    self.run.relics.active.remove(index);
                    if !self.run.relics.has(RelicId::IGotAGuy) {
                        self.run.relic_counters.remove(&RelicId::IGotAGuy);
                    }
                    *self
                        .run
                        .relic_counters
                        .entry(RelicId::HungryGhost)
                        .or_insert(0) += victim_value * 2 * 10;
                    self.run.relic_activations.push(RelicId::HungryGhost);
                    // Kintsugi counts the victim (involuntary destruction).
                    // The blade itself was sold, not destroyed, so don't
                    // credit it.
                    self.run.note_relic_destroyed();
                    return self.finish_shop_outcome(
                        command,
                        before,
                        queue_start,
                        ShopCommandData::None,
                        None,
                    );
                }
                self.run.relics.active.remove(index);
                if !self.run.relics.has(RelicId::IGotAGuy) {
                    self.run.relic_counters.remove(&RelicId::IGotAGuy);
                }
                if rid == RelicId::Snowball {
                    self.run.relic_counters.remove(&RelicId::Snowball);
                }
                self.run
                    .apply_gold_reward(refund as i32, Some(&mut self.bus));
                self.bus.push(GameEvent::UiSound(crate::audio::SfxId::Sell));
                *self.run.relic_counters.entry(RelicId::Bonfire).or_insert(0) += 1;
                if self.run.relics.has(RelicId::Bonfire) {
                    self.run.relic_activations.push(RelicId::Bonfire);
                }
                ShopCommandData::None
            }
            ShopCommand::MoveRelicLeft { index } => {
                if index == 0 || index >= self.run.relics.active.len() {
                    return ShopCommandOutcome::rejected(
                        command,
                        before,
                        ShopCommandRejection::InvalidTarget,
                    );
                }
                self.run.relics.swap_relics(index, index - 1);
                ShopCommandData::None
            }
            ShopCommand::MoveRelicRight { index } => {
                if index + 1 >= self.run.relics.active.len() {
                    return ShopCommandOutcome::rejected(
                        command,
                        before,
                        ShopCommandRejection::InvalidTarget,
                    );
                }
                self.run.relics.swap_relics(index, index + 1);
                ShopCommandData::None
            }
            ShopCommand::BuyZodiac { zodiac, price } => {
                if self.run.gold < price as i32 {
                    return ShopCommandOutcome::rejected(
                        command,
                        before,
                        ShopCommandRejection::InsufficientGold,
                    );
                }
                self.run
                    .apply_gold_delta(-(price as i32), Some(&mut self.bus));
                self.bus
                    .push(GameEvent::UiSound(crate::audio::SfxId::Purchase));
                let yaku = zodiac.yaku();
                let new_level = self.run.yaku_levels.level_up(yaku);
                ShopCommandData::ZodiacApplied {
                    zodiac_kind: zodiac,
                    yaku_name: yaku.name(),
                    new_level,
                }
            }
            ShopCommand::BuyTalisman { kind, price } => {
                if self.run.gold < price as i32 {
                    return ShopCommandOutcome::rejected(
                        command,
                        before,
                        ShopCommandRejection::InsufficientGold,
                    );
                }
                if self.run.consumables.is_full() {
                    return ShopCommandOutcome::rejected(
                        command,
                        before,
                        ShopCommandRejection::InventoryFull,
                    );
                }
                self.run
                    .apply_gold_delta(-(price as i32), Some(&mut self.bus));
                self.run.consumables.items.push(Consumable::Talisman(kind));
                self.bus
                    .push(GameEvent::UiSound(crate::audio::SfxId::Purchase));
                self.bus.push(GameEvent::TalismanPurchased(kind));
                ShopCommandData::None
            }
            ShopCommand::SellConsumable { index } => {
                if index >= self.run.consumables.items.len() {
                    return ShopCommandOutcome::rejected(
                        command,
                        before,
                        ShopCommandRejection::InvalidTarget,
                    );
                }
                let consumable = self.run.consumables.items[index];
                let refund = consumable_sell_price_for_mode(
                    consumable,
                    &self.run.mode,
                    &self.run.relics,
                );
                self.run.consumables.items.remove(index);
                self.run
                    .apply_gold_reward(refund as i32, Some(&mut self.bus));
                self.bus.push(GameEvent::UiSound(crate::audio::SfxId::Sell));
                ShopCommandData::None
            }
            ShopCommand::UseConsumable { index } => {
                let Some(result) = self.run.use_consumable(index, self.bus) else {
                    return ShopCommandOutcome::rejected(
                        command,
                        before,
                        ShopCommandRejection::NoEffect,
                    );
                };
                match result {
                    ConsumableUseResult::Zodiac { yaku, new_level } => {
                        let zodiac_kind = crate::core::zodiac::ZodiacKind::for_yaku(yaku)
                            .expect("shop zodiac use should map back to zodiac kind");
                        ShopCommandData::ZodiacApplied {
                            zodiac_kind,
                            yaku_name: yaku.name(),
                            new_level,
                        }
                    }
                    ConsumableUseResult::Talisman { .. } => ShopCommandData::None,
                }
            }
            ShopCommand::BuyPack { kind, price } => {
                if self.run.gold < price as i32 {
                    return ShopCommandOutcome::rejected(
                        command,
                        before,
                        ShopCommandRejection::InsufficientGold,
                    );
                }
                self.run
                    .apply_gold_delta(-(price as i32), Some(&mut self.bus));
                let pack_idx = self.run.tile_packs.len();
                let start_id = PACK_TILE_ID_BASE + (pack_idx as u32) * PACK_ID_STRIDE;
                let mut tiles = kind.generate_tiles(start_id);
                if let Some(enh) = kind.pre_enhancement() {
                    for t in &mut tiles {
                        self.run.tile_enhancements.insert(t.id, enh);
                        t.enhancement = Some(enh);
                    }
                }
                self.run.tile_packs.push(kind);
                self.bus.push(GameEvent::PackBought);
                ShopCommandData::PackBought {
                    tiles,
                    pack_name: kind.name(),
                    pack_kind: kind,
                }
            }
            ShopCommand::RerollShop { cost } => {
                let mut gold_cost = cost;
                if gold_cost > 0
                    && self.run.relics.has(RelicId::IGotAGuy)
                    && self
                        .run
                        .relic_counters
                        .get(&RelicId::IGotAGuy)
                        .copied()
                        .unwrap_or(0)
                        > 0
                {
                    if let Some(n) = self.run.relic_counters.get_mut(&RelicId::IGotAGuy) {
                        *n -= 1;
                    }
                    gold_cost = 0;
                }
                if self.run.gold < gold_cost as i32 {
                    return ShopCommandOutcome::rejected(
                        command,
                        before,
                        ShopCommandRejection::InsufficientGold,
                    );
                }
                self.run
                    .apply_gold_delta(-(gold_cost as i32), Some(&mut self.bus));
                ShopCommandData::Rerolled
            }
        };

        self.finish_shop_outcome(command, before, queue_start, data, None)
    }

    fn finish_outcome(
        &self,
        command: GameCommand,
        before: EngineSnapshot,
        queue_start: usize,
        data: CommandData,
        rejection: Option<CommandRejection>,
    ) -> CommandOutcome {
        let after = self.snapshot();
        let bus_events = &self.bus.queue[queue_start..];
        let mut events = self.map_bus_events(bus_events);
        let score_delta = after.round_score.saturating_sub(before.round_score);
        if score_delta > 0
            && !events
                .iter()
                .any(|event| matches!(event, EngineEvent::ScoreChanged { .. }))
        {
            events.push(EngineEvent::ScoreChanged { delta: score_delta });
        }
        let gold_delta = after.gold - before.gold;
        if gold_delta != 0
            && !events
                .iter()
                .any(|event| matches!(event, EngineEvent::GoldChanged { .. }))
        {
            events.push(EngineEvent::GoldChanged { delta: gold_delta });
        }
        if matches!(data, CommandData::TriggerStructure { earned } if earned > 0)
            && !events
                .iter()
                .any(|event| matches!(event, EngineEvent::StructureTriggered { .. }))
            && let CommandData::TriggerStructure { earned } = data
        {
            events.push(EngineEvent::StructureTriggered { earned });
        }
        if let CommandData::UseConsumable { result } = data {
            events.push(EngineEvent::ConsumableUsed { result });
        }
        if let CommandData::ApplyBlind { blind } = data {
            events.push(EngineEvent::BlindApplied { blind });
        }
        if let CommandData::SkipBlind { tag: Some(tag) } = data {
            events.push(EngineEvent::TagApplied { tag });
            events.push(EngineEvent::BlindSkipped {
                next_blind: self.run.upcoming_blind,
            });
        }

        let mut ui_hints = Self::ui_hints(before, after);
        if matches!(data, CommandData::ApplyBlind { .. }) {
            ui_hints.push(UiHint::Blind);
        }
        if matches!(data, CommandData::SkipBlind { .. }) {
            ui_hints.push(UiHint::Blind);
            ui_hints.push(UiHint::Round);
        }

        CommandOutcome {
            command,
            data,
            rejection,
            before,
            after,
            ui_hints,
            events,
        }
    }

    fn map_bus_events(&self, bus_events: &[GameEvent]) -> Vec<EngineEvent> {
        let mut engine_events = Vec::new();
        let mut drawn = 0usize;
        let mut discarded = 0usize;
        for event in bus_events {
            match event {
                GameEvent::TileDrawn => drawn += 1,
                GameEvent::TileDiscarded => discarded += 1,
                GameEvent::GoldChanged { delta } => {
                    engine_events.push(EngineEvent::GoldChanged { delta: *delta });
                }
                GameEvent::RoundComplete { reached_target, .. } => {
                    engine_events.push(EngineEvent::RoundComplete {
                        reached_target: *reached_target,
                    });
                }
                GameEvent::GameOver { reason } => {
                    engine_events.push(EngineEvent::GameOver { reason: *reason });
                }
                GameEvent::StructureCommitted => {
                    engine_events.push(EngineEvent::StructureCommitted)
                }
                GameEvent::TutorialMilestone(milestone) => {
                    engine_events.push(EngineEvent::TutorialMilestone(*milestone));
                }
                GameEvent::RelicActivated(id) => {
                    engine_events.push(EngineEvent::RelicActivated(*id));
                }
                GameEvent::InvalidAction => engine_events.push(EngineEvent::InvalidAction),
                GameEvent::TalismanUsed(kind) => {
                    engine_events.push(EngineEvent::TalismanUsed(*kind));
                }
                GameEvent::ZodiacLevelUp => engine_events.push(EngineEvent::ZodiacLevelUp),
                GameEvent::YakuScored(yaku) => engine_events.push(EngineEvent::YakuScored(*yaku)),
                _ => {}
            }
        }
        if drawn > 0 {
            engine_events.push(EngineEvent::TilesDrawn { count: drawn });
        }
        if discarded > 0 {
            engine_events.push(EngineEvent::TilesDiscarded { count: discarded });
        }
        engine_events
    }

    fn ui_hints(before: EngineSnapshot, after: EngineSnapshot) -> Vec<UiHint> {
        let mut hints = Vec::new();
        if before.hand_len != after.hand_len {
            hints.push(UiHint::Hand);
        }
        if before.selected_count != after.selected_count {
            hints.push(UiHint::Selection);
        }
        if before.round_score != after.round_score {
            hints.push(UiHint::Score);
        }
        if before.structure_tiles != after.structure_tiles
            || before.structure_sets != after.structure_sets
        {
            hints.push(UiHint::Structure);
        }
        if before.consumable_count != after.consumable_count {
            hints.push(UiHint::Consumables);
        }
        if before.plays_remaining != after.plays_remaining
            || before.discards_remaining != after.discards_remaining
            || before.gold != after.gold
        {
            hints.push(UiHint::Resources);
        }
        if before.plays_remaining != after.plays_remaining
            || before.discards_remaining != after.discards_remaining
        {
            hints.push(UiHint::Round);
        }
        hints
    }

    fn finish_shop_outcome(
        &self,
        command: ShopCommand,
        before: EngineSnapshot,
        queue_start: usize,
        data: ShopCommandData,
        rejection: Option<ShopCommandRejection>,
    ) -> ShopCommandOutcome {
        let after = self.snapshot();
        let mut events = self.map_bus_events(&self.bus.queue[queue_start..]);
        let gold_delta = after.gold - before.gold;
        if gold_delta != 0
            && !events
                .iter()
                .any(|event| matches!(event, EngineEvent::GoldChanged { .. }))
        {
            events.push(EngineEvent::GoldChanged { delta: gold_delta });
        }
        if matches!(data, ShopCommandData::ZodiacApplied { .. }) {
            events.push(EngineEvent::ZodiacLevelUp);
        }
        ShopCommandOutcome {
            command,
            data,
            rejection,
            before,
            after,
            events,
            ui_hints: Self::ui_hints(before, after),
        }
    }
}

/// Stake-aware consumable sell price — half the stake-scaled buy price, floor 1.
/// This is the only sell-price path; there is no "raw" variant because every
/// purchase is made at the stake-scaled price.
pub(crate) fn consumable_sell_price_for_mode(
    c: Consumable,
    mode: &crate::game::game_mode::GameMode,
    relics: &RelicState,
) -> u32 {
    let base = match c {
        Consumable::Zodiac(_) => crate::core::zodiac::ZodiacKind::shop_price(),
        Consumable::Talisman(t) => t.shop_price(),
    };
    let paid = mode.scale_shop_price(apply_merchants_eye_discount(base, relics));
    (paid / 2).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::consumable::Consumable;
    use crate::core::deck::{Wall, build_wall};
    use crate::core::hand::{DetectedMeld, MeldKind};
    use crate::core::tile::{Suit, Tile};
    use crate::core::zodiac::ZodiacKind;
    use crate::game::game_mode::GameMode;

    fn deterministic_run() -> RunState {
        let mut run = RunState::new(GameMode::standard());
        let tiles = build_wall();
        let mut wall = Wall::from_unshuffled(tiles);
        let target = crate::core::boss::effective_hand_size(&run);
        let mut hand = Vec::with_capacity(target);
        for _ in 0..target {
            hand.push(wall.draw().expect("enough tiles for deterministic hand"));
        }
        run.wall = wall;
        *run.hand_mut() = hand;
        *run.selected_mut() = vec![false; run.hand().len()];
        run.structure_sets_mut().clear();
        run.structure_tiles_mut().clear();
        run.last_breakdown = None;
        run
    }

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    fn assert_hand_selection_invariant(run: &RunState) {
        assert_eq!(
            run.hand().len(),
            run.selected_slice().len(),
            "hand and selected mask must stay the same length"
        );
    }

    #[test]
    fn discard_command_emits_semantic_events() {
        let mut run = deterministic_run();
        run.selected_mut()[0] = true;
        run.selected_mut()[1] = true;
        let mut bus = EventBus::default();
        let mut engine = GameEngine::new(&mut run, &mut bus);

        let outcome = engine.dispatch(GameCommand::DiscardSelectionNoRefill);

        assert_eq!(outcome.data, CommandData::DiscardSelection { count: 2 });
        assert_eq!(outcome.rejection, None);
        assert!(
            outcome
                .events
                .contains(&EngineEvent::TilesDiscarded { count: 2 })
        );
        assert!(outcome.ui_hints.contains(&UiHint::Hand));
        assert_eq!(outcome.after.hand_len, outcome.before.hand_len - 2);
        assert_hand_selection_invariant(&run);
    }

    #[test]
    fn commit_selection_rejects_invalid_melds() {
        let mut run = deterministic_run();
        *run.hand_mut() = vec![
            tile(Suit::Characters, 1, 0),
            tile(Suit::Bamboos, 4, 1),
            tile(Suit::Dots, 9, 2),
        ];
        *run.selected_mut() = vec![true, true, true];
        let mut bus = EventBus::default();
        let mut engine = GameEngine::new(&mut run, &mut bus);

        let outcome = engine.dispatch(GameCommand::CommitSelection);

        assert_eq!(outcome.rejection, Some(CommandRejection::InvalidSelection));
        assert!(outcome.events.contains(&EngineEvent::InvalidAction));
        assert_eq!(outcome.before, outcome.after);
        assert_hand_selection_invariant(&run);
    }

    #[test]
    fn use_consumable_command_reports_payload() {
        let mut run = deterministic_run();
        run.consumables
            .items
            .push(Consumable::Zodiac(ZodiacKind::Rat));
        let mut bus = EventBus::default();
        let mut engine = GameEngine::new(&mut run, &mut bus);

        let outcome = engine.dispatch(GameCommand::UseConsumable { index: 0 });

        match outcome.data {
            CommandData::UseConsumable {
                result: ConsumableUseResult::Zodiac { .. },
            } => {}
            other => panic!("unexpected command outcome: {other:?}"),
        }
        assert!(
            outcome
                .events
                .iter()
                .any(|event| matches!(event, EngineEvent::ConsumableUsed { .. }))
        );
        assert!(outcome.ui_hints.contains(&UiHint::Consumables));
    }

    #[test]
    fn refill_command_is_deterministic_for_identical_state() {
        let mut run_a = deterministic_run();
        let mut run_b = deterministic_run();
        run_a.hand_mut().truncate(10);
        run_b.hand_mut().truncate(10);
        *run_a.selected_mut() = vec![false; run_a.hand().len()];
        *run_b.selected_mut() = vec![false; run_b.hand().len()];
        let mut bus_a = EventBus::default();
        let mut bus_b = EventBus::default();

        let outcome_a = GameEngine::new(&mut run_a, &mut bus_a).dispatch(GameCommand::RefillHand);
        let outcome_b = GameEngine::new(&mut run_b, &mut bus_b).dispatch(GameCommand::RefillHand);

        assert_eq!(outcome_a.after, outcome_b.after);
        assert_eq!(outcome_a.events, outcome_b.events);
        assert_eq!(run_a.hand(), run_b.hand());
        assert_hand_selection_invariant(&run_a);
        assert_hand_selection_invariant(&run_b);
    }

    #[test]
    fn sort_commands_preserve_hand_selection_invariant() {
        let mut run = deterministic_run();
        run.selected_mut()[0] = true;
        let mut bus = EventBus::default();
        {
            let mut engine = GameEngine::new(&mut run, &mut bus);
            let _ = engine.dispatch(GameCommand::SortHandBySuit);
        }
        assert_hand_selection_invariant(&run);
        {
            let mut engine = GameEngine::new(&mut run, &mut bus);
            let _ = engine.dispatch(GameCommand::SortHandByRank);
        }
        assert_hand_selection_invariant(&run);
    }

    fn winning_structure_bank() -> (Vec<Tile>, Vec<DetectedMeld>) {
        let tiles = vec![
            tile(Suit::Characters, 1, 1),
            tile(Suit::Characters, 1, 2),
            tile(Suit::Characters, 2, 3),
            tile(Suit::Characters, 3, 4),
            tile(Suit::Characters, 4, 5),
            tile(Suit::Dots, 2, 6),
            tile(Suit::Dots, 3, 7),
            tile(Suit::Dots, 4, 8),
            tile(Suit::Bamboos, 5, 9),
            tile(Suit::Bamboos, 6, 10),
            tile(Suit::Bamboos, 7, 11),
            tile(Suit::Wind, 1, 12),
            tile(Suit::Wind, 1, 13),
            tile(Suit::Wind, 1, 14),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![9, 10, 11],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![12, 13, 14],
            },
        ];
        (tiles, sets)
    }

    #[test]
    fn trigger_structure_command_preserves_hand_selection_invariant() {
        let mut run = deterministic_run();
        run.set_auto_cash_in_on_full_structure(false);
        let (tiles, sets) = winning_structure_bank();
        *run.structure_tiles_mut() = tiles;
        *run.structure_sets_mut() = sets;
        assert!(
            run.can_trigger_structure_now(),
            "test setup should allow manual structure cash-in"
        );

        let mut bus = EventBus::default();
        let mut engine = GameEngine::new(&mut run, &mut bus);
        let outcome = engine.dispatch(GameCommand::TriggerStructure);

        assert_eq!(outcome.rejection, None);
        match outcome.data {
            CommandData::TriggerStructure { earned } => assert!(earned > 0),
            other => panic!("expected TriggerStructure outcome, got {other:?}"),
        }
        assert_hand_selection_invariant(&run);
        assert!(run.structure_sets().is_empty());
        assert!(run.structure_tiles().is_empty());
    }

    #[test]
    fn apply_blind_command_preserves_hand_selection_invariant() {
        let mut run = deterministic_run();
        run.blind = BlindKind::Small;
        run.upcoming_blind = BlindKind::Small;
        run.run_number = 1;

        let mut bus = EventBus::default();
        let mut engine = GameEngine::new(&mut run, &mut bus);
        let outcome = engine.dispatch(GameCommand::ApplyBlind {
            blind: BlindKind::Small,
        });

        assert_eq!(outcome.rejection, None);
        assert_hand_selection_invariant(&run);
        assert_eq!(run.hand().len(), run.selected_slice().len());
        assert!(!run.hand().is_empty());
    }

    #[test]
    fn skip_upcoming_blind_applies_tag_and_emits_events() {
        let mut run = deterministic_run();
        run.small_blind_tag = Some(TagKind::GoldIngot);
        run.upcoming_blind = BlindKind::Small;
        let gold_before = run.gold;
        let mut bus = EventBus::default();
        let mut engine = GameEngine::new(&mut run, &mut bus);

        let outcome = engine.dispatch(GameCommand::SkipUpcomingBlindWithTag);

        assert_eq!(
            outcome.data,
            CommandData::SkipBlind {
                tag: Some(TagKind::GoldIngot)
            }
        );
        assert!(outcome.events.contains(&EngineEvent::TagApplied {
            tag: TagKind::GoldIngot
        }));
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            EngineEvent::BlindSkipped {
                next_blind: BlindKind::Big
            }
        )));
        assert_eq!(run.gold, gold_before + 8);
    }

    #[test]
    fn buy_relic_shop_command_deducts_gold_and_adds_relic() {
        let mut run = deterministic_run();
        run.gold = 20;
        let relic = RelicId::MeltingIce;
        let price = 7;
        let mut bus = EventBus::default();

        let outcome = GameEngine::new(&mut run, &mut bus)
            .dispatch_shop(ShopCommand::BuyRelic { relic, price });

        assert_eq!(outcome.rejection, None);
        assert!(run.relics.active.contains(&relic));
        assert_eq!(run.gold, 13);
        assert_eq!(run.relic_counters.get(&RelicId::MeltingIce), Some(&80));
        assert!(outcome.ui_hints.contains(&UiHint::Resources));
        assert!(outcome.events.contains(&EngineEvent::GoldChanged {
            delta: -(price as i32)
        }));
    }

    #[test]
    fn buy_zodiac_shop_command_reports_level_up_payload() {
        let mut run = deterministic_run();
        run.gold = 10;
        let zodiac = ZodiacKind::Rat;
        let mut bus = EventBus::default();

        let outcome = GameEngine::new(&mut run, &mut bus)
            .dispatch_shop(ShopCommand::BuyZodiac { zodiac, price: 5 });

        assert_eq!(outcome.rejection, None);
        assert_eq!(run.gold, 5);
        assert_eq!(run.yaku_levels.level_of(zodiac.yaku()), 2);
        assert_eq!(
            outcome.data,
            ShopCommandData::ZodiacApplied {
                zodiac_kind: zodiac,
                yaku_name: zodiac.yaku().name(),
                new_level: 2,
            }
        );
    }

    #[test]
    fn reroll_shop_rejects_without_mutation_when_gold_is_low() {
        let mut run = deterministic_run();
        run.gold = 1;
        let before = run.gold;
        let mut bus = EventBus::default();

        let outcome =
            GameEngine::new(&mut run, &mut bus).dispatch_shop(ShopCommand::RerollShop { cost: 2 });

        assert_eq!(
            outcome.rejection,
            Some(ShopCommandRejection::InsufficientGold)
        );
        assert_eq!(run.gold, before);
        assert_eq!(outcome.before, outcome.after);
    }

    #[test]
    fn reroll_shop_spends_i_got_a_guy_charge_when_gold_is_low() {
        let mut run = deterministic_run();
        run.gold = 0;
        run.relics
            .active
            .push(crate::core::relic::RelicId::IGotAGuy);
        run.relic_counters
            .insert(crate::core::relic::RelicId::IGotAGuy, 2);
        let mut bus = EventBus::default();

        let outcome =
            GameEngine::new(&mut run, &mut bus).dispatch_shop(ShopCommand::RerollShop { cost: 5 });

        assert_eq!(outcome.rejection, None);
        assert_eq!(run.gold, 0);
        assert_eq!(
            run.relic_counters
                .get(&crate::core::relic::RelicId::IGotAGuy)
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn reroll_shop_rejects_when_gold_low_and_no_i_got_a_guy_charges() {
        let mut run = deterministic_run();
        run.gold = 0;
        run.relics
            .active
            .push(crate::core::relic::RelicId::IGotAGuy);
        run.relic_counters
            .insert(crate::core::relic::RelicId::IGotAGuy, 0);
        let before_gold = run.gold;
        let mut bus = EventBus::default();

        let outcome =
            GameEngine::new(&mut run, &mut bus).dispatch_shop(ShopCommand::RerollShop { cost: 3 });

        assert_eq!(
            outcome.rejection,
            Some(ShopCommandRejection::InsufficientGold)
        );
        assert_eq!(run.gold, before_gold);
    }

    #[test]
    fn reroll_shop_zero_cost_does_not_spend_i_got_a_guy_charge() {
        let mut run = deterministic_run();
        run.gold = 0;
        run.relics
            .active
            .push(crate::core::relic::RelicId::IGotAGuy);
        run.relic_counters
            .insert(crate::core::relic::RelicId::IGotAGuy, 3);
        let mut bus = EventBus::default();

        let outcome =
            GameEngine::new(&mut run, &mut bus).dispatch_shop(ShopCommand::RerollShop { cost: 0 });

        assert_eq!(outcome.rejection, None);
        assert_eq!(
            run.relic_counters
                .get(&crate::core::relic::RelicId::IGotAGuy)
                .copied(),
            Some(3)
        );
    }
}
