//! Yaku Journal — pushdown scene. Tiles on the table.
//!
//! Replaces the old parchment overlay with its own scene: the player steps
//! *onto* the table rather than into a book. Each yaku is rendered as a
//! signature-tile icon in a grid on the lacquered wood surface; the focused
//! yaku's full canonical 14-tile hand floats above the grid on a plaque,
//! drawn from the same `yaku_page()` data the meld guide teaches with (so
//! the plaque hand is guaranteed to score as its named yaku — see the
//! scoring test in meld_guide).

use crate::core::tile::{Suit, Tile};
use crate::core::yaku::YakuKind;
use crate::render::draw_cmd::{CameraParams, ShowcaseTilePlacement, UiFrame};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::ui::input::UiAction;

use super::{BackgroundId, DrawCtx, OverlayRequest, SceneBehavior, SceneTransition, UpdateCtx};

/// 5 / 4 / 4 grid — 13 yaku laid out so no row looks stranded. Row index
/// per yaku, in the order `YakuKind::all()` returns them. Keeping this as
/// a const lets the `update` selection navigation share the same structure
/// as the `draw_frame` layout without re-deriving it.
const ROW_COUNTS: [usize; 3] = [5, 4, 4];

pub struct YakuJournalScene {
    /// Index into `YakuKind::all()` of the currently-focused yaku.
    selected: usize,
}

impl YakuJournalScene {
    pub fn new() -> Self {
        Self { selected: 0 }
    }

    /// Convert a flat yaku index (0..13) into `(row, col)` in the grid.
    fn index_to_grid(i: usize) -> (usize, usize) {
        let mut remaining = i;
        for (row, &n) in ROW_COUNTS.iter().enumerate() {
            if remaining < n {
                return (row, remaining);
            }
            remaining -= n;
        }
        // Clamp out-of-range to the last cell.
        let row = ROW_COUNTS.len() - 1;
        (row, ROW_COUNTS[row] - 1)
    }

    /// Convert `(row, col)` back into a flat index. Clamps col into the
    /// target row's width.
    fn grid_to_index(row: usize, col: usize) -> usize {
        let row = row.min(ROW_COUNTS.len() - 1);
        let col = col.min(ROW_COUNTS[row] - 1);
        ROW_COUNTS.iter().take(row).sum::<usize>() + col
    }

    fn move_focus(&mut self, drow: isize, dcol: isize) {
        let (row, col) = Self::index_to_grid(self.selected);
        let new_row = ((row as isize + drow)
            .rem_euclid(ROW_COUNTS.len() as isize)) as usize;
        let target_col = if drow != 0 {
            col.min(ROW_COUNTS[new_row] - 1)
        } else {
            let width = ROW_COUNTS[new_row] as isize;
            ((col as isize + dcol).rem_euclid(width)) as usize
        };
        self.selected = Self::grid_to_index(new_row, target_col);
    }
}

impl SceneBehavior for YakuJournalScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        for a in ctx.actions {
            match a {
                UiAction::Cancel | UiAction::Pause | UiAction::Help => {
                    *ctx.overlay_request = Some(OverlayRequest::Pop);
                    return None;
                }
                UiAction::FocusPrev => self.move_focus(0, -1),
                UiAction::FocusNext => self.move_focus(0, 1),
                UiAction::FocusUp => self.move_focus(-1, 0),
                UiAction::FocusDown => self.move_focus(1, 0),
                _ => {}
            }
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let ui_scale = ctx.ui_scale;
        let run = ctx.run;

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.table();

        // Camera — directly top-down so pixel-space layout and the
        // projected tile positions stay 1:1 with each other, letting
        // the grid math place captions without projection offset.
        let cam_scale = h / 1600.0;
        frame.camera_override = Some(CameraParams {
            eye: [0.0, 0.0, 2040.0 * cam_scale],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            fovy_deg: 45.0,
        });

        // One soft high fill light. The previous two-light setup created
        // bright specular blooms on the wood that pulled the eye away
        // from the grid; a single, very high, wide-radius light gives
        // tiles enough dimensional shading without hotspots.
        frame.point_lights.push(PointLight {
            pos: [w * 0.5, h * -0.10, h * 1.40],
            radius: h * 3.0,
            color: [1.0, 0.96, 0.88],
            intensity: 1.2,
        });

        // No English "Yaku Journal" title — the player just pressed the
        // journal button, they know where they are, and the space is
        // better used letting the grid breathe.

        // ── Grid metrics ─────────────────────────────────────────
        let yaku = YakuKind::all();
        // Reserve the bottom portion of the screen for the floating
        // plaque. The grid fills the upper two-thirds.
        let plaque_top = h * 0.62;
        let grid_top = h * 0.04;
        let grid_bot = plaque_top - h * 0.02;
        let grid_h = grid_bot - grid_top;
        let row_h = grid_h / ROW_COUNTS.len() as f32;

        let side_margin = w * 0.04;
        // Cells are sized by the widest row so every row's cells match
        // width regardless of count — row 1 (5 wide) defines the cell.
        let cell_w = (w - side_margin * 2.0) / 5.0;

        let name_font = typography::size(typography::HEADING, h, ui_scale).max(22.0);
        let stat_font = typography::size(typography::BODY, h, ui_scale).max(18.0);
        let name_h = name_font * 1.1;
        let stat_h = stat_font * 1.1;
        let caption_block_h = name_h + stat_h;

        // Tile sizing. `size_px` on `ShowcaseTilePlacement` is the tile's
        // *short* edge; the long edge (up to ~1.5× for Chinese preset) is
        // what actually consumes vertical space on a top-down view. So
        // each row must budget `tile_size * FACE_LONG_MAX` of vertical
        // space plus the caption block above — we use the conservative
        // Chinese ratio to keep layout correct across all tile presets.
        const FACE_LONG_MAX: f32 = 1.5;
        let tiles_per_cell = 3usize;
        let tile_gap_frac = 0.15;
        let tile_divisor =
            tiles_per_cell as f32 + tile_gap_frac * (tiles_per_cell - 1) as f32;
        let max_tile_from_cell_w = cell_w * 0.75 / tile_divisor;
        let row_budget_v = (row_h * 0.85 - caption_block_h) / FACE_LONG_MAX;
        let tile_size = max_tile_from_cell_w.min(row_budget_v).max(32.0);
        let tile_gap = tile_size * tile_gap_frac;
        let tile_long_h = tile_size * FACE_LONG_MAX;

        // ── Grid draw ────────────────────────────────────────────
        let mut placements: Vec<ShowcaseTilePlacement> = Vec::new();
        let mut tile_id: u32 = 0;

        let mut yi = 0usize;
        for (row_i, &row_n) in ROW_COUNTS.iter().enumerate() {
            let row_w = cell_w * row_n as f32;
            let row_x0 = (w - row_w) * 0.5;
            let row_top = grid_top + row_h * row_i as f32;
            let caption_y = row_top + row_h * 0.06;
            // Tile center sits a full long-half below the caption block
            // so the tile's projected top edge doesn't encroach on the
            // caption above it.
            let tile_cy = caption_y + caption_block_h + tile_long_h * 0.55;

            for col_i in 0..row_n {
                if yi >= yaku.len() {
                    break;
                }
                let yk = yaku[yi];
                let is_selected = yi == self.selected;
                yi += 1;

                let state = progression_state(run, yk);
                let cell_cx = row_x0 + cell_w * (col_i as f32 + 0.5);
                let strip_w =
                    tiles_per_cell as f32 * tile_size + (tiles_per_cell - 1) as f32 * tile_gap;
                let strip_x0 = cell_cx - strip_w * 0.5;

                // Selection backing.
                if is_selected {
                    let pad_x = cell_w * 0.04;
                    let bg_x = cell_cx - cell_w * 0.5 + pad_x;
                    let bg_y = caption_y - row_h * 0.04;
                    let bg_w = cell_w - pad_x * 2.0;
                    let bg_h = row_h * 0.88;
                    frame.quad(GpuInstance {
                        rect: [bg_x, bg_y, bg_w, bg_h],
                        color: color::alpha(color::CHAMPAGNE, 0.18),
                    });
                }

                // Name — unseen yaku read as dimmer parchment.
                let name_color = match state {
                    ProgressionState::Unseen => color::alpha(color::PARCHMENT, 0.55),
                    _ if is_selected => color::CHAMPAGNE,
                    _ => color::PARCHMENT,
                };
                frame.text(TextLabel {
                    rect: [cell_cx - cell_w * 0.5, caption_y, cell_w, name_h],
                    text: yk.name().into(),
                    color: name_color,
                    align: TextAlign::Center,
                    font_px: Some(name_font),
                    ..Default::default()
                });

                // Level badge (or "sealed" marker for unseen).
                let lvl = run.yaku_levels.level_of(yk);
                let (level_text, level_color) = match state {
                    ProgressionState::Unseen => {
                        ("sealed".into(), color::alpha(color::CHAMPAGNE, 0.6))
                    }
                    ProgressionState::Leveled => (format!("Lv {lvl}"), color::GOLD),
                    _ => (format!("Lv {lvl}"), color::CHAMPAGNE),
                };
                frame.text(TextLabel {
                    rect: [cell_cx - cell_w * 0.5, caption_y + name_h, cell_w, stat_h],
                    text: level_text,
                    color: level_color,
                    align: TextAlign::Center,
                    font_px: Some(stat_font),
                    ..Default::default()
                });

                match state {
                    ProgressionState::Unseen => {
                        // Sealed tablet in place of the tile strip:
                        // dark slab + wax seal. Reads unambiguously
                        // as "locked / unknown" without the dying-
                        // bulb effect that dimmed-tile rendering had
                        // (the suit color still bleeds through
                        // `brightness`, so silhouettes looked broken
                        // rather than concealed).
                        draw_sealed_slab(
                            &mut frame,
                            strip_x0,
                            tile_cy - tile_long_h * 0.5,
                            strip_w,
                            tile_long_h,
                            h,
                            ui_scale,
                        );
                    }
                    ProgressionState::Played | ProgressionState::Leveled => {
                        let sig = signature_tiles(yk, &mut tile_id);
                        for (i, tile) in sig.iter().enumerate() {
                            let cx = strip_x0
                                + tile_size * 0.5
                                + i as f32 * (tile_size + tile_gap);
                            let (brightness, tile_glow, tile_glow_color) = match state {
                                ProgressionState::Played => (
                                    if is_selected { 1.0 } else { 0.85 },
                                    is_selected,
                                    None,
                                ),
                                ProgressionState::Leveled => (
                                    1.0,
                                    true,
                                    Some(color::alpha(
                                        color::GOLD,
                                        if is_selected { 0.9 } else { 0.55 },
                                    )),
                                ),
                                // Unreachable — Unseen is handled above.
                                ProgressionState::Unseen => (1.0, false, None),
                            };
                            placements.push(ShowcaseTilePlacement {
                                tile: *tile,
                                center_pos: [cx, tile_cy, 0.0],
                                rotation: [0.0, 0.0, std::f32::consts::PI],
                                scale: 1.0,
                                size_px: tile_size,
                                brightness,
                                selected: false,
                                hovered: false,
                                outline: is_selected,
                                glow: tile_glow,
                                glow_color: tile_glow_color,
                                pick_id: None,
                            });
                        }
                    }
                }
            }
        }

        // ── Floating plaque for the selected yaku ────────────────
        let sel_yk = yaku[self.selected];
        let sel_state = progression_state(run, sel_yk);
        draw_plaque(
            &mut frame,
            &mut placements,
            &mut tile_id,
            sel_yk,
            run.yaku_levels.level_of(sel_yk),
            sel_state,
            plaque_top,
            w,
            h,
            ui_scale,
        );

        if !placements.is_empty() {
            frame.showcase_tile_batch(placements);
        }

        frame
    }
}

/// Draw a "sealed" tablet where a tile strip would otherwise go: a dark
/// slab with a red wax-seal disc in the center stamped with `?`. This is
/// the grid-cell treatment for undiscovered yaku — unambiguous lock state
/// that reads clearly at TV distance without accidentally looking broken.
fn draw_sealed_slab(
    frame: &mut UiFrame,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    window_h: f32,
    ui_scale: f32,
) {
    // Dark obsidian slab — matches the "this is sealed" vocabulary
    // (same ink color used by scoring panel frames).
    let inset = (2.0 * (window_h / 1080.0)).max(1.0);
    frame.quad(GpuInstance {
        rect: [x, y, w, h],
        color: color::darken(color::ANTIQUE, 0.7),
    });
    frame.quad(GpuInstance {
        rect: [x + inset, y + inset, w - inset * 2.0, h - inset * 2.0],
        color: color::OBSIDIAN,
    });

    // Wax seal — two concentric red quads as a disc surrogate. Not a
    // true circle but reads as "round stamp" at distance, especially
    // with the smaller inner highlight disc.
    let seal_d = h.min(w * 0.35) * 0.68;
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    // Outer wax ring.
    frame.quad(GpuInstance {
        rect: [cx - seal_d * 0.5, cy - seal_d * 0.5, seal_d, seal_d],
        color: color::darken(color::RUBY, 0.35),
    });
    // Inner wax body — bright red.
    let inner_d = seal_d * 0.82;
    frame.quad(GpuInstance {
        rect: [
            cx - inner_d * 0.5,
            cy - inner_d * 0.5,
            inner_d,
            inner_d,
        ],
        color: color::RUBY,
    });
    // "?" stamp on the wax.
    let glyph_font = typography::size(typography::HEADING, window_h, ui_scale).max(20.0);
    frame.text(TextLabel {
        rect: [cx - seal_d * 0.5, cy - glyph_font * 0.5, seal_d, glyph_font * 1.0],
        text: "?".into(),
        color: color::CHAMPAGNE,
        align: TextAlign::Center,
        font_px: Some(glyph_font * 1.3),
        ..Default::default()
    });
}

/// Progression state for one yaku. Drives the grid material cues and the
/// plaque's reveal/veil decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgressionState {
    /// Never scored (`yaku_times_played == 0`) and never leveled (Lv 1).
    /// Rendered as a sealed tablet so first-time discovery stays a moment.
    Unseen,
    /// Scored at least once, still at base level.
    Played,
    /// Zodiac-leveled to 2 or above. Gets a gold glow.
    Leveled,
}

fn progression_state(run: &crate::game::run::RunState, yk: YakuKind) -> ProgressionState {
    let lvl = run.yaku_levels.level_of(yk);
    let played = run.yaku_times_played.get(&yk).copied().unwrap_or(0);
    if lvl >= 2 {
        ProgressionState::Leveled
    } else if played >= 1 {
        ProgressionState::Played
    } else {
        ProgressionState::Unseen
    }
}

/// Draw the floating plaque: a parchment panel across the bottom of the
/// screen showing the selected yaku's canonical 14-tile hand, scoring
/// values, name, and description. The hand comes from
/// `super::meld_guide::yaku_page`, validated by the scorer test in that
/// module — so whatever renders here is guaranteed to score as the named
/// yaku.
///
/// Header hierarchy is **score-first**: big numbers on the left
/// (`+N MULT / +N CHIPS` stacked), name and level on the right as a
/// subtitle. A player browsing the journal to decide where to spend a
/// Zodiac card wants to compare score values across yaku, so those get
/// the display weight. Identity (name) still reads clearly but doesn't
/// compete.
#[allow(clippy::too_many_arguments)]
fn draw_plaque(
    frame: &mut UiFrame,
    placements: &mut Vec<ShowcaseTilePlacement>,
    tile_id: &mut u32,
    yk: YakuKind,
    lvl: u32,
    state: ProgressionState,
    top_y: f32,
    w: f32,
    h: f32,
    ui_scale: f32,
) {
    let plaque_x = w * 0.05;
    let plaque_w = w * 0.90;
    let plaque_h = h * 0.36;
    let plaque_y = top_y;

    // Drop shadow — warmer brown tint (not pure black), bigger offset,
    // double-layered for softness. Gives the plaque a clearer sense of
    // floating above the wood rather than being painted on it.
    let shadow_scale = (h / 1080.0).max(1.0);
    let shadow_warm = [0.08, 0.04, 0.02, 1.0];
    // Far, soft shadow.
    frame.quad(GpuInstance {
        rect: [
            plaque_x - 4.0 * shadow_scale,
            plaque_y + 16.0 * shadow_scale,
            plaque_w + 8.0 * shadow_scale,
            plaque_h,
        ],
        color: color::alpha(shadow_warm, 0.35),
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
    });
    // Brass outer rim.
    frame.quad(GpuInstance {
        rect: [plaque_x, plaque_y, plaque_w, plaque_h],
        color: color::ANTIQUE,
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
    });
    // Parchment face.
    let pad = (14.0 * shadow_scale).max(10.0);
    let face_x = plaque_x + pad;
    let face_y = plaque_y + pad;
    let face_w = plaque_w - pad * 2.0;
    let face_h = plaque_h - pad * 2.0;
    frame.quad(GpuInstance {
        rect: [face_x, face_y, face_w, face_h],
        color: color::PARCHMENT,
    });

    // ── Header ───────────────────────────────────────────────────
    // Left column: big score values — MULT on top, CHIPS below.
    // Right column: yaku name + level as subtitle.
    let header_pad = (18.0 * shadow_scale).max(12.0);
    let header_x = face_x + header_pad;
    let header_w = face_w - header_pad * 2.0;
    let header_y = face_y + header_pad * 0.4;

    let mult = yk.mult_bonus_at(lvl);
    let chip = yk.chip_bonus_at(lvl);

    let score_font = typography::size(typography::DISPLAY, h, ui_scale).max(44.0);
    let score_label_font = typography::size(typography::CAPTION, h, ui_scale).max(16.0);
    let score_line_h = score_font * 1.05;

    // Score column: two stacked "+N LABEL" rows, numbers large.
    let mult_text = format!("+{mult}");
    let chip_text = format!("+{chip}");
    // Big number — MULT.
    frame.text(TextLabel {
        rect: [header_x, header_y, header_w * 0.48, score_line_h],
        text: mult_text,
        color: color::OBSIDIAN,
        align: TextAlign::Left,
        font_px: Some(score_font),
        ..Default::default()
    });
    // Small "MULT" caption hanging off the number.
    let mult_caption_x = header_x + score_font * 2.2;
    frame.text(TextLabel {
        rect: [mult_caption_x, header_y + score_font * 0.25, header_w * 0.3, score_label_font * 1.4],
        text: "MULT".into(),
        color: color::darken(color::ANTIQUE, 0.15),
        align: TextAlign::Left,
        font_px: Some(score_label_font),
        ..Default::default()
    });
    // Big number — CHIPS.
    let chip_row_y = header_y + score_line_h * 0.92;
    frame.text(TextLabel {
        rect: [header_x, chip_row_y, header_w * 0.48, score_line_h],
        text: chip_text,
        color: color::OBSIDIAN,
        align: TextAlign::Left,
        font_px: Some(score_font),
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: [mult_caption_x, chip_row_y + score_font * 0.25, header_w * 0.3, score_label_font * 1.4],
        text: "CHIPS".into(),
        color: color::darken(color::ANTIQUE, 0.15),
        align: TextAlign::Left,
        font_px: Some(score_label_font),
        ..Default::default()
    });

    // Right column: name as subtitle + level badge underneath.
    let name_font = typography::size(typography::HEADING, h, ui_scale).max(26.0);
    let name_h = name_font * 1.1;
    frame.text(TextLabel {
        rect: [header_x, header_y + score_font * 0.15, header_w, name_h],
        text: yk.name().into(),
        color: color::darken(color::ANTIQUE, 0.2),
        align: TextAlign::Right,
        font_px: Some(name_font),
        ..Default::default()
    });
    let level_badge_font = typography::size(typography::BODY, h, ui_scale).max(20.0);
    let level_badge_text = match state {
        ProgressionState::Leveled => format!("Lv {lvl}"),
        _ => format!("Lv {lvl}"),
    };
    let level_badge_color = match state {
        ProgressionState::Leveled => color::GOLD,
        _ => color::darken(color::ANTIQUE, 0.1),
    };
    frame.text(TextLabel {
        rect: [
            header_x,
            header_y + score_font * 0.15 + name_h,
            header_w,
            level_badge_font * 1.4,
        ],
        text: level_badge_text,
        color: level_badge_color,
        align: TextAlign::Right,
        font_px: Some(level_badge_font),
        ..Default::default()
    });

    // ── Description ──────────────────────────────────────────────
    let desc_font = typography::size(typography::BODY, h, ui_scale).max(20.0);
    let desc_h = desc_font * 1.4;
    let desc_y = chip_row_y + score_line_h + header_pad * 0.2;
    let (desc_text, groups) = super::meld_guide::yaku_page(yk);
    let body_text: String = match state {
        ProgressionState::Unseen => {
            "sealed — score this yaku to reveal its shape".into()
        }
        _ => desc_text.into(),
    };
    frame.text(TextLabel {
        rect: [header_x, desc_y, header_w, desc_h],
        text: body_text,
        color: color::darken(color::ANTIQUE, 0.1),
        align: TextAlign::Left,
        font_px: Some(desc_font),
        ..Default::default()
    });

    // ── Canonical hand (or sealed placeholder) ───────────────────
    let hand_tiles: Vec<Tile> = groups
        .iter()
        .flat_map(|g| g.tiles.iter().copied())
        .collect();
    if hand_tiles.is_empty() {
        return;
    }

    let hand_top = desc_y + desc_h + header_pad * 0.35;
    // Leave a single-line band at the bottom of the plaque for the
    // tucked-in control hint so it doesn't float orphaned below.
    let hint_font = typography::size(typography::CAPTION, h, ui_scale).max(14.0);
    let hint_band = hint_font * 1.4;
    let hand_bot = face_y + face_h - header_pad * 0.4 - hint_band;
    let hand_band_h = (hand_bot - hand_top).max(0.0);

    let num_gaps = groups.len().saturating_sub(1);
    let total_tiles = hand_tiles.len();
    let gap_equiv = num_gaps as f32 * 0.5;

    const FACE_LONG_MAX: f32 = 1.5;
    let max_tile_w = (face_w - header_pad * 2.0) / (total_tiles as f32 + gap_equiv);
    let max_tile_h = hand_band_h / FACE_LONG_MAX;
    let hand_tile = max_tile_w.min(max_tile_h).max(32.0);
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
            ui_scale,
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
                    ProgressionState::Leveled => {
                        (1.0, true, Some(color::alpha(color::GOLD, 0.7)))
                    }
                    ProgressionState::Unseen => (1.0, false, None), // unreachable
                };
                placements.push(ShowcaseTilePlacement {
                    tile: t,
                    center_pos: [cx, hand_cy, 0.0],
                    rotation: [0.0, 0.0, std::f32::consts::PI],
                    scale: 1.0,
                    size_px: hand_tile,
                    brightness,
                    selected: false,
                    hovered: false,
                    outline: false,
                    glow: hand_glow,
                    glow_color: hand_glow_color,
                    pick_id: None,
                });
                cursor_x += hand_tile;
            }
            cursor_x += hand_gap;
        }
    }

    // ── Control hint, tucked inside the plaque's bottom edge ────
    frame.text(TextLabel {
        rect: [
            header_x,
            face_y + face_h - hint_band - header_pad * 0.3,
            header_w,
            hint_band,
        ],
        text: "← → ↑ ↓ browse   ·   Esc to return".into(),
        color: color::alpha(color::darken(color::ANTIQUE, 0.1), 0.7),
        align: TextAlign::Right,
        font_px: Some(hint_font),
        ..Default::default()
    });
}

/// Three signature tiles that communicate the essence of a yaku at grid-icon
/// size. These are intentionally a condensed shorthand, not a full hand —
/// the floating plaque carries the scorer-validated 14-tile hand. The grid
/// icon just needs to be scannable at distance so the player can navigate
/// the collection.
fn signature_tiles(yk: YakuKind, id: &mut u32) -> Vec<Tile> {
    let specs: &[(Suit, u8)] = match yk {
        YakuKind::Tanyao => &[(Suit::Characters, 4), (Suit::Bamboos, 5), (Suit::Circles, 7)],
        YakuKind::Toitoi => &[(Suit::Circles, 5), (Suit::Circles, 5), (Suit::Circles, 5)],
        YakuKind::Honroutou => &[(Suit::Characters, 1), (Suit::Bamboos, 9), (Suit::Dragon, 1)],
        YakuKind::Iipeikou => &[(Suit::Bamboos, 1), (Suit::Bamboos, 2), (Suit::Bamboos, 3)],
        YakuKind::FullHand => &[
            (Suit::Characters, 3),
            (Suit::Characters, 4),
            (Suit::Characters, 5),
        ],
        YakuKind::Chinitsu => &[(Suit::Bamboos, 2), (Suit::Bamboos, 5), (Suit::Bamboos, 8)],
        YakuKind::SanshokuDoujun => {
            &[(Suit::Characters, 5), (Suit::Bamboos, 5), (Suit::Circles, 5)]
        }
        YakuKind::Junchan => &[(Suit::Circles, 1), (Suit::Circles, 2), (Suit::Circles, 3)],
        YakuKind::Ittsu => &[(Suit::Bamboos, 1), (Suit::Bamboos, 5), (Suit::Bamboos, 9)],
        YakuKind::Honitsu => &[(Suit::Bamboos, 3), (Suit::Bamboos, 7), (Suit::Wind, 1)],
        YakuKind::Yakuhai => &[(Suit::Dragon, 1), (Suit::Dragon, 1), (Suit::Dragon, 1)],
        YakuKind::Chiitoitsu => &[
            (Suit::Characters, 1),
            (Suit::Characters, 1),
            (Suit::Bamboos, 5),
        ],
        YakuKind::ChickenHand => &[
            (Suit::Characters, 1),
            (Suit::Characters, 2),
            (Suit::Characters, 3),
        ],
    };

    specs
        .iter()
        .map(|&(suit, rank)| {
            let tile = Tile::new(suit, rank, *id);
            *id += 1;
            tile
        })
        .collect()
}
