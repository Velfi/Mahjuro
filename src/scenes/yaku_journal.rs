//! Yaku Journal — pushdown scene. Tiles on the table.
//!
//! Replaces the old parchment overlay with its own scene: the player steps
//! *onto* the table rather than into a book. Each yaku is rendered as a
//! signature-tile icon in a grid on the lacquered wood surface; the focused
//! yaku's canonical 14-tile hand sits in a top-anchored plaque beneath a
//! single-line header (`[Lv N] Name: description` + payout), with the
//! scrollable table beneath it. Margins tighten on short screens (e.g. Steam Deck),
//! drawn from the same `yaku_page()` data the guide teaches with (so
//! the plaque hand is guaranteed to score as its named yaku — see the
//! scoring test in guide).
//!
//! Kokushi Musō does not appear in the grid until the first time it is cashed in
//! (same gate as `PlayerProgress::available_yaku`).

use crate::core::tile::Tile;
use crate::core::yaku::YakuKind;
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::render::decal::{load_mono_font, load_ui_font, measure_label_advances};
use crate::render::draw_cmd::{CameraParams, ShowcaseTilePlacement, UiFrame};
use crate::render::theme::{
    ButtonState, ButtonVariant, color,
    color::score_cascade::{CHIPS as CHIPS_COLOR, MULT as MULT_COLOR},
    typography,
};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::sfx_id::SfxId;
use crate::ui::controller_hints::{
    HintStyle, journal_plaque_footer_row, push_screen_footer_hint, screen_footer_reserve,
};
use crate::ui::focus_nav;
use crate::ui::input::UiAction;
use crate::ui::widget;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};
use std::time::Instant;

use super::header_chrome::HeaderChromeMetrics;
use super::{BackgroundId, DrawCtx, OverlayRequest, SceneBehavior, SceneTransition, UpdateCtx};

const CLICK_ROW_BASE: u32 = 0xE100;

#[derive(Clone, Copy)]
struct JournalScrollbar {
    track: [f32; 4],
    thumb: [f32; 4],
    /// Wider than the painted track so the thumb is easier to grab.
    hit_track: [f32; 4],
    thumb_travel: f32,
}

#[inline]
fn point_in_rect(mx: f32, my: f32, r: [f32; 4]) -> bool {
    mx >= r[0] && mx <= r[0] + r[2] && my >= r[1] && my <= r[1] + r[3]
}

fn journal_scrollbar(
    table: &JournalTableLayout,
    body_top: f32,
    jr: f32,
    total_rows: usize,
    scroll: f32,
    max_scroll: f32,
) -> Option<JournalScrollbar> {
    if max_scroll <= 0.0 {
        return None;
    }
    let track_w = (6.0 * jr).max(4.0);
    let track_x = table.table_x + table.table_w - track_w - 4.0 * jr;
    let track_y = body_top + 3.0 * jr;
    let track_h = table.table_h - table.header_h - 6.0 * jr;
    let thumb_h =
        (track_h * (table.visible_rows as f32 / total_rows as f32)).clamp(24.0 * jr, track_h);
    let thumb_travel = (track_h - thumb_h).max(0.0);
    let thumb_t = scroll / max_scroll;
    let thumb_y = track_y + thumb_travel * thumb_t;
    let hit_pad_x = (8.0 * jr).max(6.0);
    Some(JournalScrollbar {
        track: [track_x, track_y, track_w, track_h],
        thumb: [track_x, thumb_y, track_w, thumb_h],
        hit_track: [
            track_x - hit_pad_x,
            track_y,
            track_w + hit_pad_x * 2.0,
            track_h,
        ],
        thumb_travel,
    })
}

fn scroll_rows_from_thumb_top(thumb_top: f32, sb: &JournalScrollbar, max_scroll: f32) -> f32 {
    if sb.thumb_travel <= 0.0 {
        return 0.0;
    }
    let t = ((thumb_top - sb.track[1]) / sb.thumb_travel).clamp(0.0, 1.0);
    t * max_scroll
}

fn scroll_rows_from_cursor(my: f32, grab_y: f32, sb: &JournalScrollbar, max_scroll: f32) -> f32 {
    let thumb_top = (my - grab_y - sb.track[1]).clamp(0.0, sb.thumb_travel);
    scroll_rows_from_thumb_top(thumb_top, sb, max_scroll)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JournalNav {
    Back,
    Row(usize),
}

impl JournalNav {
    fn id(self) -> FocusId {
        match self {
            Self::Back => FocusId(0xE0F0),
            Self::Row(i) => FocusId(CLICK_ROW_BASE + i as u32),
        }
    }
}

/// Upper-left Back control — same sizing as [`super::guide::GuideLayout::header_chrome`].
fn journal_header_chrome(window_w: f32, window_h: f32) -> ([f32; 4], f32) {
    let chrome = HeaderChromeMetrics::from_window(window_w, window_h);
    (chrome.back_rect_left(), chrome.chrome_bottom())
}

/// Couch / TV legibility: ramps up font floors and tile mins from ~720p
/// short edge toward large displays (capped so UI doesn't balloon on 4K).
#[inline]
fn journal_read_boost(window_w: f32, window_h: f32) -> f32 {
    (window_w.min(window_h) / 720.0).clamp(1.0, 1.38)
}

/// Tighter margins and vertical split for handheld / Steam Deck-class aspect
/// ratios (short edge ~800px). 0 = TV/living-room defaults; 1 = max compaction.
#[inline]
fn journal_compact_factor(short_edge_px: f32) -> f32 {
    ((960.0 - short_edge_px) / 320.0).clamp(0.0, 1.0)
}

#[derive(Clone, Copy)]
struct JournalTableLayout {
    table_x: f32,
    table_y: f32,
    table_w: f32,
    table_h: f32,
    header_h: f32,
    row_h: f32,
    visible_rows: usize,
}

fn yaku_table_layout(
    window_w: f32,
    window_h: f32,
    jr: f32,
    jc: f32,
) -> (JournalTableLayout, f32, f32) {
    let (_, chrome_bottom) = journal_header_chrome(window_w, window_h);
    let top_safe = chrome_bottom + window_h * (0.008 - 0.005 * jc);
    let gap_below_plaque = window_h * (0.018 - 0.011 * jc);
    let plaque_h = window_h * (0.365 - 0.028 * jc);
    let bottom_safe = screen_footer_reserve(window_w, window_h);
    let plaque_top = top_safe;

    let table_x = window_w * (0.055 - 0.023 * jc);
    let table_w = window_w * (0.89 + 0.046 * jc);
    let table_top = plaque_top + plaque_h + gap_below_plaque;
    let table_h = (window_h - bottom_safe - table_top).max(120.0 * jr);
    let header_h = (44.0 * jr).clamp(32.0, 64.0);
    let row_h = (54.0 * jr).clamp(40.0, 82.0);
    let visible_rows = ((table_h - header_h) / row_h).floor().max(1.0) as usize;
    (
        JournalTableLayout {
            table_x,
            table_y: table_top,
            table_w,
            table_h,
            header_h,
            row_h,
            visible_rows,
        },
        plaque_top,
        plaque_h,
    )
}

pub struct YakuJournalScene {
    /// Index into the visible yaku row order (`PlayerProgress::available_yaku`).
    selected: usize,
    scroll_rows: f32,
    target_scroll_rows: f32,
    scroll_last_tick: Instant,
    tree: TreeState,
    dragging_scrollbar: bool,
    scroll_drag_grab_y: f32,
    prev_mouse_down: bool,
}

impl Default for YakuJournalScene {
    fn default() -> Self {
        Self::new()
    }
}

impl YakuJournalScene {
    pub fn new() -> Self {
        let mut tree = TreeState::new();
        tree.set_focus(JournalNav::Row(0).id());
        Self {
            selected: 0,
            scroll_rows: 0.0,
            target_scroll_rows: 0.0,
            scroll_last_tick: Instant::now(),
            tree,
            dragging_scrollbar: false,
            scroll_drag_grab_y: 0.0,
            prev_mouse_down: false,
        }
    }

    fn flat_items(
        &self,
        w: f32,
        h: f32,
        table: &JournalTableLayout,
        scroll: f32,
        total_rows: usize,
    ) -> Vec<FlatItem<JournalNav>> {
        let (back, _) = journal_header_chrome(w, h);
        let mut items = vec![FlatItem::new(JournalNav::Back.id(), back, JournalNav::Back)];
        if total_rows == 0 {
            return items;
        }
        let jr = journal_read_boost(w, h);
        let body_top = table.table_y + table.header_h;
        let max_scroll = Self::max_scroll(total_rows, table.visible_rows);
        let row_click_w = journal_scrollbar(&table, body_top, jr, total_rows, scroll, max_scroll)
            .map(|sb| sb.hit_track[0] - table.table_x)
            .unwrap_or(table.table_w);
        for idx in 0..total_rows {
            let row_y = body_top + (idx as f32 - scroll) * table.row_h;
            items.push(FlatItem::new(
                JournalNav::Row(idx).id(),
                [table.table_x, row_y, row_click_w, table.row_h],
                JournalNav::Row(idx),
            ));
        }
        items
    }

    fn sync_selected_from_focus(&mut self, visible_rows: usize, max_scroll: f32) {
        let Some(id) = self.tree.focused() else {
            return;
        };
        if let JournalNav::Row(idx) = journal_nav_from_id(id) {
            self.selected = idx;
            self.ensure_selected_visible(visible_rows, max_scroll);
        }
    }
}

fn journal_nav_from_id(id: FocusId) -> JournalNav {
    if id == JournalNav::Back.id() {
        JournalNav::Back
    } else if id.0 >= CLICK_ROW_BASE {
        JournalNav::Row((id.0 - CLICK_ROW_BASE) as usize)
    } else {
        JournalNav::Back
    }
}

impl YakuJournalScene {
    fn go_back(overlay_request: &mut Option<OverlayRequest>) -> SceneTransition {
        *overlay_request = Some(OverlayRequest::Pop);
        None
    }

    fn max_scroll(total_rows: usize, visible_rows: usize) -> f32 {
        total_rows.saturating_sub(visible_rows) as f32
    }

    fn clamp_scroll(&mut self, max_scroll: f32) {
        self.target_scroll_rows = self.target_scroll_rows.clamp(0.0, max_scroll);
        self.scroll_rows = self.scroll_rows.clamp(0.0, max_scroll);
    }

    fn ensure_selected_visible(&mut self, visible_rows: usize, max_scroll: f32) {
        let top = self.target_scroll_rows.floor() as usize;
        if self.selected < top {
            self.target_scroll_rows = self.selected as f32;
        } else if self.selected >= top + visible_rows {
            self.target_scroll_rows = (self.selected + 1 - visible_rows) as f32;
        }
        self.target_scroll_rows = self.target_scroll_rows.clamp(0.0, max_scroll);
    }

    fn tick_scroll(&mut self) {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.scroll_last_tick)
            .as_secs_f32();
        self.scroll_last_tick = now;
        let t = (dt * 12.0).clamp(0.0, 1.0);
        self.scroll_rows += (self.target_scroll_rows - self.scroll_rows) * t;
    }
}

impl SceneBehavior for YakuJournalScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let yaku_list = ctx.progress.available_yaku();
        let total_rows = yaku_list.len();
        if total_rows > 0 && self.selected >= total_rows {
            self.selected = total_rows - 1;
        }
        if total_rows == 0 {
            self.selected = 0;
            self.scroll_rows = 0.0;
            self.target_scroll_rows = 0.0;
        }

        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let jr = journal_read_boost(w, h);
        let jc = journal_compact_factor(w.min(h));
        let (table, _, _) = yaku_table_layout(w, h, jr, jc);
        let max_scroll = Self::max_scroll(total_rows, table.visible_rows);
        self.clamp_scroll(max_scroll);

        if ctx.scroll_lines.abs() > 0.001 {
            self.target_scroll_rows =
                (self.target_scroll_rows + ctx.scroll_lines).clamp(0.0, max_scroll);
        }

        let body_top = table.table_y + table.header_h;
        let scroll = self.target_scroll_rows;
        let scrollbar = journal_scrollbar(&table, body_top, jr, total_rows, scroll, max_scroll);
        let (mx, my) = ctx.cursor_pos;
        let mouse_down = ctx.mouse_left_down;
        let mouse_click = mouse_down && !self.prev_mouse_down;
        if let Some(sb) = scrollbar {
            if self.dragging_scrollbar && mouse_down {
                let rows = scroll_rows_from_cursor(my, self.scroll_drag_grab_y, &sb, max_scroll);
                self.target_scroll_rows = rows;
                self.scroll_rows = rows;
            } else if mouse_click {
                if point_in_rect(mx, my, sb.thumb) {
                    self.dragging_scrollbar = true;
                    self.scroll_drag_grab_y = my - sb.thumb[1];
                    let rows =
                        scroll_rows_from_cursor(my, self.scroll_drag_grab_y, &sb, max_scroll);
                    self.target_scroll_rows = rows;
                    self.scroll_rows = rows;
                } else if point_in_rect(mx, my, sb.hit_track) {
                    self.dragging_scrollbar = true;
                    self.scroll_drag_grab_y = sb.thumb[3] * 0.5;
                    let rows =
                        scroll_rows_from_cursor(my, self.scroll_drag_grab_y, &sb, max_scroll);
                    self.target_scroll_rows = rows;
                    self.scroll_rows = rows;
                }
            }
        }
        if !mouse_down {
            self.dragging_scrollbar = false;
        }
        self.prev_mouse_down = mouse_down;

        for a in ctx.actions {
            if matches!(a, UiAction::Cancel | UiAction::Pause | UiAction::Help) {
                return Self::go_back(ctx.overlay_request);
            }
        }

        let items = self.flat_items(w, h, &table, self.target_scroll_rows, total_rows);
        let nav_action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
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
        self.sync_selected_from_focus(table.visible_rows, max_scroll);
        match nav_action {
            Some(JournalNav::Back) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                return Self::go_back(ctx.overlay_request);
            }
            Some(JournalNav::Row(i)) => {
                self.selected = i;
                self.ensure_selected_visible(table.visible_rows, max_scroll);
            }
            None => {}
        }

        for a in ctx.actions {
            match a {
                UiAction::PagePrev => {
                    if total_rows > 0 {
                        let page = table.visible_rows.max(1);
                        self.selected = self.selected.saturating_sub(page);
                        self.ensure_selected_visible(table.visible_rows, max_scroll);
                        self.tree.set_focus(JournalNav::Row(self.selected).id());
                    }
                }
                UiAction::PageNext => {
                    if total_rows > 0 {
                        let page = table.visible_rows.max(1);
                        self.selected = (self.selected + page).min(total_rows.saturating_sub(1));
                        self.ensure_selected_visible(table.visible_rows, max_scroll);
                        self.tree.set_focus(JournalNav::Row(self.selected).id());
                    }
                }
                _ => {}
            }
        }
        self.tick_scroll();
        self.clamp_scroll(max_scroll);
        None
    }

    fn draw_frame(&self, mut ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let jr = journal_read_boost(w, h);
        let short_edge = w.min(h);
        let jc = journal_compact_factor(short_edge);
        let run = ctx.run;
        let progress = ctx.progress;
        let yaku = progress.available_yaku();
        let (table, plaque_top, plaque_h) = yaku_table_layout(w, h, jr, jc);
        let total_rows = yaku.len();

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        // Keep journal UI on a controlled walnut backdrop (no felt table tint bleed-through).
        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::WALNUT_INK,
            user: 0,
        });

        // Camera — directly top-down so pixel-space layout and the
        // projected tile positions stay 1:1 with each other, letting
        // the grid math place captions without projection offset.
        let cam_scale = h / 1600.0;
        frame.camera_override = Some(CameraParams {
            eye: [0.0, 0.0, 2040.0 * cam_scale],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            projection: crate::render::draw_cmd::CameraProjection::Perspective { fovy_deg: 45.0 },
            clip_near: None,
            clip_far: None,
        });
        // Placement policy: `yaku_journal` is on the pixel-to-world deny list (top-down 1:1 grid).

        // One soft high fill light. The previous two-light setup created
        // bright specular blooms on the wood that pulled the eye away
        // from the grid; a single, very high, wide-radius light gives
        // tiles enough dimensional shading without hotspots.
        frame.scene_lighting.push_smooth(PointLight {
            pos: [w * 0.5, h * -0.10, h * 1.40],
            radius: h * 3.0,
            color: color::rgb(color::PARCHMENT),
            intensity: 1.2,
        });

        push_journal_back_button(&mut frame, &self.tree, w, h);

        // ── Scrollable yaku table (sticky header) ────────────────
        let mut placements: Vec<ShowcaseTilePlacement> = Vec::new();
        let mut tile_id: u32 = 0;
        let yaku_progress = GameEngine::read_yaku_progress(run);
        let table_bg = color::WALNUT_DEEP;
        frame.quad(GpuInstance {
            rect: [table.table_x, table.table_y, table.table_w, table.table_h],
            color: table_bg,
            user: 0,
        });

        let header_bg = color::alpha(color::WALNUT_RAISED, 0.96);
        frame.quad(GpuInstance {
            rect: [table.table_x, table.table_y, table.table_w, table.header_h],
            color: header_bg,
            user: 0,
        });

        let row_level_w = table.table_w * 0.06;
        let row_name_w = table.table_w * 0.15;
        let row_payout_w = table.table_w * 0.17;
        let row_scored_w = table.table_w * 0.08;
        let row_rule_w = table.table_w - row_level_w - row_name_w - row_payout_w - row_scored_w;
        let col_x = [
            table.table_x,
            table.table_x + row_level_w,
            table.table_x + row_level_w + row_name_w,
            table.table_x + row_level_w + row_name_w + row_rule_w,
            table.table_x + row_level_w + row_name_w + row_rule_w + row_payout_w,
        ];
        let header_font = typography::size(typography::H42, h);
        let body_font = typography::size(typography::H42, h);
        let tiny_font = typography::size(typography::H42, h);
        let col_pad = 9.0 * jr;

        let headers = ["Lvl.", "Name", "Rule", "Payout", "Run"];
        for (i, title) in headers.iter().enumerate() {
            let cw = match i {
                0 => row_level_w,
                1 => row_name_w,
                2 => row_rule_w,
                3 => row_payout_w,
                _ => row_scored_w,
            };
            frame.text(TextLabel {
                rect: [
                    col_x[i] + col_pad,
                    table.table_y,
                    cw - col_pad * 2.0,
                    table.header_h,
                ],
                text: (*title).into(),
                color: color::CHAMPAGNE,
                align: TextAlign::Left,
                font_px: Some(header_font),
                ..Default::default()
            });
            if i > 0 {
                frame.quad(GpuInstance {
                    rect: [col_x[i], table.table_y, 1.0_f32.max(jr), table.table_h],
                    color: color::alpha(color::CHAMPAGNE, 0.08),
                    user: 0,
                });
            }
        }

        let body_top = table.table_y + table.header_h;
        let body_bottom = table.table_y + table.table_h - 2.0 * jr;
        let body_clip_rect = [
            table.table_x,
            body_top,
            table.table_w,
            (body_bottom - body_top).max(0.0),
        ];
        let max_scroll = Self::max_scroll(total_rows, table.visible_rows);
        let scroll = self.scroll_rows.clamp(0.0, max_scroll);
        let scrollbar = journal_scrollbar(&table, body_top, jr, total_rows, scroll, max_scroll);
        let first_row = scroll.floor() as usize;
        let row_offset = scroll - first_row as f32;
        let draw_rows = table.visible_rows + 2;
        for vi in 0..draw_rows {
            let idx = first_row + vi;
            if idx >= total_rows {
                break;
            }
            let yk = yaku[idx];
            let row_y = body_top + (vi as f32 - row_offset) * table.row_h;
            let row_bottom = row_y + table.row_h;
            let clipped_row_top = row_y.max(body_top);
            let clipped_row_bottom = row_bottom.min(body_bottom);
            let clipped_row_h = clipped_row_bottom - clipped_row_top;
            if clipped_row_h <= 0.0 {
                continue;
            }
            let is_selected = idx == self.selected;
            let state = progression_state(run, progress, yk);
            let lvl = yaku_progress.level_of(yk);
            let chips = yk.chip_bonus_at(lvl);
            let mult = yk.mult_bonus_at(lvl);
            let scored_this_run = yaku_progress.played_this_run(yk);
            let (rule_text, _) = super::guide::yaku_page(yk);

            let base_row_color = if vi % 2 == 0 {
                color::WALNUT_RAISED
            } else {
                color::alpha(color::WALNUT_RAISED, 0.86)
            };
            frame.quad(GpuInstance {
                rect: [table.table_x, clipped_row_top, table.table_w, clipped_row_h],
                color: base_row_color,
                user: 0,
            });
            if is_selected {
                frame.quad(GpuInstance {
                    rect: [table.table_x, clipped_row_top, table.table_w, clipped_row_h],
                    color: color::alpha(color::CHAMPAGNE, 0.14),
                    user: 0,
                });
            }
            let row_text_color = if is_selected {
                color::CHAMPAGNE
            } else {
                color::PARCHMENT
            };
            let level_text = match state {
                ProgressionState::Unseen => "—".into(),
                _ => format!("{lvl}"),
            };
            frame.text(TextLabel {
                rect: [
                    col_x[0] + col_pad,
                    row_y,
                    row_level_w - col_pad * 2.0,
                    table.row_h,
                ],
                text: level_text,
                color: if matches!(state, ProgressionState::Leveled) {
                    color::GOLD
                } else {
                    row_text_color
                },
                align: TextAlign::Left,
                font_px: Some(body_font),
                clip_rect: Some(body_clip_rect),
                ..Default::default()
            });

            let name_text: String = match state {
                ProgressionState::Unseen => "???".into(),
                _ => yk.name().into(),
            };
            frame.text(TextLabel {
                rect: [
                    col_x[1] + col_pad,
                    row_y,
                    row_name_w - col_pad * 2.0,
                    table.row_h,
                ],
                text: name_text,
                color: row_text_color,
                align: TextAlign::Left,
                font_px: Some(body_font),
                clip_rect: Some(body_clip_rect),
                ..Default::default()
            });

            let rule_row_text = match state {
                ProgressionState::Unseen => "sealed — score once to reveal".into(),
                _ => rule_text.into(),
            };
            frame.text(TextLabel {
                rect: [
                    col_x[2] + col_pad,
                    row_y,
                    row_rule_w - col_pad * 2.0,
                    table.row_h,
                ],
                text: rule_row_text,
                color: color::alpha(row_text_color, 0.90),
                align: TextAlign::Left,
                font_px: Some(tiny_font),
                clip_rect: Some(body_clip_rect),
                ..Default::default()
            });

            let payout_rect = [
                col_x[3] + col_pad,
                row_y,
                row_payout_w - col_pad * 2.0,
                table.row_h,
            ];
            match state {
                ProgressionState::Unseen => {
                    frame.text(TextLabel {
                        rect: payout_rect,
                        text: "—".into(),
                        color: row_text_color,
                        align: TextAlign::Left,
                        font_px: Some(tiny_font),
                        clip_rect: Some(body_clip_rect),
                        ..Default::default()
                    });
                }
                _ => {
                    push_chips_mult_stat_line(
                        &mut frame,
                        payout_rect[0],
                        payout_rect[1],
                        payout_rect[3],
                        tiny_font,
                        chips,
                        mult,
                        Some(body_clip_rect),
                    );
                }
            }
            frame.text(TextLabel {
                rect: [
                    col_x[4] + col_pad,
                    row_y,
                    row_scored_w - col_pad * 2.0,
                    table.row_h,
                ],
                text: format!("{scored_this_run}"),
                color: row_text_color,
                align: TextAlign::Left,
                font_px: Some(body_font),
                clip_rect: Some(body_clip_rect),
                ..Default::default()
            });
        }

        if let Some(sb) = scrollbar {
            frame.quad(GpuInstance {
                rect: sb.track,
                color: color::alpha(color::WALNUT_INK, 0.45),
                user: 0,
            });
            let thumb_alpha = if self.dragging_scrollbar { 1.0 } else { 0.82 };
            frame.quad(GpuInstance {
                rect: sb.thumb,
                color: color::alpha(color::CHAMPAGNE, thumb_alpha),
                user: 0,
            });
        }

        // ── Floating plaque for the selected yaku ────────────────
        if let Some(&sel_yk) = yaku.get(self.selected) {
            let sel_state = progression_state(run, progress, sel_yk);
            draw_plaque(
                &mut frame,
                &mut placements,
                &mut tile_id,
                sel_yk,
                sel_state,
                plaque_top,
                plaque_h,
                w,
                h,
                jr,
                jc,
                &ctx,
            );
        }

        if !placements.is_empty() {
            frame.showcase_tile_batch(placements);
        }

        let items = self.flat_items(w, h, &table, self.target_scroll_rows, total_rows);
        self.tree.register_flat_buttons(&items, &mut frame.buttons);
        ctx.stash_focus_nav_tree_flat(&self.tree, &items, |a| format!("{a:?}"));
        push_screen_footer_hint(
            &mut frame,
            &ctx,
            journal_plaque_footer_row(ctx.input_mode),
            HintStyle::standard(w, h),
        );
        frame
    }
}

fn push_journal_back_button(frame: &mut UiFrame, tree: &TreeState, w: f32, h: f32) {
    let scale = (w.min(h)) / 600.0;
    let (back, _) = journal_header_chrome(w, h);
    let focused = tree.focused() == Some(JournalNav::Back.id());
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
        frame.text(label);
    }
}

/// Draw a "sealed" tablet where a tile strip would otherwise go: a warm
/// antique card with a stacked wax-seal disc in the center. The disc is
/// built from concentric quads sized to read as round at TV distance, with
/// a highlight crescent on top so the seal feels 3D rather than painted.
///
/// Earlier iteration used a dark obsidian slab; it read as a debug
/// placeholder next to the warm wood table. Warm-antique card with an
/// inked rim stays in the same material vocabulary as the plaque.
fn draw_sealed_slab(
    frame: &mut UiFrame,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    window_h: f32,
    read_boost: f32,
) {
    let scale = (window_h / 1080.0).max(1.0) * read_boost;
    let inset = (2.0 * scale).max(1.0);

    // Inked rim + deep-lacquer card face — darker than parchment so the
    // seal reads as a *locked chapter* rather than a waiting page, and
    // darker than the wood table so it stands apart from the background.
    // Color below is roughly darkened WALNUT_INK with a warm lift, chosen
    // by eye to contrast against both PARCHMENT and the wood grain.
    frame.quad(GpuInstance {
        rect: [x, y, w, h],
        color: color::darken(color::ANTIQUE, 0.75),
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x + inset, y + inset, w - inset * 2.0, h - inset * 2.0],
        color: color::WALNUT_RAISED,
        user: 0,
    });

    // Wax seal — stacked discs. Outer shadow ring sits slightly offset
    // down/right to fake drop shadow and give the disc lift. Then the
    // dark wax rim, the bright wax body, and a small offset highlight
    // crescent in champagne so the seal reads as 3D. Sized to nearly
    // fill the short edge of the card so it's the visual anchor.
    let seal_d = h.min(w) * 0.85;
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;

    // Shadow pad (offset) — very translucent dark, gives the seal lift.
    frame.quad(GpuInstance {
        rect: [
            cx - seal_d * 0.5 + 3.0 * scale,
            cy - seal_d * 0.5 + 4.0 * scale,
            seal_d,
            seal_d,
        ],
        color: color::alpha(color::WALNUT_INK, 0.55),
        user: 0,
    });
    // Dark wax ring.
    frame.quad(GpuInstance {
        rect: [cx - seal_d * 0.5, cy - seal_d * 0.5, seal_d, seal_d],
        color: color::darken(color::RUBY, 0.5),
        user: 0,
    });
    // Wax body.
    let body_d = seal_d * 0.86;
    frame.quad(GpuInstance {
        rect: [cx - body_d * 0.5, cy - body_d * 0.5, body_d, body_d],
        color: color::RUBY,
        user: 0,
    });
    // Highlight crescent — small off-center champagne square reads as a
    // specular hit on the wax. Placed up-left of center, sized small
    // enough that rectangular edges aren't obvious at TV distance.
    let hl_d = body_d * 0.28;
    frame.quad(GpuInstance {
        rect: [
            cx - body_d * 0.22 - hl_d * 0.5,
            cy - body_d * 0.22 - hl_d * 0.5,
            hl_d,
            hl_d,
        ],
        color: color::alpha(color::CHAMPAGNE, 0.55),
        user: 0,
    });

    // "?" stamp — larger glyph, champagne ink so it reads as pressed
    // metal into the wax rather than a flat typeface.
    let glyph_font = typography::size(typography::H20, window_h);
    frame.text(TextLabel {
        rect: [
            cx - seal_d * 0.5,
            cy - glyph_font * 0.55,
            seal_d,
            glyph_font * 1.1,
        ],
        text: "?".into(),
        color: color::alpha(color::CHAMPAGNE, 0.92),
        align: TextAlign::Center,
        font_px: Some(typography::size(typography::H16, window_h)),
        ..Default::default()
    });
}

struct MysteryNamePillStyle {
    pill_center_x: f32,
    top_y: f32,
    pill_h: f32,
    font_px: f32,
    shadow_scale: f32,
    pill_bg: [f32; 4],
    text_color: [f32; 4],
}

/// Caption / header pill hiding the yaku name until the first cash-in.
fn draw_mystery_name_pill(frame: &mut UiFrame, style: MysteryNamePillStyle) {
    let MysteryNamePillStyle {
        pill_center_x,
        top_y,
        pill_h,
        font_px,
        shadow_scale,
        pill_bg,
        text_color,
    } = style;
    let pill_w = font_px * 2.38;
    let pill_x = pill_center_x - pill_w * 0.5;
    frame.quad(GpuInstance {
        rect: [
            pill_x + 1.5 * shadow_scale,
            top_y + 2.0 * shadow_scale,
            pill_w,
            pill_h,
        ],
        color: color::alpha(color::WALNUT_DEEP, 0.35),
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [pill_x, top_y, pill_w, pill_h],
        color: pill_bg,
        user: 0,
    });
    frame.text(TextLabel {
        rect: [pill_x, top_y + pill_h * 0.12, pill_w, pill_h * 0.76],
        text: "???".into(),
        color: text_color,
        align: TextAlign::Center,
        font_px: Some(font_px),
        ..Default::default()
    });
}

/// Progression state for one yaku. Drives the grid material cues and the
/// plaque's reveal/veil decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgressionState {
    /// Never **cashed in** this yaku in any run (`PlayerProgress::yaku_times_scored`
    /// is zero). Rendered as a sealed tablet and `???` caption so first-time
    /// discovery stays a moment.
    Unseen,
    /// Scored at least once in any run's history, still at base level.
    Played,
    /// Zodiac-leveled to 2 or above. Gets a gold glow.
    Leveled,
}

/// Journal reveal is tied to **cash-in** only: cumulative
/// `PlayerProgress::yaku_times_scored` (from `GameEvent::YakuScored`), not
/// in-run preview state.
fn progression_state(
    run: &crate::game::run::RunState,
    progress: &crate::core::progression::PlayerProgress,
    yk: YakuKind,
) -> ProgressionState {
    let yaku_progress = GameEngine::read_yaku_progress(run);
    let lvl = yaku_progress.level_of(yk);
    let scored_ever = progress.yaku_times_scored.get(&yk).copied().unwrap_or(0);
    if scored_ever == 0 {
        ProgressionState::Unseen
    } else if lvl >= 2 {
        ProgressionState::Leveled
    } else {
        ProgressionState::Played
    }
}

fn format_yaku_mult_bonus(mult: f64) -> String {
    if (mult - mult.round()).abs() < 1e-6 {
        format!("{}", mult.round() as i64)
    } else {
        format!("{mult:.1}")
    }
}

fn format_yaku_mult_plus(mult: f64) -> String {
    format!("+{}", format_yaku_mult_bonus(mult))
}

/// Fixed glyph width for payout numerals so the table column lines up row to row.
const PAYOUT_VALUE_COL_W: usize = 5;

fn measure_payout_text_width(text: &str, font_px: f32) -> f32 {
    let text_h_px = font_px.max(8.0).round().max(1.0) as u32;
    let font = load_mono_font().or_else(load_ui_font);
    if let Some(font) = font {
        let (_, _, advances) = measure_label_advances(font, text, 8192, text_h_px, Some(font_px));
        advances.iter().copied().sum()
    } else {
        (font_px * 0.52 * text.chars().count().max(1) as f32).max(8.0)
    }
}

fn measure_ui_text_width(text: &str, font_px: f32) -> f32 {
    let text_h_px = font_px.max(8.0).round().max(1.0) as u32;
    if let Some(font) = load_ui_font() {
        let (_, _, advances) = measure_label_advances(font, text, 8192, text_h_px, Some(font_px));
        advances.iter().copied().sum()
    } else {
        (font_px * 0.52 * text.chars().count().max(1) as f32).max(8.0)
    }
}

fn format_payout_value_col(raw: &str) -> String {
    if raw.len() >= PAYOUT_VALUE_COL_W {
        raw.to_string()
    } else {
        format!("{raw:>PAYOUT_VALUE_COL_W$}")
    }
}

fn chips_mult_stat_segments(chips: i32, mult: f64) -> Vec<(String, [f32; 4])> {
    let sep_color = color::alpha(color::CHAMPAGNE, 0.50);
    vec![
        (format_payout_value_col(&format!("+{chips}")), CHIPS_COLOR),
        (" cp ".into(), CHIPS_COLOR),
        ("/".into(), sep_color),
        (
            format_payout_value_col(&format_yaku_mult_plus(mult)),
            MULT_COLOR,
        ),
        (" mu".into(), MULT_COLOR),
    ]
}

fn push_chips_mult_stat_segments(
    frame: &mut UiFrame,
    x: f32,
    y: f32,
    band_h: f32,
    font_px: f32,
    segments: &[(String, [f32; 4])],
    clip_rect: Option<[f32; 4]>,
) {
    let mut cursor_x = x;
    for (text, seg_color) in segments {
        let w = measure_payout_text_width(text, font_px).max(1.0);
        frame.text(TextLabel {
            rect: [cursor_x, y, w, band_h],
            text: text.clone(),
            color: *seg_color,
            align: TextAlign::Left,
            font_px: Some(font_px),
            mono: true,
            clip_rect,
            ..Default::default()
        });
        cursor_x += w;
    }
}

fn push_chips_mult_stat_line(
    frame: &mut UiFrame,
    x: f32,
    y: f32,
    band_h: f32,
    font_px: f32,
    chips: i32,
    mult: f64,
    clip_rect: Option<[f32; 4]>,
) {
    let segments = chips_mult_stat_segments(chips, mult);
    push_chips_mult_stat_segments(frame, x, y, band_h, font_px, &segments, clip_rect);
}

fn push_chips_mult_stat_line_right(
    frame: &mut UiFrame,
    right_x: f32,
    y: f32,
    band_h: f32,
    font_px: f32,
    chips: i32,
    mult: f64,
) {
    let segments = chips_mult_stat_segments(chips, mult);
    let total_w: f32 = segments
        .iter()
        .map(|(t, _)| measure_payout_text_width(t, font_px))
        .sum();
    push_chips_mult_stat_segments(
        frame,
        (right_x - total_w).max(0.0),
        y,
        band_h,
        font_px,
        &segments,
        None,
    );
}

/// Draw the floating plaque: a bamboo-lacquer panel across the bottom of the
/// screen showing the selected yaku's canonical 14-tile hand, scoring
/// values, name, and description. The hand comes from
/// `super::guide::yaku_page`, validated by the scorer test in that
/// module — so whatever renders here is guaranteed to score as the named
/// yaku.
///
/// Header is one line — `[Lv N] Name: description` with chips/mult on the
/// right — then the canonical hand tiles directly beneath.
#[allow(clippy::too_many_arguments)]
fn draw_plaque(
    frame: &mut UiFrame,
    placements: &mut Vec<ShowcaseTilePlacement>,
    tile_id: &mut u32,
    yk: YakuKind,
    state: ProgressionState,
    top_y: f32,
    plaque_h: f32,
    w: f32,
    h: f32,
    jr: f32,
    jc: f32,
    ctx: &DrawCtx<'_>,
) {
    let plaque_x = w * (0.055 - 0.027 * jc);
    let plaque_w = w * (0.89 + 0.054 * jc);
    let plaque_y = top_y;

    // Drop shadow — warmer brown tint (not pure black), bigger offset,
    // double-layered for softness. Gives the plaque a clearer sense of
    // floating above the wood rather than being painted on it.
    let shadow_scale = (h / 1080.0).max(1.0);
    let shadow_warm = color::WALNUT_DEEP;
    // Far, soft shadow.
    frame.quad(GpuInstance {
        rect: [
            plaque_x - 4.0 * shadow_scale,
            plaque_y + 16.0 * shadow_scale,
            plaque_w + 8.0 * shadow_scale,
            plaque_h,
        ],
        color: color::alpha(shadow_warm, 0.35),
        user: 0,
    });
    // Near, crisp shadow.
    frame.quad(GpuInstance {
        rect: [
            plaque_x + 3.0 * shadow_scale,
            plaque_y + 9.0 * shadow_scale,
            plaque_w,
            plaque_h,
        ],
        color: color::alpha(shadow_warm, 0.55),
        user: 0,
    });
    // Brass outer rim.
    frame.quad(GpuInstance {
        rect: [plaque_x, plaque_y, plaque_w, plaque_h],
        color: color::ANTIQUE,
        user: 0,
    });
    // Bevel highlight.
    let bevel = 2.0 * shadow_scale;
    frame.quad(GpuInstance {
        rect: [
            plaque_x + bevel,
            plaque_y + bevel,
            plaque_w - bevel * 2.0,
            plaque_h - bevel * 2.0,
        ],
        color: color::BRASS,
        user: 0,
    });
    // Walnut lacquer face fills the plaque interior.
    let pad_scale = 1.0 - 0.14 * jc;
    let pad = ((14.0 * shadow_scale).max(10.0)) * pad_scale;
    let face_x = plaque_x + pad;
    let face_y = plaque_y + pad;
    let face_w = plaque_w - pad * 2.0;
    let face_h = plaque_h - pad * 2.0;
    // Deep brown lacquer to match the global white-on-dark-walnut theme.
    let bamboo_face = color::WALNUT_DEEP;
    frame.quad(GpuInstance {
        rect: [face_x, face_y, face_w, face_h],
        color: bamboo_face,
        user: 0,
    });
    let label_champagne = color::CHAMPAGNE;
    let label_champagne_soft = color::alpha(color::CHAMPAGNE, 0.88);
    let label_champagne_muted = color::alpha(color::CHAMPAGNE, 0.52);

    let yaku_progress = GameEngine::read_yaku_progress(ctx.run);
    let lvl = yaku_progress.level_of(yk);
    let chips = yk.chip_bonus_at(lvl);
    let mult = yk.mult_bonus_at(lvl);

    // ── Header — one line: [Lv N] Name: description ··· payout ──
    let header_pad = ((18.0 * shadow_scale).max(12.0)) * pad_scale;
    let header_x = face_x + header_pad;
    let header_w = face_w - header_pad * 2.0;
    let header_y = face_y + header_pad * 0.6;

    let line_font = typography::size(typography::H28, h);
    let header_line_h = line_font * 1.15;
    let inline_gap = header_pad * 0.35;

    let stat_font = typography::size(typography::H24, h);
    let stat_band_h = stat_font * 1.2;
    let stat_font_px = typography::size(typography::H32, h);
    let payout_reserve = match state {
        ProgressionState::Unseen => measure_ui_text_width("— — —", stat_font_px),
        _ => chips_mult_stat_segments(chips, mult)
            .iter()
            .map(|(t, _)| measure_payout_text_width(t, stat_font_px))
            .sum(),
    };
    let payout_gap = inline_gap;
    let text_right = header_x + header_w - payout_reserve - payout_gap;

    // Compact level pill — inline at the start of the header row.
    let pill_font = typography::size(typography::H32, h);
    let pill_label = format!("Lv {lvl}");
    let pill_text_w = measure_ui_text_width(&pill_label, pill_font);
    let pill_pad_x = pill_font * 0.55;
    let pill_w = pill_text_w + pill_pad_x * 2.0;
    let pill_h = header_line_h * 0.82;
    let pill_x = header_x;
    let pill_y = header_y + (header_line_h - pill_h) * 0.5;
    let badge_ink = color::darken(color::WALNUT_INK, 0.12);
    let (pill_bg, pill_fg) = match state {
        ProgressionState::Leveled => (color::darken(color::GOLD, 0.06), badge_ink),
        ProgressionState::Unseen => (color::darken(bamboo_face, 0.28), label_champagne_soft),
        ProgressionState::Played => (color::darken(color::BRASS, 0.07), badge_ink),
    };
    frame.quad(GpuInstance {
        rect: [
            pill_x + 1.5 * shadow_scale,
            pill_y + 2.0 * shadow_scale,
            pill_w,
            pill_h,
        ],
        color: color::alpha([0.08, 0.04, 0.02, 1.0], 0.35),
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [pill_x, pill_y, pill_w, pill_h],
        color: pill_bg,
        user: 0,
    });
    frame.text(TextLabel {
        rect: [pill_x, pill_y + pill_h * 0.14, pill_w, pill_h * 0.82],
        text: pill_label,
        color: pill_fg,
        align: TextAlign::Center,
        font_px: Some(pill_font),
        ..Default::default()
    });

    let (desc_text, groups) = super::guide::yaku_page(yk);
    let body_text: String = match state {
        ProgressionState::Unseen => "sealed — score this yaku to reveal its shape".into(),
        _ => desc_text.into(),
    };

    let mut text_x = pill_x + pill_w + inline_gap;
    let text_clip = [
        text_x,
        header_y,
        (text_right - text_x).max(0.0),
        header_line_h,
    ];

    if matches!(state, ProgressionState::Unseen) {
        let mystery_glyph = line_font * 1.02;
        let mystery_pill_w = mystery_glyph * 2.38;
        let mystery_pill_h = header_line_h * 0.88;
        draw_mystery_name_pill(
            frame,
            MysteryNamePillStyle {
                pill_center_x: text_x + mystery_pill_w * 0.5,
                top_y: header_y + (header_line_h - mystery_pill_h) * 0.5,
                pill_h: mystery_pill_h,
                font_px: mystery_glyph,
                shadow_scale,
                pill_bg: color::darken(bamboo_face, 0.28),
                text_color: label_champagne_soft,
            },
        );
        text_x += mystery_pill_w + inline_gap * 0.35;
    } else {
        let name_w = measure_ui_text_width(yk.name(), line_font).min(text_right - text_x);
        frame.text(TextLabel {
            rect: [text_x, header_y, name_w, header_line_h],
            text: yk.name().into(),
            color: label_champagne,
            align: TextAlign::Left,
            font_px: Some(line_font),
            clip_rect: Some(text_clip),
            ..Default::default()
        });
        text_x += name_w;
    }

    let desc_x = text_x;
    let desc_w = (text_right - desc_x).max(0.0);
    frame.text(TextLabel {
        rect: [desc_x, header_y, desc_w, header_line_h],
        text: format!(": {body_text}"),
        color: label_champagne_soft,
        align: TextAlign::Left,
        font_px: Some(line_font),
        clip_rect: Some([desc_x, header_y, desc_w, header_line_h]),
        ..Default::default()
    });

    let stat_y = header_y + (header_line_h - stat_band_h) * 0.5;
    match state {
        ProgressionState::Unseen => {
            frame.text(TextLabel {
                rect: [
                    header_x + header_w - payout_reserve,
                    stat_y,
                    payout_reserve,
                    stat_band_h,
                ],
                text: "— — —".into(),
                color: label_champagne_muted,
                align: TextAlign::Right,
                font_px: Some(stat_font_px),
                ..Default::default()
            });
        }
        _ => {
            push_chips_mult_stat_line_right(
                frame,
                header_x + header_w,
                stat_y,
                stat_band_h,
                stat_font_px,
                chips,
                mult,
            );
        }
    }

    // ── Canonical hand (or sealed placeholder) ───────────────────
    let hand_tiles: Vec<Tile> = groups
        .iter()
        .flat_map(|g| g.tiles.iter().copied())
        .collect();
    if hand_tiles.is_empty() {
        return;
    }

    let hand_top = header_y + header_line_h + header_pad * 0.35;
    let hand_bot = face_y + face_h - header_pad * 0.2;
    let hand_band_h = (hand_bot - hand_top).max(0.0);

    let num_gaps = groups.len().saturating_sub(1);
    let total_tiles = hand_tiles.len();
    let gap_equiv = num_gaps as f32 * 0.5;

    const FACE_LONG_MAX: f32 = 1.5;
    let max_tile_w = (face_w - header_pad * 2.0) / (total_tiles as f32 + gap_equiv);
    let max_tile_h = hand_band_h / FACE_LONG_MAX;
    let hand_tile = max_tile_w
        .min(max_tile_h)
        .max((36.0 + 2.0 * (1.0 - jc)) * jr);
    let hand_gap = hand_tile * 0.5;

    let hand_total_w = total_tiles as f32 * hand_tile + num_gaps as f32 * hand_gap;
    let hand_x0 = face_x + (face_w - hand_total_w) * 0.5;
    let hand_cy = hand_top + hand_band_h * 0.5;

    if matches!(state, ProgressionState::Unseen) {
        // Sealed tablet across the hand band instead of ghostly tiles.
        let band_x = face_x + header_pad * 0.5;
        let band_w = face_w - header_pad;
        draw_sealed_slab(
            frame,
            band_x,
            hand_cy - hand_tile * FACE_LONG_MAX * 0.5,
            band_w,
            hand_tile * FACE_LONG_MAX,
            h,
            jr,
        );
    } else {
        let mut cursor_x = hand_x0;
        for group in &groups {
            for tile in &group.tiles {
                // Re-id the tile so the scene's placement batch has unique
                // ids (yaku_page uses per-group 0..N which would collide
                // across groups in one draw batch).
                let t = Tile::new(tile.suit, tile.rank, *tile_id);
                *tile_id += 1;
                let cx = cursor_x + hand_tile * 0.5;
                let (brightness, hand_glow, hand_glow_color) = match state {
                    ProgressionState::Played => (1.0, false, None),
                    ProgressionState::Leveled => (1.0, true, Some(color::alpha(color::GOLD, 0.7))),
                    ProgressionState::Unseen => (1.0, false, None), // unreachable
                };
                placements.push(ShowcaseTilePlacement {
                    tile: t,
                    center_pos: [cx, hand_cy, 0.0],
                    rotation: [0.0, 0.0, std::f32::consts::PI],
                    scale: 1.0,
                    size_px: hand_tile,
                    brightness,
                    opacity: 1.0,
                    selected: false,
                    hovered: false,
                    outline: false,
                    glow: hand_glow,
                    glow_color: hand_glow_color,
                    outline_sel: None,
                    pick_id: None,
                    overlay_rect_group: None,
                });
                cursor_x += hand_tile;
            }
            cursor_x += hand_gap;
        }
    }
}
