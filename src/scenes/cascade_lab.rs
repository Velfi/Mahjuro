//! Cascade Lab — debug scene for scoring cascade timing, table presets, and cash-in.
//!
//! Entered from Debug → Labs → Cascade Lab…

use crate::core::OrdealKindExt;
use crate::core::hand::{DetectedMeld, MeldKind};
use crate::core::ordeal_kind::OrdealKind;
use crate::core::relic::{RelicId, all_relic_defs};
use crate::core::rules::ChamberKind;
use crate::core::rules::RuleModifier;
use crate::core::structure_notation::{self, STRUCTURE_NOTATION_HINT, StructureNotationError};
use crate::core::tile::{Suit, Tile};
use crate::game::cascade::CascadeTuning;
use crate::render::draw_cmd::CameraParams;
use crate::render::draw_cmd::{ImageQuad, ImageQuadSource, UiFrame};
use crate::render::gameplay_glb::{
    self, PLAYER_RELIC_MARKERS, STRUCTURE_TILES_LEFT, STRUCTURE_TILES_RIGHT, TILE_PLINTH_MARKERS,
};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::controller_hints::{HintStyle, back_footer_row, push_screen_footer_hint_for};
use crate::ui::input::UiAction;
use crate::ui::ordeal_icons::ordeal_icon_source;

use super::cascade_lab_click::LabClick;
use super::gameplay::{GameplayScene, relic_tray_slot_screen_center};
use super::{ButtonAction, ButtonDef, DrawCtx, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx};

const RELIC_SLOT_COUNT: usize = PLAYER_RELIC_MARKERS.len();

const STRUCTURE_MELD_SLOTS_MAX: usize = 5;
const DORA_SLOT_MAX: usize = 2;
const ROUND_WIND_SLOT_MAX: usize = 2;

const TUNING_SLIDER_ROWS: usize = 8;

const TUNING_ROW_META: [(&str, u64, u64, u64); TUNING_SLIDER_ROWS] = [
    ("Base hold (ms)", 50, 3000, 10),
    ("Step hold (ms)", 50, 3000, 10),
    ("Tick (ms)", 50, 2000, 10),
    ("Popup pop (ms)", 40, 800, 10),
    ("Popup loiter (ms)", 80, 2000, 10),
    ("Popup fly (ms)", 80, 2000, 10),
    ("Popup overshoot", 0, 100, 1),
    ("Post-step hold (ms)", 0, 800, 10),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabTab {
    Timing,
    Table,
    RunState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabPicker {
    Relic(usize),
    Dora(usize),
    RoundWind(usize),
    Boss,
    StructureMeld(usize),
    AddStructureMeld,
}

impl LabPicker {
    fn title(self, run: &crate::game::run::RunState) -> String {
        match self {
            Self::Relic(slot) => {
                let current = CascadeLabScene::relic_at_slot(run, slot)
                    .map(|id| {
                        all_relic_defs()
                            .iter()
                            .find(|d| d.id == id)
                            .map(|d| d.name.to_string())
                            .unwrap_or_else(|| format!("{id:?}"))
                    })
                    .unwrap_or_else(|| "Empty".into());
                format!("Relic slot {} — {current}", slot + 1)
            }
            Self::Dora(i) => format!("Dora {}", i + 1),
            Self::RoundWind(i) => {
                if i == 0 {
                    "Round wind".into()
                } else {
                    "Bonus round wind (Windreader)".into()
                }
            }
            Self::Boss => {
                let current = run
                    .ordeal
                    .upcoming
                    .map(|k| k.name().to_string())
                    .unwrap_or_else(|| "None".into());
                format!("Boss — {current}")
            }
            Self::StructureMeld(i) => format!("Structure meld {}", i + 1),
            Self::AddStructureMeld => "Add meld to structure".into(),
        }
    }

    fn clear_label(self) -> Option<&'static str> {
        match self {
            Self::Relic(_) => Some("Clear slot"),
            Self::StructureMeld(_) => Some("Clear meld"),
            Self::Boss => Some("Clear boss"),
            Self::Dora(_) | Self::RoundWind(_) | Self::AddStructureMeld => None,
        }
    }
}

impl LabTab {
    const ALL: [Self; 3] = [Self::Timing, Self::Table, Self::RunState];

    fn label(self) -> &'static str {
        match self {
            Self::Timing => "Timing",
            Self::Table => "Table",
            Self::RunState => "Run",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum StructurePreset {
    StandardWin,
    Tanyao,
    YakuhaiTriplet,
}

impl StructurePreset {
    const ALL: [Self; 3] = [Self::StandardWin, Self::Tanyao, Self::YakuhaiTriplet];

    fn label(self) -> &'static str {
        match self {
            Self::StandardWin => "Standard win",
            Self::Tanyao => "Tanyao",
            Self::YakuhaiTriplet => "Yakuhai triplet",
        }
    }

    fn tiles_and_sets(self) -> (Vec<Tile>, Vec<DetectedMeld>) {
        match self {
            Self::StandardWin => standard_win_structure(),
            Self::Tanyao => tanyao_structure(),
            Self::YakuhaiTriplet => yakuhai_structure(),
        }
    }
}

fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
    Tile::new(suit, rank, id)
}

fn standard_win_structure() -> (Vec<Tile>, Vec<DetectedMeld>) {
    let tiles = vec![
        tile(Suit::Manzu, 1, 1),
        tile(Suit::Manzu, 1, 2),
        tile(Suit::Manzu, 2, 3),
        tile(Suit::Manzu, 3, 4),
        tile(Suit::Manzu, 4, 5),
        tile(Suit::Pinzu, 2, 6),
        tile(Suit::Pinzu, 3, 7),
        tile(Suit::Pinzu, 4, 8),
        tile(Suit::Souzu, 5, 9),
        tile(Suit::Souzu, 6, 10),
        tile(Suit::Souzu, 7, 11),
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

fn tanyao_structure() -> (Vec<Tile>, Vec<DetectedMeld>) {
    let tiles = vec![
        tile(Suit::Manzu, 2, 1),
        tile(Suit::Manzu, 2, 2),
        tile(Suit::Manzu, 3, 3),
        tile(Suit::Manzu, 4, 4),
        tile(Suit::Manzu, 5, 5),
        tile(Suit::Pinzu, 3, 6),
        tile(Suit::Pinzu, 4, 7),
        tile(Suit::Pinzu, 5, 8),
        tile(Suit::Souzu, 4, 9),
        tile(Suit::Souzu, 5, 10),
        tile(Suit::Souzu, 6, 11),
        tile(Suit::Manzu, 6, 12),
        tile(Suit::Manzu, 6, 13),
        tile(Suit::Manzu, 6, 14),
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

fn yakuhai_structure() -> (Vec<Tile>, Vec<DetectedMeld>) {
    let (mut tiles, mut sets) = standard_win_structure();
    tiles[11] = tile(Suit::Dragon, 1, 12);
    tiles[12] = tile(Suit::Dragon, 1, 13);
    tiles[13] = tile(Suit::Dragon, 1, 14);
    sets[4] = DetectedMeld {
        kind: MeldKind::Triplet,
        tile_ids: vec![12, 13, 14],
    };
    (tiles, sets)
}

const STRUCTURE_TEXT_MAX: usize = 128;

const MELD_PRESETS: &[&str] = &[
    "11m", "22p", "33s", "44m", "55p", "66s", "77m", "88p", "99s", "123m", "234p", "345s", "456m",
    "567p", "678s", "789m", "111m", "222p", "333s", "eee", "sss", "www", "nnn", "rrr", "ggg",
    "whwh", "f1f2f3",
];

fn plinth_hit_rect(
    projected: Option<[f32; 4]>,
    anchor: &[f32; 3],
    layout: &crate::ui::layout::LayoutResult,
    tile_count: usize,
) -> [f32; 4] {
    if let Some(r) = projected
        && r[2] > 1.0
        && r[3] > 1.0
        && r[0].is_finite()
        && r[1].is_finite()
    {
        return r;
    }
    let spacing = layout.mm(24.0);
    let tile_w = layout.mm(22.0);
    let strip_w = if tile_count >= 2 {
        spacing + tile_w
    } else {
        tile_w
    };
    let strip_h = layout.mm(30.0);
    [
        anchor[0] - strip_w * 0.5,
        anchor[1] - strip_h * 0.5,
        strip_w,
        strip_h,
    ]
}

fn split_rect_slots(rect: [f32; 4], index: usize, count: usize) -> [f32; 4] {
    if count <= 1 {
        return rect;
    }
    let [x, y, w, h] = rect;
    let cw = w / count as f32;
    [x + cw * index as f32, y, cw, h]
}

fn split_rect_by_weights(rect: [f32; 4], weights: &[usize], index: usize) -> [f32; 4] {
    let [x, y, w, h] = rect;
    let total = weights.iter().sum::<usize>().max(1) as f32;
    let mut off = 0.0f32;
    for (i, &wt) in weights.iter().enumerate() {
        let frac = wt as f32 / total;
        let cw = w * frac;
        if i == index {
            return [x + off, y, cw, h];
        }
        off += cw;
    }
    rect
}

pub struct CascadeLabScene {
    has_suspended: bool,
    gameplay: GameplayScene,
    preset_idx: usize,
    structure_text: String,
    structure_field_focused: bool,
    structure_submit_pending: bool,
    structure_error: Option<String>,
    pub tuning: CascadeTuning,
    tuning_cursor: usize,
    dragging_slider: Option<usize>,
    prev_mouse_down: bool,
    prepared: bool,
    panel_collapsed: bool,
    active_tab: LabTab,
    active_picker: Option<LabPicker>,
    picker_scroll: f32,
    counter_edit_slot: usize,
    sorted_relic_indices: Vec<usize>,
    sorted_ordeal_indices: Vec<usize>,
}

impl CascadeLabScene {
    pub fn new(has_suspended: bool, tuning: CascadeTuning) -> Self {
        let mut sorted_relic_indices: Vec<usize> = (0..all_relic_defs().len()).collect();
        sorted_relic_indices.sort_by_key(|&i| all_relic_defs()[i].name);
        let mut sorted_ordeal_indices: Vec<usize> = (0..OrdealKind::ALL.len()).collect();
        sorted_ordeal_indices.sort_by_key(|&i| OrdealKind::ALL[i].name());
        Self {
            has_suspended,
            gameplay: GameplayScene::new(),
            preset_idx: 0,
            structure_text: String::new(),
            structure_field_focused: false,
            structure_submit_pending: false,
            structure_error: None,
            tuning,
            tuning_cursor: 0,
            dragging_slider: None,
            prev_mouse_down: false,
            prepared: false,
            panel_collapsed: false,
            active_tab: LabTab::Table,
            active_picker: None,
            picker_scroll: 0.0,
            counter_edit_slot: 0,
            sorted_relic_indices,
            sorted_ordeal_indices,
        }
    }

    fn leave_lab(&mut self, run: &mut crate::game::run::RunState) {
        run.suppress_chamber_resolution = false;
        self.gameplay.exit_lab_mode();
    }

    fn go_back(
        &mut self,
        run: &mut crate::game::run::RunState,
        overlay_request: &mut Option<super::OverlayRequest>,
    ) -> SceneTransition {
        self.leave_lab(run);
        if self.has_suspended {
            *overlay_request = Some(super::OverlayRequest::Pop);
            None
        } else {
            Some(SceneIntent::MainMenu)
        }
    }

    fn preset(&self) -> StructurePreset {
        StructurePreset::ALL[self.preset_idx % StructurePreset::ALL.len()]
    }

    fn prepare_run(&mut self, run: &mut crate::game::run::RunState) {
        run.suppress_chamber_resolution = true;
        run.set_auto_cash_in_on_full_structure(false);
        run.round_rules
            .retain(|r| r != &RuleModifier::CashInRequiresNoDiscards);
        run.plays_remaining = run.plays_max.max(4);
        run.discards_remaining = run.discards_max.max(3);
        self.gameplay.enter_lab_mode();
        self.apply_preset(run);
        self.sync_structure_text_from_preset();
        Self::sync_table_relic_props(run);
        run.chamber = ChamberKind::Small;
        run.upcoming_chamber = ChamberKind::Small;
        self.gameplay.lab_sync_score_display(run.round_score);
        self.prepared = true;
    }

    fn apply_lab_boss_effect(run: &mut crate::game::run::RunState) {
        run.chamber = ChamberKind::Ordeal;
        run.upcoming_chamber = ChamberKind::Ordeal;
        if let Some(eff) = run.ordeal.effect.take() {
            for &m in &eff.rule_pushes {
                if !run.round_rules.contains(&m) {
                    run.round_rules.push(m);
                }
            }
            run.tile_debuffs = eff.tile_debuffs.clone();
            run.relics.set_debuffed(eff.relic_debuffs.iter().copied());
            if let Some(hook) = eff.on_apply {
                hook(run);
            }
            run.ordeal.effect = Some(eff);
        }
    }

    fn set_lab_boss(run: &mut crate::game::run::RunState, kind: OrdealKind) {
        Self::clear_lab_boss(run);
        run.ordeal.upcoming = Some(kind);
        run.resolve_upcoming_ordeal();
        Self::apply_lab_boss_effect(run);
    }

    fn clear_lab_boss(run: &mut crate::game::run::RunState) {
        if let Some(eff) = run.ordeal.effect.take() {
            for m in eff.rule_pushes {
                run.round_rules.retain(|r| r != &m);
            }
        }
        run.ordeal.upcoming = None;
        run.ordeal.effect = None;
        run.ordeal.bonus_hand_size = 0;
        run.ordeal.yen_cost_per_play = 0;
        run.ordeal.tax_collector_cost = 0;
        run.tile_debuffs.clear();
        run.relics.clear_debuffs();
        run.chamber = ChamberKind::Small;
        run.upcoming_chamber = ChamberKind::Small;
    }

    fn sync_table_relic_props(run: &mut crate::game::run::RunState) {
        run.refresh_windreader_bonus_wind();
        if run.relics.has(RelicId::DoraCrown) && run.wall.dora_indicator_tiles().len() < 2 {
            run.wall.reveal_extra_dora_indicator();
        }
    }

    fn sync_structure_text_from_run(&mut self, run: &crate::game::run::RunState) {
        self.structure_text = structure_notation::format_structure_notation(
            run.structure_tiles(),
            run.structure_sets(),
        );
        self.structure_error = None;
    }

    fn dora_slot_count(run: &crate::game::run::RunState) -> usize {
        if run.relics.has(RelicId::DoraCrown) {
            2
        } else {
            1
        }
    }

    fn round_wind_slot_count(run: &crate::game::run::RunState) -> usize {
        if run.relics.has(RelicId::WindReader) {
            2
        } else {
            1
        }
    }

    fn set_dora_slot(run: &mut crate::game::run::RunState, index: usize, suit: Suit, rank: u8) {
        let mut faces: Vec<(Suit, u8)> = run
            .wall
            .dora_indicator_tiles()
            .iter()
            .map(|t| (t.suit, t.rank))
            .collect();
        let need = Self::dora_slot_count(run).max(index + 1);
        faces.resize(need, (suit, rank));
        faces[index] = (suit, rank);
        run.wall.set_dora_indicator_faces(&faces);
    }

    fn set_round_wind_slot(run: &mut crate::game::run::RunState, index: usize, rank: u8) {
        let rank = rank.clamp(1, 4);
        if index == 0 {
            run.wing = rank as u32;
        } else {
            run.windreader_bonus_wind = Some(rank);
        }
    }

    fn clear_structure_meld(run: &mut crate::game::run::RunState, slot: usize) {
        if slot >= run.structure_sets().len() {
            return;
        }
        let ids = run.structure_sets()[slot].tile_ids.clone();
        run.structure_sets_mut().remove(slot);
        run.structure_tiles_mut().retain(|t| !ids.contains(&t.id));
    }

    fn apply_meld_preset(
        run: &mut crate::game::run::RunState,
        slot: Option<usize>,
        token: &str,
    ) -> bool {
        let Ok((mut tiles, mut sets)) = structure_notation::parse_structure_notation(token) else {
            return false;
        };
        if sets.len() != 1 {
            return false;
        }
        if let Some(i) = slot {
            Self::clear_structure_meld(run, i);
        }
        let base = run
            .structure_tiles()
            .iter()
            .map(|t| t.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        for (j, t) in tiles.iter_mut().enumerate() {
            t.id = base + j as u32;
        }
        let meld = sets.remove(0);
        run.structure_tiles_mut().extend(tiles);
        if let Some(i) = slot {
            let at = i.min(run.structure_sets_mut().len());
            run.structure_sets_mut().insert(at, meld);
        } else {
            run.structure_sets_mut().push(meld);
        }
        true
    }

    fn apply_preset(&self, run: &mut crate::game::run::RunState) {
        let (tiles, sets) = self.preset().tiles_and_sets();
        self.apply_structure(run, tiles, sets);
    }

    fn apply_structure(
        &self,
        run: &mut crate::game::run::RunState,
        tiles: Vec<Tile>,
        sets: Vec<DetectedMeld>,
    ) {
        *run.structure_tiles_mut() = tiles;
        *run.structure_sets_mut() = sets;
    }

    fn sync_structure_text_from_preset(&mut self) {
        let (tiles, sets) = self.preset().tiles_and_sets();
        self.structure_text = structure_notation::format_structure_notation(&tiles, &sets);
        self.structure_error = None;
    }

    fn apply_structure_from_text(&mut self, run: &mut crate::game::run::RunState) -> bool {
        let input = self.structure_text.trim();
        if input.is_empty() {
            self.apply_preset(run);
            self.structure_error = None;
            return true;
        }
        match structure_notation::parse_structure_notation(input) {
            Ok((tiles, sets)) => {
                self.apply_structure(run, tiles, sets);
                self.structure_error = None;
                true
            }
            Err(StructureNotationError(msg)) => {
                self.structure_error = Some(msg);
                false
            }
        }
    }

    /// Keyboard typing for the structure field. Returns `true` if consumed.
    pub fn feed_structure_key(
        &mut self,
        scancode: Option<sdl3::keyboard::Scancode>,
        shift: bool,
    ) -> bool {
        if !self.structure_field_focused {
            return false;
        }
        let Some(code) = scancode else {
            return false;
        };
        use sdl3::keyboard::Scancode;
        match code {
            Scancode::Backspace => {
                self.structure_text.pop();
                self.structure_error = None;
                true
            }
            Scancode::Escape => {
                self.structure_field_focused = false;
                true
            }
            Scancode::Return | Scancode::KpEnter => {
                self.structure_submit_pending = true;
                true
            }
            Scancode::Space => {
                self.push_structure_char(' ');
                true
            }
            _ => {
                if let Some(c) = scancode_to_structure_char(code, shift) {
                    self.push_structure_char(c);
                    true
                } else {
                    false
                }
            }
        }
    }

    fn push_structure_char(&mut self, c: char) {
        if self.structure_text.len() >= STRUCTURE_TEXT_MAX {
            return;
        }
        if c.is_ascii() {
            self.structure_text.push(c.to_ascii_lowercase());
            self.structure_error = None;
        }
    }

    fn reset_score(&mut self, run: &mut crate::game::run::RunState) {
        run.round_score = 0;
        self.gameplay.lab_reset_score_state();
        self.apply_preset(run);
        self.sync_structure_text_from_preset();
    }

    fn ensure_relic_capacity(run: &mut crate::game::run::RunState) {
        run.relics.max_slots = run.relics.max_slots.max(RELIC_SLOT_COUNT);
    }

    fn assign_relic_slot(run: &mut crate::game::run::RunState, slot: usize, rid: RelicId) {
        if slot >= RELIC_SLOT_COUNT {
            return;
        }
        Self::ensure_relic_capacity(run);
        // Active relics are a dense list — clicking a physical tray slot beyond
        // the next open index appends rather than silently doing nothing.
        let assign_slot = slot.min(run.relics.active.len());
        if assign_slot < run.relics.active.len() {
            run.relics.active[assign_slot] = rid;
        } else if run.relics.active.len() < RELIC_SLOT_COUNT {
            run.relics.active.push(rid);
        }
        run.recompute_capacities();
        Self::sync_table_relic_props(run);
    }

    fn relic_slot_hit_rect(
        w: f32,
        h: f32,
        env_h: f32,
        slot: usize,
        proj: &crate::render::wgpu_renderer::ProjectionCache,
        cam: Option<&CameraParams>,
    ) -> Option<[f32; 4]> {
        if let Some([rx, ry, rw, rh]) = proj.relic_rects.get(slot).copied()
            && rw > 1.0
            && rh > 1.0
            && rx.is_finite()
            && ry.is_finite()
        {
            let scale = metrics::scene_scale(w, h);
            let pad = (6.0 * scale).max(4.0);
            return Some([rx - pad, ry - pad, rw + pad * 2.0, rh + pad * 2.0]);
        }
        let hit = (44.0 * metrics::scene_scale(w, h)).max(32.0);
        if let Some(cam) = cam {
            let name = PLAYER_RELIC_MARKERS.get(slot)?;
            return gameplay_glb::with_gameplay_glb_cpu(|cpu| {
                let cpu = cpu?;
                gameplay_glb::gameplay_marker_screen_rect(w, h, cam, env_h, cpu, name, hit, hit)
            });
        }
        relic_tray_slot_screen_center(w, h, env_h, slot)
            .map(|(cx, cy)| [cx - hit * 0.5, cy - hit * 0.5, hit, hit])
    }

    fn is_picker_button(id: u32) -> bool {
        LabClick::from_id(id).is_some_and(LabClick::is_picker)
    }

    fn picker_row_count(picker: LabPicker) -> usize {
        match picker {
            LabPicker::Relic(_) => all_relic_defs().len(),
            LabPicker::Dora(_) => dora_picker_faces().len(),
            LabPicker::RoundWind(_) => 4,
            LabPicker::Boss => OrdealKind::ALL.len(),
            LabPicker::StructureMeld(_) | LabPicker::AddStructureMeld => MELD_PRESETS.len(),
        }
    }

    /// Picker rows / chrome must beat other lab rects; dim dismiss sits just
    /// above gameplay so outside clicks close without stealing picker hits.
    fn reorder_lab_hit_buttons(
        picker_open: bool,
        w: f32,
        h: f32,
        lab_buttons: Vec<ButtonDef>,
        gameplay_buttons: Vec<ButtonDef>,
    ) -> Vec<ButtonDef> {
        if !picker_open {
            return lab_buttons.into_iter().chain(gameplay_buttons).collect();
        }
        let (picker_btns, rest): (Vec<_>, Vec<_>) = lab_buttons.into_iter().partition(
            |btn| matches!(btn.action, ButtonAction::Scene(id) if Self::is_picker_button(id)),
        );
        let mut ordered =
            Vec::with_capacity(picker_btns.len() + rest.len() + gameplay_buttons.len() + 1);
        ordered.extend(picker_btns);
        ordered.extend(rest);
        ordered.push(ButtonDef::scene(
            (0.0, 0.0, w, h),
            LabClick::PickerClose.id(),
        ));
        ordered.extend(gameplay_buttons);
        ordered
    }

    fn clear_relic_slot(run: &mut crate::game::run::RunState, slot: usize) {
        if slot < run.relics.active.len() {
            run.relics.active.remove(slot);
            run.recompute_capacities();
            Self::sync_table_relic_props(run);
        }
    }

    fn relic_at_slot(run: &crate::game::run::RunState, slot: usize) -> Option<RelicId> {
        run.relics.active.get(slot).copied()
    }

    fn counter_relic_for_slot(run: &crate::game::run::RunState, slot: usize) -> Option<RelicId> {
        Self::relic_at_slot(run, slot)
    }

    fn cash_in(&mut self, ctx: &mut UpdateCtx<'_>) {
        self.apply_structure_from_text(ctx.run);
        self.gameplay.lab_cash_in(ctx);
    }

    fn panel_layout(w: f32, h: f32, collapsed: bool) -> (f32, f32, f32, f32) {
        let scale = metrics::scene_scale(w, h);
        let panel_w = (340.0 * scale).min(w * 0.38);
        let _tab_h = (26.0 * scale).max(20.0);
        let panel_h = if collapsed {
            (28.0 * scale).max(22.0)
        } else {
            (h * 0.42).clamp(220.0 * scale, h * 0.55)
        };
        let panel_x = (10.0 * scale).max(6.0);
        let panel_y = h - panel_h - (10.0 * scale).max(6.0);
        (panel_x, panel_y, panel_w, panel_h)
    }

    /// Shared header geometry for the expanded lab panel (tabs + body origin).
    fn expanded_panel_body_y(panel_y: f32, scale: f32) -> f32 {
        let pad = (8.0 * scale).max(5.0);
        let tab_h = (26.0 * scale).max(20.0);
        let title_y = panel_y + pad;
        let tabs_y = title_y + tab_h + pad * 0.5;
        tabs_y + tab_h + pad
    }

    fn timing_tab_layout(
        _panel_x: f32,
        body_y: f32,
        panel_w: f32,
        scale: f32,
    ) -> (f32, f32, f32, f32, f32) {
        let row_h = (18.0 * scale).max(14.0);
        let row_gap = (2.0 * scale).max(1.0);
        let label_w = panel_w * 0.48;
        let slider_w = panel_w * 0.26;
        (body_y, row_h, row_gap, label_w, slider_w)
    }

    fn tuning_row_value(&self, row: usize) -> f32 {
        match row {
            0 => self.tuning.base_hold_ms as f32,
            1 => self.tuning.step_hold_ms as f32,
            2 => self.tuning.tick_duration_ms as f32,
            3 => self.tuning.popup_pop_ms as f32,
            4 => self.tuning.popup_loiter_ms as f32,
            5 => self.tuning.popup_fly_ms as f32,
            6 => self.tuning.popup_overshoot * 100.0,
            7 => self.tuning.total_hold_ms as f32,
            _ => 0.0,
        }
    }

    fn set_tuning_row_value(&mut self, row: usize, v: f32) {
        match row {
            0 => self.tuning.base_hold_ms = v.round() as u64,
            1 => self.tuning.step_hold_ms = v.round() as u64,
            2 => self.tuning.tick_duration_ms = v.round() as u64,
            3 => self.tuning.popup_pop_ms = v.round() as u64,
            4 => self.tuning.popup_loiter_ms = v.round() as u64,
            5 => self.tuning.popup_fly_ms = v.round() as u64,
            6 => self.tuning.popup_overshoot = (v / 100.0).clamp(0.0, 0.8),
            7 => self.tuning.total_hold_ms = v.round() as u64,
            _ => {}
        }
    }

    fn update_tuning_drag(&mut self, ctx: &UpdateCtx<'_>, w: f32, h: f32) {
        if self.panel_collapsed || self.active_tab != LabTab::Timing {
            return;
        }
        let scale = metrics::scene_scale(w, h);
        let (panel_x, panel_y, panel_w, _panel_h) = Self::panel_layout(w, h, false);
        let body_y = Self::expanded_panel_body_y(panel_y, scale);
        let (rows_y0, row_h, row_gap, label_w, slider_w) =
            Self::timing_tab_layout(panel_x, body_y, panel_w, scale);
        let (mx, my) = ctx.cursor_pos;
        let mouse_down = ctx.mouse_left_down;
        let mouse_click = mouse_down && !self.prev_mouse_down;

        if let Some(di) = self.dragging_slider
            && mouse_down
            && di < TUNING_SLIDER_ROWS
        {
            let track_x = panel_x + label_w;
            let t = ((mx - track_x) / slider_w.max(1e-6)).clamp(0.0, 1.0);
            let (_, min, max, _) = TUNING_ROW_META[di];
            self.set_tuning_row_value(di, min as f32 + t * (max - min) as f32);
        }

        if mouse_click || (mouse_down && self.dragging_slider.is_none()) {
            for (i, _) in TUNING_ROW_META.iter().enumerate().take(TUNING_SLIDER_ROWS) {
                let row_y = rows_y0 + i as f32 * (row_h + row_gap);
                let track_x = panel_x + label_w;
                if mx >= track_x && mx <= track_x + slider_w && my >= row_y && my <= row_y + row_h {
                    self.tuning_cursor = i;
                    let (_, min, max, _) = TUNING_ROW_META[i];
                    let t = ((mx - track_x) / slider_w.max(1e-6)).clamp(0.0, 1.0);
                    self.set_tuning_row_value(i, min as f32 + t * (max - min) as f32);
                    if mouse_down {
                        self.dragging_slider = Some(i);
                    }
                    break;
                }
            }
        }

        if !mouse_down {
            self.dragging_slider = None;
        }
    }

    fn handle_picker_scroll(&mut self, scroll_lines: f32, w: f32, h: f32) {
        let Some(picker) = self.active_picker else {
            return;
        };
        if scroll_lines.abs() < f32::EPSILON {
            return;
        }
        let scale = metrics::scene_scale(w, h);
        let row_h = (52.0 * scale).max(40.0);
        let rows = Self::picker_row_count(picker) as f32;
        let max_scroll = (rows * row_h - h * 0.7).max(0.0);
        self.picker_scroll =
            (self.picker_scroll - scroll_lines * row_h * 0.85).clamp(0.0, max_scroll);
    }

    fn push_btn(
        frame: &mut UiFrame,
        rect: (f32, f32, f32, f32),
        label: &str,
        click: LabClick,
        font_h: f32,
    ) {
        frame.quad(GpuInstance {
            rect: [rect.0, rect.1, rect.2, rect.3],
            color: color::alpha(color::WALNUT_RAISED, 0.92),
            user: 0,
        });
        frame.text(TextLabel {
            rect: [rect.0, rect.1, rect.2, rect.3],
            text: label.into(),
            color: color::PARCHMENT,
            font_px: Some(typography::tier_at_most(font_h * 0.52, rect.3)),
            align: TextAlign::Center,
            ..Default::default()
        });
        frame.buttons.push(ButtonDef::scene(rect, click.id()));
    }

    fn draw_collapsed_tab(&self, frame: &mut UiFrame, w: f32, h: f32) {
        let (panel_x, panel_y, panel_w, panel_h) = Self::panel_layout(w, h, true);
        frame.quad(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: color::alpha(color::WALNUT_INK, 0.88),
            user: 0,
        });
        frame.text(TextLabel {
            rect: [panel_x, panel_y, panel_w, panel_h],
            text: "▲ Cascade Lab".into(),
            color: color::JADE,
            font_px: Some(typography::tier_at_most(panel_h * 0.55, h)),
            align: TextAlign::Center,
            ..Default::default()
        });
        frame.buttons.push(ButtonDef::scene(
            (panel_x, panel_y, panel_w, panel_h),
            LabClick::TogglePanel.id(),
        ));
    }

    fn draw_expanded_panel(&self, frame: &mut UiFrame, w: f32, h: f32) {
        let scale = metrics::scene_scale(w, h);
        let (panel_x, panel_y, panel_w, panel_h) = Self::panel_layout(w, h, false);
        let pad = (8.0 * scale).max(5.0);
        let row_h = (24.0 * scale).max(18.0);
        let tab_h = (26.0 * scale).max(20.0);
        let row_font = typography::tier_at_most(row_h * 0.48, h);

        frame.quad(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: color::alpha(color::WALNUT_INK, 0.92),
            user: 0,
        });

        let title_y = panel_y + pad;
        frame.text(TextLabel {
            rect: [panel_x + pad, title_y, panel_w * 0.6, tab_h],
            text: "Cascade Lab".into(),
            color: color::JADE,
            font_px: Some(typography::size(typography::H28, h)),
            align: TextAlign::Left,
            ..Default::default()
        });
        Self::push_btn(
            frame,
            (
                panel_x + panel_w - pad - (52.0 * scale),
                title_y,
                52.0 * scale,
                tab_h,
            ),
            "▼",
            LabClick::TogglePanel,
            tab_h,
        );

        let tabs_y = title_y + tab_h + pad * 0.5;
        let tab_w = (panel_w - pad * 2.0 - 8.0) / 3.0;
        for (i, tab) in LabTab::ALL.iter().enumerate() {
            let tx = panel_x + pad + i as f32 * (tab_w + 4.0);
            let active = self.active_tab == *tab;
            frame.quad(GpuInstance {
                rect: [tx, tabs_y, tab_w, tab_h],
                color: if active {
                    color::alpha(color::WALNUT_SOFT, 0.95)
                } else {
                    color::alpha(color::WALNUT_DEEP, 0.85)
                },
                user: 0,
            });
            frame.text(TextLabel {
                rect: [tx, tabs_y, tab_w, tab_h],
                text: tab.label().into(),
                color: if active {
                    color::PARCHMENT
                } else {
                    color::alpha(color::STONE, 0.9)
                },
                font_px: Some(row_font),
                align: TextAlign::Center,
                ..Default::default()
            });
            let tab_click = match tab {
                LabTab::Timing => LabClick::TabTiming,
                LabTab::Table => LabClick::TabTable,
                LabTab::RunState => LabClick::TabState,
            };
            frame
                .buttons
                .push(ButtonDef::scene((tx, tabs_y, tab_w, tab_h), tab_click.id()));
        }

        let body_y = Self::expanded_panel_body_y(panel_y, scale);
        let body_h = panel_y + panel_h - body_y - pad;
        match self.active_tab {
            LabTab::Timing => {
                self.draw_timing_tab(frame, panel_x, body_y, panel_w, body_h, h, scale)
            }
            LabTab::Table => {
                self.draw_table_tab(frame, panel_x, body_y, panel_w, body_h, h, scale, row_h)
            }
            LabTab::RunState => {}
        }
    }

    fn draw_timing_tab(
        &self,
        frame: &mut UiFrame,
        panel_x: f32,
        body_y: f32,
        panel_w: f32,
        _body_h: f32,
        h: f32,
        scale: f32,
    ) {
        let (rows_y0, row_h, row_gap, label_w, slider_w) =
            Self::timing_tab_layout(panel_x, body_y, panel_w, scale);
        let value_w = (panel_w - label_w - slider_w - 16.0 * scale).max(28.0);
        let row_font = typography::tier_at_most(row_h * 0.48, h);

        for (i, (name, min, max, _)) in TUNING_ROW_META.iter().enumerate() {
            let row_y = rows_y0 + i as f32 * (row_h + row_gap);
            let v = self.tuning_row_value(i);
            let t = ((v - *min as f32) / (*max - *min).max(1) as f32).clamp(0.0, 1.0);
            let track_x = panel_x + label_w;
            let track_h = (4.0 * scale).max(3.0);
            let track_y = row_y + (row_h - track_h) * 0.5;
            frame.text(TextLabel {
                rect: [panel_x + 6.0, row_y, label_w, row_h],
                text: (*name).into(),
                font_px: Some(row_font),
                color: color::alpha(color::STONE, 0.95),
                align: TextAlign::Left,
                ..Default::default()
            });
            frame.quad(GpuInstance {
                rect: [track_x, track_y, slider_w, track_h],
                color: color::WALNUT_DEEP,
                user: 0,
            });
            frame.quad(GpuInstance {
                rect: [track_x, track_y, slider_w * t, track_h],
                color: color::alpha(color::JADE, 0.72),
                user: 0,
            });
            let value_text = if i == 6 {
                format!("{:.0}%", v)
            } else {
                format!("{:.0}", v)
            };
            frame.text(TextLabel {
                rect: [track_x + slider_w + 4.0, row_y, value_w, row_h],
                text: value_text,
                font_px: Some(row_font),
                color: color::alpha(color::STONE, 0.95),
                align: TextAlign::Right,
                ..Default::default()
            });
        }
    }

    fn draw_table_tab(
        &self,
        frame: &mut UiFrame,
        panel_x: f32,
        body_y: f32,
        panel_w: f32,
        _body_h: f32,
        h: f32,
        scale: f32,
        row_h: f32,
    ) {
        let gap = (4.0 * scale).max(3.0);
        let mut y = body_y;
        let preset_label = format!("Preset: {}", self.preset().label());
        frame.text(TextLabel {
            rect: [panel_x + 8.0, y, panel_w - 16.0, row_h],
            text: preset_label,
            color: color::PARCHMENT,
            font_px: Some(typography::tier_at_most(row_h * 0.52, h)),
            align: TextAlign::Left,
            ..Default::default()
        });
        y += row_h + gap;
        frame.text(TextLabel {
            rect: [panel_x + 8.0, y, panel_w - 16.0, row_h * 0.7],
            text: "Click relics, dora, wind, boss, or structure melds on the table.".into(),
            color: color::alpha(color::STONE, 0.85),
            font_px: Some(typography::tier_at_most(row_h * 0.38, h)),
            align: TextAlign::Left,
            ..Default::default()
        });
        y += row_h * 0.75 + gap;
        let field_h = row_h * 1.05;
        let focused = self.structure_field_focused;
        frame.quad(GpuInstance {
            rect: [panel_x + 8.0, y, panel_w - 16.0, field_h],
            color: if focused {
                color::alpha(color::WALNUT_SOFT, 0.95)
            } else {
                color::alpha(color::WALNUT_DEEP, 0.88)
            },
            user: 0,
        });
        let display = if self.structure_text.is_empty() && !focused {
            STRUCTURE_NOTATION_HINT.to_string()
        } else {
            let mut s = self.structure_text.clone();
            if focused {
                s.push('|');
            }
            s
        };
        frame.text(TextLabel {
            rect: [panel_x + 12.0, y + 2.0, panel_w - 24.0, field_h - 4.0],
            text: display,
            color: if self.structure_text.is_empty() && !focused {
                color::alpha(color::STONE, 0.65)
            } else {
                color::PARCHMENT
            },
            font_px: Some(typography::tier_at_most(field_h * 0.42, h)),
            align: TextAlign::Left,
            ..Default::default()
        });
        frame.buttons.push(ButtonDef::scene(
            (panel_x + 8.0, y, panel_w - 16.0, field_h),
            LabClick::StructureField.id(),
        ));
        y += field_h + gap * 0.5;
        if let Some(err) = &self.structure_error {
            frame.text(TextLabel {
                rect: [panel_x + 8.0, y, panel_w - 16.0, row_h * 0.75],
                text: err.clone(),
                color: color::alpha(color::RUBY, 0.95),
                font_px: Some(typography::tier_at_most(row_h * 0.38, h)),
                align: TextAlign::Left,
                ..Default::default()
            });
            y += row_h * 0.8 + gap;
        }
        let half_w = (panel_w - 20.0 * scale) * 0.5;
        Self::push_btn(
            frame,
            (panel_x + 8.0, y, half_w, row_h),
            "◀ Prev",
            LabClick::Prev,
            row_h,
        );
        Self::push_btn(
            frame,
            (panel_x + 12.0 + half_w, y, half_w, row_h),
            "Next ▶",
            LabClick::Next,
            row_h,
        );
        y += row_h + gap;
        for (label, id) in [
            ("Apply structure", LabClick::Apply),
            ("Cash in", LabClick::CashIn),
            ("Reset score", LabClick::ResetScore),
            ("Save tuning", LabClick::Save),
            ("Reset tuning", LabClick::ResetTuning),
            ("Back", LabClick::Back),
        ] {
            Self::push_btn(
                frame,
                (panel_x + 8.0, y, panel_w - 16.0, row_h),
                label,
                id,
                row_h,
            );
            y += row_h + gap;
        }
    }

    fn draw_relic_slot_targets(
        &self,
        frame: &mut UiFrame,
        w: f32,
        h: f32,
        env_h: f32,
        proj: &crate::render::wgpu_renderer::ProjectionCache,
        cam: Option<&CameraParams>,
    ) {
        for i in 0..RELIC_SLOT_COUNT {
            let Some([rx, ry, rw, rh]) = Self::relic_slot_hit_rect(w, h, env_h, i, proj, cam)
            else {
                continue;
            };
            let picking = self.active_picker == Some(LabPicker::Relic(i));
            frame.quad(GpuInstance {
                rect: [rx, ry, rw, rh],
                color: if picking {
                    color::alpha(color::JADE, 0.35)
                } else {
                    color::alpha(color::JADE, 0.12)
                },
                user: 0,
            });
            frame.buttons.push(ButtonDef::scene(
                (rx, ry, rw, rh),
                LabClick::RelicTraySlot(i).id(),
            ));
        }
    }

    fn draw_table_pick_targets(
        &self,
        frame: &mut UiFrame,
        w: f32,
        h: f32,
        env_h: f32,
        layout: &crate::ui::layout::LayoutResult,
        proj: &crate::render::wgpu_renderer::ProjectionCache,
        run: &crate::game::run::RunState,
    ) {
        let scale = metrics::scene_scale(w, h);
        let pad = (6.0 * scale).max(4.0);
        let highlight = |frame: &mut UiFrame, rect: [f32; 4], picking: bool, id: u32| {
            let [rx, ry, rw, rh] = rect;
            frame.quad(GpuInstance {
                rect: [rx - pad, ry - pad, rw + pad * 2.0, rh + pad * 2.0],
                color: if picking {
                    color::alpha(color::JADE, 0.35)
                } else {
                    color::alpha(color::JADE, 0.12)
                },
                user: 0,
            });
            frame.buttons.push(ButtonDef::scene(
                (rx - pad, ry - pad, rw + pad * 2.0, rh + pad * 2.0),
                id,
            ));
        };

        let plinth = gameplay_glb::with_gameplay_glb_cpu(|cpu| {
            let cpu = cpu?;
            let mut poses = Vec::new();
            for name in TILE_PLINTH_MARKERS {
                if let Ok(pose) = gameplay_glb::require_gameplay_marker_pose(w, h, env_h, cpu, name)
                {
                    poses.push(pose);
                }
            }
            (!poses.is_empty()).then_some(poses)
        });
        if let Some(plinth) = plinth {
            let dora_count = Self::dora_slot_count(run);
            let dora_rect =
                plinth_hit_rect(proj.dora_tile_rect, &plinth[0].anchor, layout, dora_count);
            for i in 0..dora_count {
                let picking = self.active_picker == Some(LabPicker::Dora(i));
                highlight(
                    frame,
                    split_rect_slots(dora_rect, i, dora_count),
                    picking,
                    LabClick::DoraSlot(i).id(),
                );
            }
            let wind_count = Self::round_wind_slot_count(run);
            let wind_rect = plinth_hit_rect(
                proj.round_wind_tile_rect,
                &plinth[1].anchor,
                layout,
                wind_count,
            );
            for i in 0..wind_count {
                let picking = self.active_picker == Some(LabPicker::RoundWind(i));
                highlight(
                    frame,
                    split_rect_slots(wind_rect, i, wind_count),
                    picking,
                    LabClick::RoundWindSlot(i).id(),
                );
            }
            if plinth.len() >= 3 {
                let boss_rect = plinth_hit_rect(None, &plinth[2].anchor, layout, 1);
                let picking = self.active_picker == Some(LabPicker::Boss);
                highlight(frame, boss_rect, picking, LabClick::Boss.id());
            }
        }

        let meld_h = (36.0 * scale).max(28.0);
        if let Some(strip) = structure_strip_rect(w, h, env_h, meld_h) {
            let sets = run.structure_sets();
            let mut weights: Vec<usize> = sets.iter().map(|s| s.tile_ids.len().max(1)).collect();
            let show_add = sets.len() < STRUCTURE_MELD_SLOTS_MAX;
            if show_add {
                weights.push(2);
            }
            for (i, _) in sets.iter().enumerate() {
                let picking = self.active_picker == Some(LabPicker::StructureMeld(i));
                highlight(
                    frame,
                    split_rect_by_weights(strip, &weights, i),
                    picking,
                    LabClick::StructureMeldSlot(i).id(),
                );
            }
            if show_add {
                let picking = self.active_picker == Some(LabPicker::AddStructureMeld);
                highlight(
                    frame,
                    split_rect_by_weights(strip, &weights, sets.len()),
                    picking,
                    LabClick::StructureAdd.id(),
                );
            }
        }
    }

    fn draw_picker(
        &self,
        frame: &mut UiFrame,
        w: f32,
        h: f32,
        picker: LabPicker,
        run: &crate::game::run::RunState,
    ) {
        let scale = metrics::scene_scale(w, h);
        let picker_w = (300.0 * scale).min(w * 0.34);
        let picker_x = w - picker_w - (8.0 * scale);
        let picker_y = (8.0 * scale).max(6.0);
        let picker_h = h - picker_y * 2.0;

        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::alpha(color::WALNUT_INK, 0.35),
            user: 0,
        });
        frame.quad(GpuInstance {
            rect: [picker_x, picker_y, picker_w, picker_h],
            color: color::alpha(color::WALNUT_INK, 0.96),
            user: 0,
        });

        let pad = (8.0 * scale).max(5.0);
        let header_h = (28.0 * scale).max(22.0);
        frame.text(TextLabel {
            rect: [
                picker_x + pad,
                picker_y + pad,
                picker_w - pad * 2.0,
                header_h,
            ],
            text: picker.title(run),
            color: color::JADE,
            font_px: Some(typography::tier_at_most(header_h * 0.52, h)),
            align: TextAlign::Left,
            ..Default::default()
        });
        Self::push_btn(
            frame,
            (
                picker_x + picker_w - pad - header_h,
                picker_y + pad,
                header_h,
                header_h,
            ),
            "✕",
            LabClick::PickerClose,
            header_h,
        );

        let row_h = (52.0 * scale).max(40.0);
        let mut list_y0 = picker_y + pad + header_h + pad * 0.5;
        if let Some(clear) = picker.clear_label() {
            Self::push_btn(
                frame,
                (picker_x + pad, list_y0, picker_w - pad * 2.0, row_h * 0.55),
                clear,
                LabClick::PickerClear,
                row_h,
            );
            list_y0 += row_h * 0.65 + pad;
        }
        let list_h = picker_y + picker_h - list_y0 - pad;
        frame.quad(GpuInstance {
            rect: [picker_x + pad, list_y0, picker_w - pad * 2.0, list_h],
            color: color::alpha(color::WALNUT_DEEP, 0.55),
            user: 0,
        });

        let first_visible = (self.picker_scroll / row_h).floor() as usize;
        let visible_rows = (list_h / row_h).ceil() as usize + 1;
        let row_font = typography::tier_at_most(row_h * 0.38, h);

        match picker {
            LabPicker::Relic(_) => {
                let icon = row_h * 0.72;
                let defs = all_relic_defs();
                for (vis, &def_idx) in self
                    .sorted_relic_indices
                    .iter()
                    .skip(first_visible)
                    .take(visible_rows)
                    .enumerate()
                {
                    let def = &defs[def_idx];
                    let row_y = list_y0 + vis as f32 * row_h - (self.picker_scroll % row_h);
                    if row_y + row_h < list_y0 || row_y > list_y0 + list_h {
                        continue;
                    }
                    frame.quad(GpuInstance {
                        rect: [
                            picker_x + pad + 2.0,
                            row_y,
                            picker_w - pad * 2.0 - 4.0,
                            row_h - 2.0,
                        ],
                        color: color::alpha(color::WALNUT_RAISED, 0.82),
                        user: 0,
                    });
                    frame.image_quads([ImageQuad {
                        inst: GpuInstance {
                            rect: [
                                picker_x + pad + 6.0,
                                row_y + (row_h - icon) * 0.5,
                                icon,
                                icon,
                            ],
                            color: [1.0, 1.0, 1.0, 1.0],
                            user: 0,
                        },
                        source: ImageQuadSource::Relic(def.id),
                        clip_rect: None,
                    }]);
                    frame.text(TextLabel {
                        rect: [
                            picker_x + pad + icon + 12.0,
                            row_y,
                            picker_w - icon - pad * 3.0,
                            row_h,
                        ],
                        text: def.name.into(),
                        color: color::PARCHMENT,
                        font_px: Some(row_font),
                        align: TextAlign::Left,
                        ..Default::default()
                    });
                    frame.buttons.push(ButtonDef::scene(
                        (
                            picker_x + pad + 2.0,
                            row_y,
                            picker_w - pad * 2.0 - 4.0,
                            row_h - 2.0,
                        ),
                        LabClick::PickRelicDef(def_idx).id(),
                    ));
                }
            }
            LabPicker::Dora(_) => {
                let faces = dora_picker_faces();
                for (vis, (label, _, _)) in faces
                    .iter()
                    .skip(first_visible)
                    .take(visible_rows)
                    .enumerate()
                {
                    let row_y = list_y0 + vis as f32 * row_h - (self.picker_scroll % row_h);
                    if row_y + row_h < list_y0 || row_y > list_y0 + list_h {
                        continue;
                    }
                    frame.quad(GpuInstance {
                        rect: [
                            picker_x + pad + 2.0,
                            row_y,
                            picker_w - pad * 2.0 - 4.0,
                            row_h - 2.0,
                        ],
                        color: color::alpha(color::WALNUT_RAISED, 0.82),
                        user: 0,
                    });
                    frame.text(TextLabel {
                        rect: [picker_x + pad + 12.0, row_y, picker_w - pad * 3.0, row_h],
                        text: label.clone(),
                        color: color::PARCHMENT,
                        font_px: Some(row_font),
                        align: TextAlign::Left,
                        ..Default::default()
                    });
                    frame.buttons.push(ButtonDef::scene(
                        (
                            picker_x + pad + 2.0,
                            row_y,
                            picker_w - pad * 2.0 - 4.0,
                            row_h - 2.0,
                        ),
                        LabClick::PickDoraRow(first_visible + vis).id(),
                    ));
                }
            }
            LabPicker::RoundWind(_) => {
                for vis in 0..visible_rows {
                    let rank = first_visible as u8 + vis as u8 + 1;
                    if rank > 4 {
                        continue;
                    }
                    let row_y = list_y0 + vis as f32 * row_h - (self.picker_scroll % row_h);
                    let label = ChamberKind::wind_name(rank);
                    frame.quad(GpuInstance {
                        rect: [
                            picker_x + pad + 2.0,
                            row_y,
                            picker_w - pad * 2.0 - 4.0,
                            row_h - 2.0,
                        ],
                        color: color::alpha(color::WALNUT_RAISED, 0.82),
                        user: 0,
                    });
                    frame.text(TextLabel {
                        rect: [picker_x + pad + 12.0, row_y, picker_w - pad * 3.0, row_h],
                        text: label.into(),
                        color: color::PARCHMENT,
                        font_px: Some(row_font),
                        align: TextAlign::Left,
                        ..Default::default()
                    });
                    frame.buttons.push(ButtonDef::scene(
                        (
                            picker_x + pad + 2.0,
                            row_y,
                            picker_w - pad * 2.0 - 4.0,
                            row_h - 2.0,
                        ),
                        LabClick::PickWind(rank).id(),
                    ));
                }
            }
            LabPicker::Boss => {
                let icon = row_h * 0.72;
                for (vis, &kind_idx) in self
                    .sorted_ordeal_indices
                    .iter()
                    .skip(first_visible)
                    .take(visible_rows)
                    .enumerate()
                {
                    let kind = OrdealKind::ALL[kind_idx];
                    let row_y = list_y0 + vis as f32 * row_h - (self.picker_scroll % row_h);
                    if row_y + row_h < list_y0 || row_y > list_y0 + list_h {
                        continue;
                    }
                    frame.quad(GpuInstance {
                        rect: [
                            picker_x + pad + 2.0,
                            row_y,
                            picker_w - pad * 2.0 - 4.0,
                            row_h - 2.0,
                        ],
                        color: color::alpha(color::WALNUT_RAISED, 0.82),
                        user: 0,
                    });
                    frame.image_quads([ImageQuad {
                        inst: GpuInstance {
                            rect: [
                                picker_x + pad + 6.0,
                                row_y + (row_h - icon) * 0.5,
                                icon,
                                icon,
                            ],
                            color: [1.0, 1.0, 1.0, 1.0],
                            user: 0,
                        },
                        source: ordeal_icon_source(kind),
                        clip_rect: None,
                    }]);
                    frame.text(TextLabel {
                        rect: [
                            picker_x + pad + icon + 12.0,
                            row_y,
                            picker_w - icon - pad * 3.0,
                            row_h,
                        ],
                        text: kind.name().into(),
                        color: color::PARCHMENT,
                        font_px: Some(row_font),
                        align: TextAlign::Left,
                        ..Default::default()
                    });
                    frame.buttons.push(ButtonDef::scene(
                        (
                            picker_x + pad + 2.0,
                            row_y,
                            picker_w - pad * 2.0 - 4.0,
                            row_h - 2.0,
                        ),
                        LabClick::PickBossRow(first_visible + vis).id(),
                    ));
                }
            }
            LabPicker::StructureMeld(_) | LabPicker::AddStructureMeld => {
                for (vis, token) in MELD_PRESETS
                    .iter()
                    .skip(first_visible)
                    .take(visible_rows)
                    .enumerate()
                {
                    let row_y = list_y0 + vis as f32 * row_h - (self.picker_scroll % row_h);
                    if row_y + row_h < list_y0 || row_y > list_y0 + list_h {
                        continue;
                    }
                    frame.quad(GpuInstance {
                        rect: [
                            picker_x + pad + 2.0,
                            row_y,
                            picker_w - pad * 2.0 - 4.0,
                            row_h - 2.0,
                        ],
                        color: color::alpha(color::WALNUT_RAISED, 0.82),
                        user: 0,
                    });
                    frame.text(TextLabel {
                        rect: [picker_x + pad + 12.0, row_y, picker_w - pad * 3.0, row_h],
                        text: (*token).into(),
                        color: color::PARCHMENT,
                        font_px: Some(row_font),
                        align: TextAlign::Left,
                        ..Default::default()
                    });
                    frame.buttons.push(ButtonDef::scene(
                        (
                            picker_x + pad + 2.0,
                            row_y,
                            picker_w - pad * 2.0 - 4.0,
                            row_h - 2.0,
                        ),
                        LabClick::PickMeldRow(first_visible + vis).id(),
                    ));
                }
            }
        }
    }

    fn draw_run_state_values(
        &self,
        frame: &mut UiFrame,
        panel_x: f32,
        body_y: f32,
        panel_w: f32,
        h: f32,
        scale: f32,
        row_h: f32,
        run: &crate::game::run::RunState,
    ) {
        if self.active_tab != LabTab::RunState || self.panel_collapsed {
            return;
        }
        let gap = (4.0 * scale).max(3.0);
        let mut y = body_y;
        let hint = "Tweak round state for relic hooks (Momentum, Chain Reaction, etc.).";
        frame.text(TextLabel {
            rect: [panel_x + 8.0, y, panel_w - 16.0, row_h * 0.9],
            text: hint.into(),
            color: color::alpha(color::STONE, 0.85),
            font_px: Some(typography::tier_at_most(row_h * 0.42, h)),
            align: TextAlign::Left,
            ..Default::default()
        });
        y += row_h + gap;

        let draw_stepper = |frame: &mut UiFrame,
                            y: f32,
                            label: &str,
                            value: &str,
                            dec: LabClick,
                            inc: LabClick| {
            frame.text(TextLabel {
                rect: [panel_x + 8.0, y, panel_w * 0.42, row_h],
                text: label.into(),
                color: color::PARCHMENT,
                font_px: Some(typography::tier_at_most(row_h * 0.48, h)),
                align: TextAlign::Left,
                ..Default::default()
            });
            let bx = panel_x + panel_w - 8.0 - row_h * 2.6;
            Self::push_btn(frame, (bx, y, row_h, row_h), "−", dec, row_h);
            frame.text(TextLabel {
                rect: [bx + row_h + 2.0, y, row_h * 1.4, row_h],
                text: value.into(),
                color: color::PARCHMENT,
                font_px: Some(typography::tier_at_most(row_h * 0.48, h)),
                align: TextAlign::Center,
                ..Default::default()
            });
            Self::push_btn(frame, (bx + row_h * 2.5, y, row_h, row_h), "+", inc, row_h);
        };

        draw_stepper(
            frame,
            y,
            "Discards left",
            &run.discards_remaining.to_string(),
            LabClick::DiscardsDec,
            LabClick::DiscardsInc,
        );
        y += row_h + gap;
        draw_stepper(
            frame,
            y,
            "Plays left",
            &run.plays_remaining.to_string(),
            LabClick::PlaysDec,
            LabClick::PlaysInc,
        );
        y += row_h + gap;
        draw_stepper(
            frame,
            y,
            "Yen",
            &run.yen.to_string(),
            LabClick::YenDec,
            LabClick::YenInc,
        );
        y += row_h + gap;

        let scored = if run.scored_last_turn { "On" } else { "Off" };
        Self::push_btn(
            frame,
            (panel_x + 8.0, y, panel_w - 16.0, row_h),
            &format!("Scored last turn: {scored}"),
            LabClick::ToggleScoredLast,
            row_h,
        );
        y += row_h + gap;

        let slot = self.counter_edit_slot;
        if let Some(rid) = Self::counter_relic_for_slot(run, slot) {
            let counter = run.relic_counters.get(&rid).copied().unwrap_or(0);
            let name = all_relic_defs()
                .iter()
                .find(|d| d.id == rid)
                .map(|d| d.name)
                .unwrap_or("Relic");
            draw_stepper(
                frame,
                y,
                &format!("{name} counter"),
                &counter.to_string(),
                LabClick::CounterDec,
                LabClick::CounterInc,
            );
        }
    }
}

impl SceneBehavior for CascadeLabScene {
    fn has_blocking_overlay(&self) -> bool {
        true
    }

    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        if !self.prepared {
            self.prepare_run(ctx.run);
        }
        if self.structure_submit_pending {
            self.structure_submit_pending = false;
            self.apply_structure_from_text(ctx.run);
        }

        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        self.handle_picker_scroll(ctx.scroll_lines, w, h);
        self.update_tuning_drag(&ctx, w, h);
        self.prev_mouse_down = ctx.mouse_left_down;

        for &cid in ctx.button_clicks {
            let Some(click) = LabClick::from_id(cid) else {
                continue;
            };
            if self.structure_field_focused
                && click != LabClick::StructureField
                && !click.is_picker()
            {
                self.structure_field_focused = false;
            }
            match click {
                LabClick::TogglePanel => self.panel_collapsed = !self.panel_collapsed,
                LabClick::StructureField => {
                    self.structure_field_focused = true;
                    self.structure_error = None;
                }
                LabClick::PickerClose => self.active_picker = None,
                LabClick::PickerClear => {
                    match self.active_picker {
                        Some(LabPicker::Relic(slot)) => Self::clear_relic_slot(ctx.run, slot),
                        Some(LabPicker::StructureMeld(i)) => {
                            Self::clear_structure_meld(ctx.run, i);
                            self.sync_structure_text_from_run(ctx.run);
                        }
                        Some(LabPicker::Boss) => Self::clear_lab_boss(ctx.run),
                        _ => {}
                    }
                    self.active_picker = None;
                }
                LabClick::PickRelicDef(def_idx) => {
                    if let Some(LabPicker::Relic(slot)) = self.active_picker {
                        if let Some(def) = all_relic_defs().get(def_idx) {
                            Self::assign_relic_slot(ctx.run, slot, def.id);
                            self.active_picker = None;
                        }
                    }
                }
                LabClick::PickDoraRow(pick_i) => {
                    if let Some(LabPicker::Dora(i)) = self.active_picker {
                        if let Some((_, suit, rank)) = dora_picker_faces().get(pick_i) {
                            Self::set_dora_slot(ctx.run, i, *suit, *rank);
                            self.active_picker = None;
                        }
                    }
                }
                LabClick::PickWind(rank) => {
                    if let Some(LabPicker::RoundWind(i)) = self.active_picker {
                        Self::set_round_wind_slot(ctx.run, i, rank);
                        self.active_picker = None;
                    }
                }
                LabClick::PickBossRow(pick_i) => {
                    if self.active_picker == Some(LabPicker::Boss) {
                        if let Some(&kind_idx) = self.sorted_ordeal_indices.get(pick_i) {
                            Self::set_lab_boss(ctx.run, OrdealKind::ALL[kind_idx]);
                            self.active_picker = None;
                        }
                    }
                }
                LabClick::PickMeldRow(pick_i) => {
                    if let Some(token) = MELD_PRESETS.get(pick_i) {
                        match self.active_picker {
                            Some(LabPicker::StructureMeld(i)) => {
                                if Self::apply_meld_preset(ctx.run, Some(i), token) {
                                    self.sync_structure_text_from_run(ctx.run);
                                }
                            }
                            Some(LabPicker::AddStructureMeld) => {
                                if Self::apply_meld_preset(ctx.run, None, token) {
                                    self.sync_structure_text_from_run(ctx.run);
                                }
                            }
                            _ => {}
                        }
                        self.active_picker = None;
                    }
                }
                LabClick::RelicTraySlot(slot) if slot < RELIC_SLOT_COUNT => {
                    self.counter_edit_slot = slot;
                    self.active_picker = Some(LabPicker::Relic(slot));
                    self.picker_scroll = 0.0;
                }
                LabClick::DoraSlot(i) if i < DORA_SLOT_MAX => {
                    self.active_picker = Some(LabPicker::Dora(i));
                    self.picker_scroll = 0.0;
                }
                LabClick::RoundWindSlot(i) if i < ROUND_WIND_SLOT_MAX => {
                    self.active_picker = Some(LabPicker::RoundWind(i));
                    self.picker_scroll = 0.0;
                }
                LabClick::StructureMeldSlot(i) => {
                    if i < STRUCTURE_MELD_SLOTS_MAX && i < ctx.run.structure_sets().len() {
                        self.active_picker = Some(LabPicker::StructureMeld(i));
                        self.picker_scroll = 0.0;
                    }
                }
                LabClick::StructureAdd => {
                    if ctx.run.structure_sets().len() < STRUCTURE_MELD_SLOTS_MAX {
                        self.active_picker = Some(LabPicker::AddStructureMeld);
                        self.picker_scroll = 0.0;
                    }
                }
                LabClick::Boss => {
                    self.active_picker = Some(LabPicker::Boss);
                    self.picker_scroll = 0.0;
                }
                _ if self.active_picker.is_some() => {}
                LabClick::Back => return self.go_back(ctx.run, ctx.overlay_request),
                LabClick::TabTiming => self.active_tab = LabTab::Timing,
                LabClick::TabTable => self.active_tab = LabTab::Table,
                LabClick::TabState => self.active_tab = LabTab::RunState,
                LabClick::Prev => {
                    self.preset_idx = (self.preset_idx + StructurePreset::ALL.len() - 1)
                        % StructurePreset::ALL.len();
                    self.apply_preset(ctx.run);
                    self.sync_structure_text_from_preset();
                }
                LabClick::Next => {
                    self.preset_idx = (self.preset_idx + 1) % StructurePreset::ALL.len();
                    self.apply_preset(ctx.run);
                    self.sync_structure_text_from_preset();
                }
                LabClick::Apply => {
                    let _ = self.apply_structure_from_text(ctx.run);
                }
                LabClick::CashIn => {
                    self.cash_in(&mut ctx);
                }
                LabClick::ResetScore => self.reset_score(ctx.run),
                LabClick::Save => {
                    if let Err(e) = std::fs::write(
                        "cascade_tuning.json",
                        serde_json::to_string_pretty(&self.tuning).unwrap_or_default(),
                    ) {
                        log::warn!("Failed to save cascade tuning: {e}");
                    }
                }
                LabClick::ResetTuning => self.tuning = CascadeTuning::default(),
                LabClick::DiscardsDec => {
                    ctx.run.discards_remaining = ctx.run.discards_remaining.saturating_sub(1);
                }
                LabClick::DiscardsInc => {
                    ctx.run.discards_remaining =
                        (ctx.run.discards_remaining + 1).min(ctx.run.discards_max.max(12));
                }
                LabClick::PlaysDec => {
                    ctx.run.plays_remaining = ctx.run.plays_remaining.saturating_sub(1);
                }
                LabClick::PlaysInc => {
                    ctx.run.plays_remaining =
                        (ctx.run.plays_remaining + 1).min(ctx.run.plays_max.max(12));
                }
                LabClick::YenDec => {
                    ctx.run
                        .set_run_yen_direct(ctx.run.yen.saturating_sub(50), None);
                }
                LabClick::YenInc => {
                    ctx.run
                        .set_run_yen_direct(ctx.run.yen.saturating_add(50), None);
                }
                LabClick::ToggleScoredLast => {
                    ctx.run.scored_last_turn = !ctx.run.scored_last_turn;
                }
                LabClick::CounterDec => {
                    let slot = self.counter_edit_slot;
                    if let Some(rid) = Self::counter_relic_for_slot(ctx.run, slot) {
                        let entry = ctx.run.relic_counters.entry(rid).or_insert(0);
                        *entry = entry.saturating_sub(1);
                    }
                }
                LabClick::CounterInc => {
                    let slot = self.counter_edit_slot;
                    if let Some(rid) = Self::counter_relic_for_slot(ctx.run, slot) {
                        let entry = ctx.run.relic_counters.entry(rid).or_insert(0);
                        *entry = entry.saturating_add(1);
                    }
                }
                _ => {}
            }
        }

        for a in ctx.actions {
            if matches!(a, UiAction::Cancel | UiAction::Pause) {
                if self.active_picker.take().is_some() {
                    continue;
                }
                return self.go_back(ctx.run, ctx.overlay_request);
            }
        }

        if !self.gameplay.lab_cascade_active() && ctx.run.structure_sets().is_empty() {
            self.apply_preset(ctx.run);
            self.sync_structure_text_from_preset();
        }

        self.gameplay.update(ctx);
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let env_h = ctx.room_gltf_height_scale;
        let run = ctx.run;
        let proj = ctx.proj;
        let layout = ctx.layout;
        let input_mode = ctx.input_mode;
        let glyphs = ctx.glyphs;
        let mut frame = self.gameplay.draw_frame(ctx);
        let gameplay_button_count = frame.buttons.len();
        frame.window_title = "Mahjuro — Cascade Lab".into();

        let cam = frame.camera_override;
        self.draw_relic_slot_targets(&mut frame, w, h, env_h, proj, cam.as_ref());
        self.draw_table_pick_targets(&mut frame, w, h, env_h, layout, proj, run);

        if self.panel_collapsed {
            self.draw_collapsed_tab(&mut frame, w, h);
        } else {
            self.draw_expanded_panel(&mut frame, w, h);
            let (panel_x, panel_y, panel_w, _panel_h) = Self::panel_layout(w, h, false);
            let row_h = (24.0 * scale).max(18.0);
            let body_y = Self::expanded_panel_body_y(panel_y, scale);
            self.draw_run_state_values(&mut frame, panel_x, body_y, panel_w, h, scale, row_h, run);
        }

        if let Some(picker) = self.active_picker {
            self.draw_picker(&mut frame, w, h, picker, run);
        }

        // Lab buttons must win hit-testing over the embedded gameplay scene's
        // fullscreen 3D pick dispatcher and action tablets.
        let lab_buttons = frame.buttons.split_off(gameplay_button_count);
        frame.buttons = Self::reorder_lab_hit_buttons(
            self.active_picker.is_some(),
            w,
            h,
            lab_buttons,
            frame.buttons,
        );

        push_screen_footer_hint_for(
            &mut frame,
            w,
            h,
            input_mode,
            glyphs,
            back_footer_row(input_mode),
            HintStyle::standard(w, h),
        );

        frame
    }
}

fn structure_strip_rect(w: f32, h: f32, env_h: f32, meld_h: f32) -> Option<[f32; 4]> {
    gameplay_glb::with_gameplay_glb_cpu(|cpu| {
        let cpu = cpu?;
        let left =
            gameplay_glb::require_gameplay_marker_pose(w, h, env_h, cpu, STRUCTURE_TILES_LEFT)
                .ok()?;
        let right =
            gameplay_glb::require_gameplay_marker_pose(w, h, env_h, cpu, STRUCTURE_TILES_RIGHT)
                .ok()?;
        let (x, y, rw, rh) =
            gameplay_glb::marker_pair_screen_rect_from_poses(&left, &right, meld_h);
        Some([x, y, rw, rh])
    })
}

fn dora_picker_faces() -> Vec<(String, Suit, u8)> {
    let mut out = Vec::with_capacity(30);
    for suit in [Suit::Manzu, Suit::Pinzu, Suit::Souzu] {
        let letter = match suit {
            Suit::Manzu => 'm',
            Suit::Pinzu => 'p',
            Suit::Souzu => 's',
            _ => continue,
        };
        for rank in 1..=9 {
            out.push((format!("{rank}{letter}"), suit, rank));
        }
    }
    out.push(("rrr".into(), Suit::Dragon, 1));
    out.push(("ggg".into(), Suit::Dragon, 2));
    out.push(("whwh".into(), Suit::Dragon, 3));
    out
}

fn scancode_to_structure_char(code: sdl3::keyboard::Scancode, shift: bool) -> Option<char> {
    use sdl3::keyboard::Scancode;
    let upper = shift;
    match code {
        Scancode::A => Some(if upper { 'A' } else { 'a' }),
        Scancode::B => Some(if upper { 'B' } else { 'b' }),
        Scancode::C => Some(if upper { 'C' } else { 'c' }),
        Scancode::D => Some(if upper { 'D' } else { 'd' }),
        Scancode::E => Some(if upper { 'E' } else { 'e' }),
        Scancode::F => Some(if upper { 'F' } else { 'f' }),
        Scancode::G => Some(if upper { 'G' } else { 'g' }),
        Scancode::H => Some(if upper { 'H' } else { 'h' }),
        Scancode::I => Some(if upper { 'I' } else { 'i' }),
        Scancode::J => Some(if upper { 'J' } else { 'j' }),
        Scancode::K => Some(if upper { 'K' } else { 'k' }),
        Scancode::L => Some(if upper { 'L' } else { 'l' }),
        Scancode::M => Some(if upper { 'M' } else { 'm' }),
        Scancode::N => Some(if upper { 'N' } else { 'n' }),
        Scancode::O => Some(if upper { 'O' } else { 'o' }),
        Scancode::P => Some(if upper { 'P' } else { 'p' }),
        Scancode::Q => Some(if upper { 'Q' } else { 'q' }),
        Scancode::R => Some(if upper { 'R' } else { 'r' }),
        Scancode::S => Some(if upper { 'S' } else { 's' }),
        Scancode::T => Some(if upper { 'T' } else { 't' }),
        Scancode::U => Some(if upper { 'U' } else { 'u' }),
        Scancode::V => Some(if upper { 'V' } else { 'v' }),
        Scancode::W => Some(if upper { 'W' } else { 'w' }),
        Scancode::X => Some(if upper { 'X' } else { 'x' }),
        Scancode::Y => Some(if upper { 'Y' } else { 'y' }),
        Scancode::Z => Some(if upper { 'Z' } else { 'z' }),
        Scancode::_0 | Scancode::Kp0 => Some('0'),
        Scancode::_1 | Scancode::Kp1 => Some('1'),
        Scancode::_2 | Scancode::Kp2 => Some('2'),
        Scancode::_3 | Scancode::Kp3 => Some('3'),
        Scancode::_4 | Scancode::Kp4 => Some('4'),
        Scancode::_5 | Scancode::Kp5 => Some('5'),
        Scancode::_6 | Scancode::Kp6 => Some('6'),
        Scancode::_7 | Scancode::Kp7 => Some('7'),
        Scancode::_8 | Scancode::Kp8 => Some('8'),
        Scancode::_9 | Scancode::Kp9 => Some('9'),
        _ => None,
    }
}
