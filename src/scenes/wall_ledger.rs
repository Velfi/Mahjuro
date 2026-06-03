//! Wall Ledger — grouped tile supply overlay (live gameplay or shop preview).

use std::collections::HashMap;
use std::time::Instant;

use crate::core::tile::Suit;
use crate::game::event_bus::GameEvent;
use crate::game::wall_ledger::{WallLedgerFaceGroup, WallLedgerMode, read_wall_ledger};
use crate::render::draw_cmd::{CameraParams, ShowcaseTilePlacement, UiFrame};
use crate::render::text_effect::TextEffectId;
use crate::render::theme::{ButtonState, ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::sfx_id::SfxId;
use crate::ui::controller_hints::{
    HintStyle, back_scroll_footer_row, push_inline_hint_rows,
};
use crate::ui::focus_nav;
use crate::ui::input::UiAction;
use crate::ui::widget;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::{
    BackgroundId, DrawCtx, OverlayRequest, SceneBehavior, SceneTransition, UpdateCtx,
};

const DRAWN_BRIGHTNESS: f32 = 0.32;
const DRAWN_SCALE: f32 = 0.92;
/// Breathing room below the flowers row inside the scroll panel.
const GRID_BOTTOM_PAD_FRAC: f32 = 0.028;
/// Show a corner badge (e.g. `×6`) when a face has more copies than the default wall.
const STACK_COUNT_BADGE_MIN: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LedgerNav {
    Back,
}

impl LedgerNav {
    fn id(self) -> FocusId {
        FocusId(0xE200 + self as u32)
    }
}

/// Row definitions for the standard 38-face grid (matches tile-select preview).
const GRID_ROWS: [(usize, usize); 5] = [
    (0, 9),
    (9, 9),
    (18, 9),
    (27, 7),
    (34, 4),
];

const ROW_LABELS: [&str; 5] = ["Manzu", "Souzu", "Pinzu", "Honors", "Flowers"];

pub struct WallLedgerScene {
    mode: WallLedgerMode,
    scroll_px: f32,
    target_scroll_px: f32,
    scroll_last_tick: Instant,
    tree: TreeState,
}

impl WallLedgerScene {
    pub fn live() -> Self {
        Self::with_mode(WallLedgerMode::Live)
    }

    pub fn shop_preview() -> Self {
        Self::with_mode(WallLedgerMode::ShopPreview)
    }

    fn with_mode(mode: WallLedgerMode) -> Self {
        Self {
            mode,
            scroll_px: 0.0,
            target_scroll_px: 0.0,
            scroll_last_tick: Instant::now(),
            tree: TreeState::new(),
        }
    }

    fn go_back(overlay_request: &mut Option<OverlayRequest>) -> SceneTransition {
        *overlay_request = Some(OverlayRequest::Pop);
        None
    }

    fn flat_items(&self, w: f32, h: f32) -> Vec<FlatItem<LedgerNav>> {
        let (back, _) = header_chrome(w, h);
        vec![FlatItem::new(LedgerNav::Back.id(), back, LedgerNav::Back)]
    }

    fn tick_scroll(&mut self) {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.scroll_last_tick)
            .as_secs_f32();
        self.scroll_last_tick = now;
        let t = (dt * 12.0).clamp(0.0, 1.0);
        self.scroll_px += (self.target_scroll_px - self.scroll_px) * t;
    }
}

fn header_chrome(window_w: f32, window_h: f32) -> ([f32; 4], f32) {
    let ui = (window_w / 1920.0)
        .min(window_h / 1080.0)
        .clamp(0.55, 1.35);
    let margin = 48.0 * ui;
    let header_btn_h = (window_h * 0.052).clamp(44.0, 72.0);
    let back_w = (108.0 * (margin / 48.0)).clamp(88.0, 132.0);
    let back = [margin, margin, back_w, header_btn_h];
    let chrome_bottom = margin + header_btn_h + 12.0 * ui;
    (back, chrome_bottom)
}

#[inline]
fn read_boost(window_w: f32, window_h: f32) -> f32 {
    (window_w.min(window_h) / 720.0).clamp(1.0, 1.38)
}

fn grid_slots(
    grid_x: f32,
    grid_y: f32,
    grid_w: f32,
    slot_h: f32,
    row_gap: f32,
) -> Vec<(f32, f32, f32, f32)> {
    let cols = 9.0_f32;
    let slot_w = grid_w / cols;

    let mut slots = Vec::with_capacity(38);
    for (row_idx, &(_start, count)) in GRID_ROWS.iter().enumerate() {
        let row_y = grid_y + row_idx as f32 * (slot_h + row_gap);
        let row_offset = (cols - count as f32) * slot_w * 0.5;
        for col in 0..count {
            let x = grid_x + row_offset + col as f32 * slot_w;
            slots.push((x, row_y, slot_w, slot_h));
        }
    }
    slots
}

struct LedgerLayout {
    content_top: f32,
    content_h: f32,
    grid_x: f32,
    grid_w: f32,
    pack_section_top: f32,
    total_content_h: f32,
    label_col_w: f32,
    slot_h: f32,
    row_gap: f32,
    hints_h: f32,
}

fn grid_row_metrics(
    w: f32,
    h: f32,
    jr: f32,
    content_h: f32,
) -> (f32, f32, f32, f32, f32, f32) {
    let grid_bottom_pad = (h * GRID_BOTTOM_PAD_FRAC).max(18.0 * jr);
    let label_col_w = w * 0.09;
    let grid_x = w * 0.055 + label_col_w;
    let grid_w = w * 0.88 - label_col_w;
    let available_h = (content_h - grid_bottom_pad).max(80.0 * jr);
    let min_gap = (4.0 * jr).max(3.0);
    let natural_slot_h = (grid_w / 9.0) * 1.36;
    let natural_grid_h = GRID_ROWS.len() as f32 * natural_slot_h
        + (GRID_ROWS.len() as f32 - 1.0) * min_gap;

    let (slot_h, row_gap) = if natural_grid_h <= available_h {
        let extra = (available_h - natural_grid_h).max(0.0);
        let gap = min_gap + extra / (GRID_ROWS.len() as f32 - 1.0).max(1.0);
        (natural_slot_h, gap)
    } else {
        let row_h = (available_h - min_gap * (GRID_ROWS.len() as f32 - 1.0))
            / GRID_ROWS.len() as f32;
        (row_h, min_gap)
    };
    let standard_grid_h =
        GRID_ROWS.len() as f32 * slot_h + (GRID_ROWS.len() as f32 - 1.0) * row_gap;
    (
        grid_x,
        grid_w,
        slot_h,
        row_gap,
        standard_grid_h,
        grid_bottom_pad,
    )
}

fn footer_hint_metrics(h: f32, jr: f32) -> (f32, f32) {
    let hint_h = (34.0 * jr).clamp(28.0, 46.0);
    let bottom_margin = (h * 0.028).max(16.0 * jr);
    (hint_h, bottom_margin)
}

fn ledger_layout(w: f32, h: f32, jr: f32, pack_row_count: usize) -> LedgerLayout {
    let (_, chrome_bottom) = header_chrome(w, h);
    let header_block_h = (72.0 * jr).clamp(52.0, 96.0);
    let (hints_h, hints_bottom) = footer_hint_metrics(h, jr);
    let bottom_safe = hints_bottom + hints_h;
    let content_top = chrome_bottom + header_block_h + 8.0 * jr;
    let content_h = (h - bottom_safe - content_top).max(120.0 * jr);

    let label_col_w = w * 0.09;
    let (grid_x, grid_w, slot_h, row_gap, standard_grid_h, grid_bottom_pad) =
        grid_row_metrics(w, h, jr, content_h);
    let pack_row_count = pack_row_count;
    let pack_gap = if pack_row_count > 0 { 12.0 * jr } else { 0.0 };
    let pack_section_top = content_top + standard_grid_h + grid_bottom_pad + pack_gap
        + if pack_row_count > 0 { 22.0 * jr } else { 0.0 };
    let pack_section_h = if pack_row_count > 0 {
        pack_row_count as f32 * (slot_h + row_gap * 0.5) + 8.0 * jr
    } else {
        0.0
    };
    let total_content_h =
        standard_grid_h + grid_bottom_pad + pack_section_h + pack_gap + header_block_h * 0.15;

    LedgerLayout {
        content_top,
        content_h,
        grid_x,
        grid_w,
        pack_section_top,
        total_content_h,
        label_col_w,
        slot_h,
        row_gap,
        hints_h,
    }
}

fn groups_by_face(groups: &[WallLedgerFaceGroup]) -> HashMap<(Suit, u8), &WallLedgerFaceGroup> {
    groups
        .iter()
        .map(|g| ((g.suit, g.rank), g))
        .collect()
}

fn push_cell_tiles(
    placements: &mut Vec<ShowcaseTilePlacement>,
    entries: &[crate::game::wall_ledger::WallTileEntry],
    cell: (f32, f32, f32, f32),
    run: &crate::game::run::RunState,
) {
    let (cx, cy, cw, ch) = cell;
    let n = entries.len();
    if n == 0 {
        return;
    }
    let tile_size = (cw * 0.82).min(ch * 0.88);
    let has_mixed = entries.iter().any(|e| e.drawn) && entries.iter().any(|e| !e.drawn);
    let stack_dx = if has_mixed { cw * 0.065 } else { cw * 0.04 };
    let stack_dy = if has_mixed { ch * 0.05 } else { ch * 0.035 };
    let base_x = cx + (cw - tile_size) * 0.5 - stack_dx * (n.saturating_sub(1) as f32) * 0.5;
    let row_center_y = cy + ch * 0.5;

    let mut ordered: Vec<&crate::game::wall_ledger::WallTileEntry> = entries.iter().collect();
    // Drawn copies first (back), undrawn last (front) so fade reads in mixed stacks.
    ordered.sort_by(|a, b| b.drawn.cmp(&a.drawn));

    for (i, entry) in ordered.iter().enumerate() {
        let tile = crate::game::engine::GameEngine::display_tile(entry.tile, run);
        let drawn = entry.drawn;
        let brightness = if drawn { DRAWN_BRIGHTNESS } else { 1.0 };
        let scale = if drawn { DRAWN_SCALE } else { 1.0 };
        let offset_x = i as f32 * stack_dx;
        let offset_y = i as f32 * stack_dy;
        placements.push(ShowcaseTilePlacement {
            tile,
            center_pos: [base_x + offset_x + tile_size * 0.5, row_center_y + offset_y, 0.0],
            rotation: [0.0, 0.0, std::f32::consts::PI],
            scale,
            size_px: tile_size,
            brightness,
            selected: false,
            hovered: false,
            outline: false,
            glow: false,
            glow_color: None,
            pick_id: None,
            overlay_rect_group: None,
        });
    }
}

fn push_cell_stack_count_badge(
    quads: &mut Vec<GpuInstance>,
    texts: &mut Vec<TextLabel>,
    count: usize,
    cell: (f32, f32, f32, f32),
    window_w: f32,
    window_h: f32,
) {
    if count < STACK_COUNT_BADGE_MIN {
        return;
    }
    let rect = [cell.0, cell.1, cell.2, cell.3];
    let label = format!("×{count}");
    crate::ui::corner_badge::push_corner_badge(quads, texts, rect, window_w, window_h, &label);
}

fn push_wall_ledger_cell(
    placements: &mut Vec<ShowcaseTilePlacement>,
    badge_quads: &mut Vec<GpuInstance>,
    badge_texts: &mut Vec<TextLabel>,
    entries: &[crate::game::wall_ledger::WallTileEntry],
    cell: (f32, f32, f32, f32),
    run: &crate::game::run::RunState,
    window_w: f32,
    window_h: f32,
) {
    push_cell_tiles(placements, entries, cell, run);
    push_cell_stack_count_badge(
        badge_quads,
        badge_texts,
        entries.len(),
        cell,
        window_w,
        window_h,
    );
}

impl SceneBehavior for WallLedgerScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let ledger = read_wall_ledger(ctx.run, self.mode);
        let jr = read_boost(w, h);
        let pack_rows = pack_display_rows(&ledger.pack_groups);
        let layout = ledger_layout(w, h, jr, pack_rows.len());
        let max_scroll = (layout.total_content_h - layout.content_h).max(0.0);

        if ctx.scroll_lines.abs() > 0.001 {
            self.target_scroll_px =
                (self.target_scroll_px + ctx.scroll_lines * layout.slot_h * 0.85)
                    .clamp(0.0, max_scroll);
        }
        self.target_scroll_px = self.target_scroll_px.clamp(0.0, max_scroll);
        self.scroll_px = self.scroll_px.clamp(0.0, max_scroll);

        for a in ctx.actions {
            if matches!(a, UiAction::Cancel | UiAction::Pause | UiAction::Help) {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                return Self::go_back(ctx.overlay_request);
            }
        }

        let items = self.flat_items(w, h);
        let nav_actions: Vec<UiAction> = ctx
            .actions
            .iter()
            .copied()
            .filter(|a| {
                !matches!(
                    a,
                    UiAction::FocusUp
                        | UiAction::FocusDown
                        | UiAction::FocusPrev
                        | UiAction::FocusNext
                )
            })
            .collect();
        let nav_action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: &nav_actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (w, h),
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }
        if matches!(nav_action, Some(LedgerNav::Back)) {
            ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
            return Self::go_back(ctx.overlay_request);
        }

        self.tick_scroll();
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let jr = read_boost(w, h);
        let ledger = read_wall_ledger(ctx.run, self.mode);
        let pack_rows = pack_display_rows(&ledger.pack_groups);
        let layout = ledger_layout(w, h, jr, pack_rows.len());
        let scroll = self.scroll_px;
        let max_scroll = (layout.total_content_h - layout.content_h).max(0.0);
        let _ = max_scroll;

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::WALNUT_INK,
            user: 0,
        });

        let cam_scale = h / 1600.0;
        frame.camera_override = Some(CameraParams {
            eye: [0.0, 0.0, 2040.0 * cam_scale],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            fovy_deg: 45.0,
            clip_near: None,
            clip_far: None,
        });
        // Placement policy: `wall_ledger` is on the pixel-to-world deny list (top-down 1:1 grid).

        frame.scene_lighting.push_smooth(PointLight {
            pos: [w * 0.5, h * -0.10, h * 1.40],
            radius: h * 3.0,
            color: color::rgb(color::PARCHMENT),
            intensity: 1.2,
        });

        push_back_button(&mut frame, &self.tree, w, h);

        let (_, chrome_bottom) = header_chrome(w, h);
        let title_y = chrome_bottom + 4.0 * jr;
        let title_px = typography::size(typography::H24, h) * jr.min(1.15);
        let sub_px = typography::size(typography::H28, h) * jr.min(1.1);
        frame.texts([
            TextLabel {
                rect: [w * 0.055, title_y, w * 0.5, title_px * 1.4],
                text: "Wall Ledger".into(),
                color: color::CHAMPAGNE,
                font_px: Some(title_px),
                align: TextAlign::Left,
                scroll_offset: 0.0,
                flavor_spans: None,
                bold: true,
                italic: false,
                underline: false,
                text_effect: TextEffectId::Flat,
                rotation_quarters: 0,
                baseline_shift_px: 0.0,
                clip_rect: None,
                mono: false,
            },
            TextLabel {
                rect: [
                    w * 0.055,
                    title_y + title_px * 1.35,
                    w * 0.7,
                    sub_px * 1.3,
                ],
                text: ledger.subtitle.clone(),
                color: color::alpha(color::CHAMPAGNE, 0.72),
                font_px: Some(sub_px),
                align: TextAlign::Left,
                scroll_offset: 0.0,
                flavor_spans: None,
                bold: false,
                italic: false,
                underline: false,
                text_effect: TextEffectId::Flat,
                rotation_quarters: 0,
                baseline_shift_px: 0.0,
                clip_rect: None,
                mono: false,
            },
        ]);

        let panel_top = layout.content_top - scroll;
        let panel_rect = [w * 0.04, layout.content_top, w * 0.92, layout.content_h];
        frame.quad(GpuInstance {
            rect: panel_rect,
            color: color::alpha(color::WALNUT_DEEP, 0.92),
            user: 0,
        });

        let clip = panel_rect;
        let standard_by_face = groups_by_face(&ledger.standard_groups);
        let grid_y = panel_top;
        let slots = grid_slots(
            layout.grid_x,
            grid_y,
            layout.grid_w,
            layout.slot_h,
            layout.row_gap,
        );
        let slot_w = layout.grid_w / 9.0;

        let label_px = typography::size(typography::H16, h) * jr.min(1.1);
        for (row_idx, label) in ROW_LABELS.iter().enumerate() {
            let row_y = grid_y + row_idx as f32 * (layout.slot_h + layout.row_gap);
            if row_y + layout.slot_h < clip[1] - 2.0 || row_y > clip[1] + clip[3] + 2.0 {
                continue;
            }
            let accent = row_suit_color(row_idx);
            frame.texts([TextLabel {
                rect: [
                    w * 0.055,
                    row_y + layout.slot_h * 0.5 - label_px * 0.5,
                    layout.label_col_w,
                    label_px * 1.4,
                ],
                text: (*label).into(),
                color: accent,
                font_px: Some(label_px),
                align: TextAlign::Left,
                scroll_offset: 0.0,
                flavor_spans: None,
                bold: false,
                italic: false,
                underline: false,
                text_effect: TextEffectId::Flat,
                rotation_quarters: 0,
                baseline_shift_px: 0.0,
                clip_rect: None,
                mono: false,
            }]);
        }

        let mut placements = Vec::new();
        let mut stack_badges_quads = Vec::new();
        let mut stack_badges_texts = Vec::new();
        for (face_idx, slot) in slots.iter().enumerate() {
            if slot.1 + slot.3 < clip[1] || slot.1 > clip[1] + clip[3] {
                continue;
            }
            let Some((suit, rank)) = face_index_to_face(face_idx) else {
                continue;
            };
            if let Some(group) = standard_by_face.get(&(suit, rank)) {
                push_wall_ledger_cell(
                    &mut placements,
                    &mut stack_badges_quads,
                    &mut stack_badges_texts,
                    &group.copies,
                    *slot,
                    ctx.run,
                    w,
                    h,
                );
            }
        }

        if !ledger.pack_groups.is_empty() {
            let pack_label_y = layout.pack_section_top - scroll;
            if pack_label_y + 20.0 >= clip[1] && pack_label_y <= clip[1] + clip[3] {
                frame.texts([TextLabel {
                    rect: [w * 0.055, pack_label_y, w * 0.4, label_px * 1.4],
                    text: "Packs".into(),
                    color: color::alpha(color::CHAMPAGNE, 0.85),
                    font_px: Some(label_px),
                    align: TextAlign::Left,
                    scroll_offset: 0.0,
                    flavor_spans: None,
                    bold: true,
                    italic: false,
                    underline: false,
                    text_effect: TextEffectId::Flat,
                    rotation_quarters: 0,
                    baseline_shift_px: 0.0,
                    clip_rect: Some(clip),
                    mono: false,
                }]);
            }
            for (row_i, row_groups) in pack_rows.iter().enumerate() {
                let row_y = layout.pack_section_top - scroll
                    + row_i as f32 * (layout.slot_h + layout.row_gap * 0.5)
                    + 18.0 * jr;
                if row_y + layout.slot_h < clip[1] || row_y > clip[1] + clip[3] {
                    continue;
                }
                for (col_i, group) in row_groups.iter().enumerate() {
                    let cell_x = layout.grid_x + col_i as f32 * slot_w;
                    let cell = (cell_x, row_y, slot_w, layout.slot_h);
                    push_wall_ledger_cell(
                        &mut placements,
                        &mut stack_badges_quads,
                        &mut stack_badges_texts,
                        &group.copies,
                        cell,
                        ctx.run,
                        w,
                        h,
                    );
                }
            }
        }

        if !placements.is_empty() {
            frame.showcase_tile_batch(placements);
        }
        if !stack_badges_quads.is_empty() {
            frame.quads(stack_badges_quads);
        }
        if !stack_badges_texts.is_empty() {
            frame.texts(stack_badges_texts);
        }

        let (_, hints_bottom) = footer_hint_metrics(h, jr);
        let hints_y = h - hints_bottom - layout.hints_h;
        let hint_style = HintStyle::standard(h);
        let hint_rect = [0.0, hints_y, w, layout.hints_h];
        let hint_row = back_scroll_footer_row(ctx.input_mode);
        push_inline_hint_rows(&mut frame, &ctx, &[hint_rect], &[hint_row], hint_style);

        frame
    }
}

fn face_index_to_face(idx: usize) -> Option<(Suit, u8)> {
    match idx {
        0..=8 => Some((Suit::Manzu, (idx + 1) as u8)),
        9..=17 => Some((Suit::Souzu, (idx - 8) as u8)),
        18..=26 => Some((Suit::Pinzu, (idx - 17) as u8)),
        27..=30 => Some((Suit::Wind, (idx - 26) as u8)),
        31..=33 => Some((Suit::Dragon, (idx - 30) as u8)),
        34..=37 => Some((Suit::Flower, (idx - 33) as u8)),
        _ => None,
    }
}

fn row_suit_color(row_idx: usize) -> [f32; 4] {
    match row_idx {
        0 => Suit::Manzu.keyword_color(),
        1 => Suit::Souzu.keyword_color(),
        2 => Suit::Pinzu.keyword_color(),
        3 => Suit::Wind.keyword_color(),
        _ => Suit::Flower.keyword_color(),
    }
}

fn pack_display_rows(groups: &[WallLedgerFaceGroup]) -> Vec<Vec<&WallLedgerFaceGroup>> {
    const COLS: usize = 9;
    if groups.is_empty() {
        return Vec::new();
    }
    groups
        .chunks(COLS)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

fn push_back_button(frame: &mut UiFrame, tree: &TreeState, w: f32, h: f32) {
    let scale = (w.min(h)) / 600.0;
    let (back, _) = header_chrome(w, h);
    let focused = tree.focused() == Some(LedgerNav::Back.id());
    let mut nav_quads = Vec::new();
    let mut nav_texts = Vec::new();
    let mut junk_buttons = Vec::new();
    widget::push_button(
        &mut nav_quads,
        &mut nav_texts,
        &mut junk_buttons,
        widget::ButtonSpec {
            rect: back,
            label: "Back",
            variant: ButtonVariant::Default,
            state: if focused {
                ButtonState::Hover
            } else {
                ButtonState::Rest
            },
            action: UiAction::Confirm,
        },
    );
    if focused {
        focus_nav::push_focus_ring(back, scale, w, h, &mut nav_quads);
    }
    frame.quads(nav_quads);
    for label in nav_texts {
        frame.texts([label]);
    }
}
