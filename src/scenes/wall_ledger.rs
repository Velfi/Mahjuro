//! Wall Ledger — grouped tile supply overlay (live gameplay or shop preview).

use std::collections::HashMap;
use std::time::Instant;

use crate::core::tile::Suit;
use crate::game::event_bus::GameEvent;
use crate::game::wall_ledger::{WallLedgerFaceGroup, WallLedgerMode, read_wall_ledger};
use crate::render::draw_cmd::{CameraParams, ShowcaseTilePlacement, UiFrame};
use crate::render::text_effect::TextEffectId;
use crate::render::theme::{ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::sfx_id::SfxId;
use crate::ui::controller_hints::{
    HintStyle, back_scroll_footer_row, push_screen_footer_hint, screen_footer_reserve,
};
use crate::ui::focus_nav;
use crate::ui::input::UiAction;
use crate::ui::widget::{self, PLAIN_TEXT_LINE_STEP_MUL};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::header_chrome::{HeaderChromeMetrics, HeaderTitleLayout};
use super::{BackgroundId, DrawCtx, OverlayRequest, SceneBehavior, SceneTransition, UpdateCtx};

const DRAWN_BRIGHTNESS: f32 = 0.32;
const DRAWN_SCALE: f32 = 0.92;
/// Breathing room above the first grid row inside the scroll panel.
const GRID_TOP_PAD_FRAC: f32 = 0.014;
/// Breathing room below the flowers row inside the scroll panel.
const GRID_BOTTOM_PAD_FRAC: f32 = 0.028;
/// Bottom band of each grid cell reserved for the drawable/total counter.
const CELL_COUNTER_BAND_FRAC: f32 = 0.28;
/// Inset from the top of the tile band so tiles sit above the counter comfortably.
const CELL_TILE_TOP_PAD_FRAC: f32 = 0.06;

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
const GRID_ROWS: [(usize, usize); 5] = [(0, 9), (9, 9), (18, 9), (27, 7), (34, 4)];

const ROW_LABELS: [&str; 5] = ["Manzu", "Souzu", "Pinzu", "Honors", "Flowers"];

/// Line box height for a pinned `font_px` (see `docs/agents/font-scaling.md`).
#[inline]
fn text_line_h(font_px: f32) -> f32 {
    font_px * PLAIN_TEXT_LINE_STEP_MUL
}

/// Left inset for title/subtitle copy (matches draw rects).
fn copy_inset_x(w: f32) -> f32 {
    w * 0.055
}

/// Max width for header subtitle wrapping.
fn subtitle_max_w(w: f32) -> f32 {
    w * 0.72
}

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
        let back = HeaderChromeMetrics::from_window(w, h).back_rect_left();
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
    grid_top_pad: f32,
    pack_section_top: f32,
    total_content_h: f32,
    label_col_w: f32,
    slot_h: f32,
    row_gap: f32,
}

fn grid_row_metrics(w: f32, h: f32, jr: f32, content_h: f32) -> (f32, f32, f32, f32, f32, f32, f32) {
    let grid_top_pad = (h * GRID_TOP_PAD_FRAC).max(10.0 * jr);
    let grid_bottom_pad = (h * GRID_BOTTOM_PAD_FRAC).max(18.0 * jr);
    let label_col_w = w * 0.09;
    let grid_x = w * 0.055 + label_col_w;
    let grid_w = w * 0.88 - label_col_w;
    let available_h = (content_h - grid_top_pad - grid_bottom_pad).max(80.0 * jr);
    let min_gap = (4.0 * jr).max(3.0);
    let natural_slot_h = (grid_w / 9.0) * 1.36;
    let natural_grid_h =
        GRID_ROWS.len() as f32 * natural_slot_h + (GRID_ROWS.len() as f32 - 1.0) * min_gap;

    let (slot_h, row_gap) = if natural_grid_h <= available_h {
        let extra = (available_h - natural_grid_h).max(0.0);
        let gap = min_gap + extra / (GRID_ROWS.len() as f32 - 1.0).max(1.0);
        (natural_slot_h, gap)
    } else {
        let row_h =
            (available_h - min_gap * (GRID_ROWS.len() as f32 - 1.0)) / GRID_ROWS.len() as f32;
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
        grid_top_pad,
        grid_bottom_pad,
    )
}

fn cell_counter_font_px(slot_w: f32, window_h: f32) -> f32 {
    typography::tier_at_most(slot_w * 0.34, window_h)
}

/// Fixed header band above the scroll panel — layout and draw must share these rects.
struct HeaderBandMetrics {
    title_px: f32,
    sub_px: f32,
    title_line_h: f32,
    subtitle_line_h: f32,
    copy_x: f32,
    title_y: f32,
    subtitle_y: f32,
    content_top: f32,
}

fn header_band_metrics(
    back: [f32; 4],
    w: f32,
    h: f32,
    jr: f32,
    subtitle: &str,
) -> HeaderBandMetrics {
    let scale = metrics::scene_scale(w, h);
    let title_px = typography::size(typography::H24, h) * jr.min(1.12);
    let sub_px = typography::size(typography::H42, h) * jr.min(1.05);
    let title_line_h = text_line_h(title_px);
    let subtitle_line_h = widget::plain_text_block_height(
        subtitle,
        subtitle_max_w(w),
        sub_px,
        PLAIN_TEXT_LINE_STEP_MUL,
    );
    let after_subtitle_gap = 14.0 * jr;
    let title = HeaderTitleLayout::nav_row_aligned(
        back,
        copy_inset_x(w),
        (18.0 * scale).max(10.0),
        title_px,
        jr,
    );
    let content_top = title.subtitle_y + subtitle_line_h + after_subtitle_gap;
    HeaderBandMetrics {
        title_px,
        sub_px,
        title_line_h,
        subtitle_line_h,
        copy_x: title.copy_x,
        title_y: title.title_y,
        subtitle_y: title.subtitle_y,
        content_top,
    }
}

fn ledger_layout(
    w: f32,
    h: f32,
    jr: f32,
    pack_row_count: usize,
    subtitle: &str,
) -> LedgerLayout {
    let back = HeaderChromeMetrics::from_window(w, h).back_rect_left();
    let header = header_band_metrics(back, w, h, jr, subtitle);
    let bottom_safe = screen_footer_reserve(w, h);
    let content_top = header.content_top;
    let content_h = (h - bottom_safe - content_top).max(120.0 * jr);

    let label_col_w = w * 0.09;
    let (grid_x, grid_w, slot_h, row_gap, standard_grid_h, grid_top_pad, grid_bottom_pad) =
        grid_row_metrics(w, h, jr, content_h);
    let pack_row_count = pack_row_count;
    let pack_gap = if pack_row_count > 0 { 12.0 * jr } else { 0.0 };
    let pack_section_top = content_top
        + grid_top_pad
        + standard_grid_h
        + grid_bottom_pad
        + pack_gap
        + if pack_row_count > 0 { 22.0 * jr } else { 0.0 };
    let pack_section_h = if pack_row_count > 0 {
        pack_row_count as f32 * (slot_h + row_gap * 0.5) + 8.0 * jr
    } else {
        0.0
    };
    let total_content_h =
        grid_top_pad + standard_grid_h + grid_bottom_pad + pack_section_h + pack_gap;

    LedgerLayout {
        content_top,
        content_h,
        grid_x,
        grid_w,
        grid_top_pad,
        pack_section_top,
        total_content_h,
        label_col_w,
        slot_h,
        row_gap,
    }
}

fn groups_by_face(groups: &[WallLedgerFaceGroup]) -> HashMap<(Suit, u8), &WallLedgerFaceGroup> {
    groups.iter().map(|g| ((g.suit, g.rank), g)).collect()
}

fn face_supply_counts(entries: &[crate::game::wall_ledger::WallTileEntry]) -> (usize, usize) {
    let total = entries.len();
    let drawable = entries.iter().filter(|e| !e.drawn).count();
    (drawable, total)
}

fn cell_counter_band_h(cell_h: f32, counter_font_px: f32) -> f32 {
    let desired = cell_h * CELL_COUNTER_BAND_FRAC;
    let readable_min = text_line_h(counter_font_px) + 4.0;
    // Reserve at least the counter tier even when that steals from the tile band.
    let max_band = (cell_h * 0.42).max(readable_min);
    desired.max(readable_min).min(max_band).max(1.0)
}

fn cell_tile_and_counter_areas(
    cell: (f32, f32, f32, f32),
    counter_font_px: f32,
) -> ((f32, f32, f32, f32), (f32, f32, f32, f32)) {
    let (cx, cy, cw, ch) = cell;
    let counter_h = cell_counter_band_h(ch, counter_font_px);
    let tile_h = (ch - counter_h).max(1.0);
    let tile_area = (cx, cy, cw, tile_h);
    let counter_area = (cx, cy + tile_h, cw, counter_h);
    (tile_area, counter_area)
}

fn representative_entry<'a>(
    entries: &'a [crate::game::wall_ledger::WallTileEntry],
) -> Option<&'a crate::game::wall_ledger::WallTileEntry> {
    entries.iter().find(|e| !e.drawn).or_else(|| entries.first())
}

fn push_cell_tile(
    placements: &mut Vec<ShowcaseTilePlacement>,
    entry: &crate::game::wall_ledger::WallTileEntry,
    drawable: usize,
    tile_area: (f32, f32, f32, f32),
    run: &crate::game::run::RunState,
) {
    let (cx, cy, cw, ch) = tile_area;
    let top_pad = ch * CELL_TILE_TOP_PAD_FRAC;
    let usable_h = (ch - top_pad).max(1.0);
    let tile_size = (cw * 0.78).min(usable_h * 0.90);
    let exhausted = drawable == 0;
    placements.push(ShowcaseTilePlacement {
        tile: crate::game::engine::GameEngine::display_tile(entry.tile, run),
        center_pos: [cx + cw * 0.5, cy + top_pad + usable_h * 0.5, 0.0],
        rotation: [0.0, 0.0, std::f32::consts::PI],
        scale: if exhausted { DRAWN_SCALE } else { 1.0 },
        size_px: tile_size,
        brightness: if exhausted { DRAWN_BRIGHTNESS } else { 1.0 },
        selected: false,
        hovered: false,
        outline: false,
        glow: false,
        glow_color: None,
        outline_sel: None,
        pick_id: None,
        overlay_rect_group: None,
    });
}

fn push_cell_supply_counter(
    texts: &mut Vec<TextLabel>,
    drawable: usize,
    total: usize,
    counter_area: (f32, f32, f32, f32),
    font_px: f32,
    clip: Option<[f32; 4]>,
) {
    if total == 0 {
        return;
    }
    let (cx, cy, cw, ch) = counter_area;
    let label = format!("{drawable}/{total}");
    let (counter_color, bold) = if drawable == 0 {
        (color::alpha(color::UMBER, 0.72), false)
    } else if drawable == total {
        (color::alpha(color::CHAMPAGNE, 0.94), false)
    } else {
        (color::alpha(color::STONE, 0.92), true)
    };
    let line_h = text_line_h(font_px).min(ch);
    texts.push(TextLabel {
        rect: [cx, cy + (ch - line_h) * 0.5, cw, line_h],
        text: label,
        color: counter_color,
        font_px: Some(font_px),
        align: TextAlign::Center,
        scroll_offset: 0.0,
        flavor_spans: None,
        bold,
        italic: false,
        underline: false,
        text_effect: TextEffectId::Flat,
        rotation_quarters: 0,
        baseline_shift_px: 0.0,
        clip_rect: clip,
        block_vertical_align: Default::default(),
        mono: true,
    });
}

fn push_wall_ledger_cell(
    placements: &mut Vec<ShowcaseTilePlacement>,
    counter_texts: &mut Vec<TextLabel>,
    entries: &[crate::game::wall_ledger::WallTileEntry],
    cell: (f32, f32, f32, f32),
    run: &crate::game::run::RunState,
    counter_font_px: f32,
    clip: Option<[f32; 4]>,
) {
    let Some(entry) = representative_entry(entries) else {
        return;
    };
    let (drawable, total) = face_supply_counts(entries);
    let (tile_area, counter_area) = cell_tile_and_counter_areas(cell, counter_font_px);
    push_cell_tile(placements, entry, drawable, tile_area, run);
    push_cell_supply_counter(
        counter_texts,
        drawable,
        total,
        counter_area,
        counter_font_px,
        clip,
    );
}

impl SceneBehavior for WallLedgerScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let ledger = read_wall_ledger(ctx.run, self.mode);
        let jr = read_boost(w, h);
        let pack_rows = pack_display_rows(&ledger.pack_groups);
        let layout = ledger_layout(w, h, jr, pack_rows.len(), &ledger.subtitle);
        let max_scroll = (layout.total_content_h - layout.content_h).max(0.0);

        if ctx.scroll_lines.abs() > 0.001 {
            self.target_scroll_px = (self.target_scroll_px
                + ctx.scroll_lines * layout.slot_h * 0.85)
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

        let scroll_step = layout.slot_h * 0.85;
        for a in ctx.actions {
            match a {
                UiAction::FocusUp => {
                    self.target_scroll_px = (self.target_scroll_px - scroll_step).max(0.0);
                }
                UiAction::FocusDown => {
                    self.target_scroll_px =
                        (self.target_scroll_px + scroll_step).min(max_scroll);
                }
                _ => {}
            }
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
        let layout = ledger_layout(w, h, jr, pack_rows.len(), &ledger.subtitle);
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
        let cam = CameraParams {
            eye: [0.0, 0.0, 2040.0 * cam_scale],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            fovy_deg: 45.0,
            clip_near: None,
            clip_far: None,
        };
        frame.camera_override = Some(cam);
        // Placement policy: `wall_ledger` is on the pixel-to-world deny list (top-down 1:1 grid).

        frame.scene_lighting.push_smooth(PointLight {
            pos: [w * 0.5, h * -0.10, h * 1.40],
            radius: h * 3.0,
            color: color::rgb(color::PARCHMENT),
            intensity: 1.2,
        });

        push_back_button(&mut frame, &self.tree, w, h);

        let back = HeaderChromeMetrics::from_window(w, h).back_rect_left();
        let header = header_band_metrics(back, w, h, jr, &ledger.subtitle);
        let subtitle_lines =
            widget::wrap_text(&ledger.subtitle, subtitle_max_w(w), header.sub_px / 0.99);
        frame.texts([
            TextLabel {
                rect: [
                    header.copy_x,
                    header.title_y,
                    w * 0.55,
                    header.title_line_h,
                ],
                text: "Wall Ledger".into(),
                color: color::CHAMPAGNE,
                font_px: Some(header.title_px),
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
                block_vertical_align: Default::default(),
                mono: false,
            },
            TextLabel {
                rect: [
                    header.copy_x,
                    header.subtitle_y,
                    subtitle_max_w(w),
                    header.subtitle_line_h,
                ],
                text: subtitle_lines.join("\n"),
                color: color::STONE,
                font_px: Some(header.sub_px),
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
                block_vertical_align: Default::default(),
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
        frame.quad(GpuInstance {
            rect: [
                panel_rect[0] + 1.0,
                panel_rect[1],
                panel_rect[2] - 2.0,
                1.0,
            ],
            color: color::alpha(color::STONE, 0.35),
            user: 0,
        });

        let clip = panel_rect;
        let standard_by_face = groups_by_face(&ledger.standard_groups);
        let grid_y = panel_top + layout.grid_top_pad;
        let slots = grid_slots(
            layout.grid_x,
            grid_y,
            layout.grid_w,
            layout.slot_h,
            layout.row_gap,
        );
        let slot_w = layout.grid_w / 9.0;

        let label_px = typography::tier_at_most(layout.label_col_w * 0.92, h);
        let counter_px = cell_counter_font_px(slot_w, h);
        let counter_band = cell_counter_band_h(layout.slot_h, counter_px);
        let label_line_h = text_line_h(label_px);
        let tile_band_h = (layout.slot_h - counter_band).max(1.0);
        for (row_idx, label) in ROW_LABELS.iter().enumerate() {
            let row_y = grid_y + row_idx as f32 * (layout.slot_h + layout.row_gap);
            if row_y + layout.slot_h < clip[1] - 2.0 || row_y > clip[1] + clip[3] + 2.0 {
                continue;
            }
            let accent = row_suit_color(row_idx);
            let tile_center_y = row_y + tile_band_h * 0.5;
            frame.texts([TextLabel {
                rect: [
                    copy_inset_x(w),
                    tile_center_y - label_line_h * 0.5,
                    layout.label_col_w,
                    label_line_h,
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
                block_vertical_align: Default::default(),
                mono: false,
            }]);
        }

        let mut placements = Vec::new();
        let mut counter_texts = Vec::new();
        let cell_clip = Some(clip);
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
                    &mut counter_texts,
                    &group.copies,
                    *slot,
                    ctx.run,
                    counter_px,
                    cell_clip,
                );
            }
        }

        if !ledger.pack_groups.is_empty() {
            let pack_label_y = layout.pack_section_top - scroll;
            if pack_label_y + label_line_h >= clip[1] && pack_label_y <= clip[1] + clip[3] {
                frame.texts([TextLabel {
                    rect: [copy_inset_x(w), pack_label_y, w * 0.4, label_line_h],
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
                    block_vertical_align: Default::default(),
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
                        &mut counter_texts,
                        &group.copies,
                        cell,
                        ctx.run,
                        counter_px,
                        cell_clip,
                    );
                }
            }
        }

        if !placements.is_empty() {
            frame.showcase_tile_batch(placements);
        }
        if !counter_texts.is_empty() {
            frame.texts(counter_texts);
        }

        push_screen_footer_hint(
            &mut frame,
            &ctx,
            back_scroll_footer_row(ctx.input_mode),
            HintStyle::standard(w, h),
        );

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
    let back = HeaderChromeMetrics::from_window(w, h).back_rect_left();
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
