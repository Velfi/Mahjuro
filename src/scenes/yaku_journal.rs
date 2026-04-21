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
        let progress = ctx.progress;

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
        // plaque. Grid sinks toward center (top at 7%, bottom at 66%)
        // so rows breathe instead of hugging the window edge; plaque
        // shrinks to ~30% of screen height, freeing vertical air.
        let plaque_top = h * 0.64;
        let grid_top = h * 0.05;
        let grid_bot = plaque_top - h * 0.02;
        let grid_h = grid_bot - grid_top;
        let row_h = grid_h / ROW_COUNTS.len() as f32;

        let side_margin = w * 0.06;
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

                let state = progression_state(run, progress, yk);
                let cell_cx = row_x0 + cell_w * (col_i as f32 + 0.5);
                let strip_w =
                    tiles_per_cell as f32 * tile_size + (tiles_per_cell - 1) as f32 * tile_gap;
                let strip_x0 = cell_cx - strip_w * 0.5;

                // Discovered cells get a faint parchment-glow pad behind
                // the tile strip so unlocked yaku read as "lit lanterns"
                // on the table, differentiated from the sealed cards
                // around them. Selection adds a stronger champagne halo
                // on top of that, tightened to the tile envelope so
                // focus sits on the tiles, not a row-wide stripe.
                let pad_pad_y = tile_long_h * 0.18;
                let pad_pad_x = tile_size * 0.35;
                let pad_x0 = strip_x0 - pad_pad_x;
                let pad_y0 = tile_cy - tile_long_h * 0.5 - pad_pad_y;
                let pad_w = strip_w + pad_pad_x * 2.0;
                let pad_h = tile_long_h + pad_pad_y * 2.0;

                if matches!(state, ProgressionState::Played | ProgressionState::Leveled) {
                    frame.quad(GpuInstance {
                        rect: [pad_x0, pad_y0, pad_w, pad_h],
                        color: color::alpha(color::PARCHMENT, 0.06),
                    });
                }
                if is_selected {
                    // Stacked halo layers — outer soft pool, inner warm
                    // wash, then a crisp 1px brass rim. Against the dark
                    // lacquer of sealed cards the subtler version got lost,
                    // so we push opacity up and add a visible ring to
                    // anchor "this is the one you're reading."
                    let halo_pad = tile_long_h * 0.30;
                    frame.quad(GpuInstance {
                        rect: [
                            pad_x0 - halo_pad,
                            pad_y0 - halo_pad,
                            pad_w + halo_pad * 2.0,
                            pad_h + halo_pad * 2.0,
                        ],
                        color: color::alpha(color::CHAMPAGNE, 0.10),
                    });
                    let mid_pad = tile_long_h * 0.14;
                    frame.quad(GpuInstance {
                        rect: [
                            pad_x0 - mid_pad,
                            pad_y0 - mid_pad,
                            pad_w + mid_pad * 2.0,
                            pad_h + mid_pad * 2.0,
                        ],
                        color: color::alpha(color::CHAMPAGNE, 0.22),
                    });
                    // Crisp brass outline — 4-edge ring so focus reads
                    // even on dark sealed cards.
                    let ring_px = (2.0 * (h / 1080.0)).max(1.5);
                    let rx = pad_x0 - mid_pad;
                    let ry = pad_y0 - mid_pad;
                    let rw = pad_w + mid_pad * 2.0;
                    let rh = pad_h + mid_pad * 2.0;
                    let ring_color = color::alpha(color::GOLD, 0.85);
                    frame.quad(GpuInstance { rect: [rx, ry, rw, ring_px], color: ring_color });
                    frame.quad(GpuInstance { rect: [rx, ry + rh - ring_px, rw, ring_px], color: ring_color });
                    frame.quad(GpuInstance { rect: [rx, ry, ring_px, rh], color: ring_color });
                    frame.quad(GpuInstance { rect: [rx + rw - ring_px, ry, ring_px, rh], color: ring_color });
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

                // Level caption. Sealed cells get a thin en-dash —
                // the wax seal already says "locked," so a redundant
                // "sealed" word just creates visual noise on the row.
                let lvl = run.yaku_levels.level_of(yk);
                let (level_text, level_color) = match state {
                    ProgressionState::Unseen => (
                        "—".into(),
                        color::alpha(color::PARCHMENT, 0.35),
                    ),
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
        let sel_state = progression_state(run, progress, sel_yk);
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
    ui_scale: f32,
) {
    let scale = (window_h / 1080.0).max(1.0);
    let inset = (2.0 * scale).max(1.0);

    // Inked rim + deep-lacquer card face — darker than parchment so the
    // seal reads as a *locked chapter* rather than a waiting page, and
    // darker than the wood table so it stands apart from the background.
    // Color below is roughly darkened OBSIDIAN with a warm lift, chosen
    // by eye to contrast against both PARCHMENT and the wood grain.
    frame.quad(GpuInstance {
        rect: [x, y, w, h],
        color: color::darken(color::ANTIQUE, 0.75),
    });
    frame.quad(GpuInstance {
        rect: [x + inset, y + inset, w - inset * 2.0, h - inset * 2.0],
        color: [0.14, 0.10, 0.08, 1.0],
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
        color: color::alpha([0.05, 0.02, 0.01, 1.0], 0.55),
    });
    // Dark wax ring.
    frame.quad(GpuInstance {
        rect: [cx - seal_d * 0.5, cy - seal_d * 0.5, seal_d, seal_d],
        color: color::darken(color::RUBY, 0.5),
    });
    // Wax body.
    let body_d = seal_d * 0.86;
    frame.quad(GpuInstance {
        rect: [cx - body_d * 0.5, cy - body_d * 0.5, body_d, body_d],
        color: color::RUBY,
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
    });

    // "?" stamp — larger glyph, champagne ink so it reads as pressed
    // metal into the wax rather than a flat typeface.
    let glyph_font = typography::size(typography::TITLE, window_h, ui_scale).max(28.0);
    frame.text(TextLabel {
        rect: [cx - seal_d * 0.5, cy - glyph_font * 0.55, seal_d, glyph_font * 1.1],
        text: "?".into(),
        color: color::alpha(color::CHAMPAGNE, 0.92),
        align: TextAlign::Center,
        font_px: Some(glyph_font * 1.2),
        ..Default::default()
    });
}

/// Progression state for one yaku. Drives the grid material cues and the
/// plaque's reveal/veil decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgressionState {
    /// Never scored in any run (cumulative `PlayerProgress::yaku_times_scored`
    /// is zero) and never leveled. Rendered as a sealed tablet so first-time
    /// discovery stays a moment.
    Unseen,
    /// Scored at least once in any run's history, still at base level.
    Played,
    /// Zodiac-leveled to 2 or above. Gets a gold glow.
    Leveled,
}

/// Once a yaku has appeared in any round, it stays unlocked forever —
/// so the "played" check is against the cumulative
/// `PlayerProgress::yaku_times_scored` (persisted across runs), not the
/// per-run `RunState::yaku_times_played` which resets.
fn progression_state(
    run: &crate::game::run::RunState,
    progress: &crate::core::progression::PlayerProgress,
    yk: YakuKind,
) -> ProgressionState {
    let lvl = run.yaku_levels.level_of(yk);
    let scored_ever = progress.yaku_times_scored.get(&yk).copied().unwrap_or(0);
    let played_this_run = run.yaku_times_played.get(&yk).copied().unwrap_or(0);
    if lvl >= 2 {
        ProgressionState::Leveled
    } else if scored_ever >= 1 || played_this_run >= 1 {
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
/// Header hierarchy is **identity-first**: yaku name is the title on the
/// left with a brass level-pill tag; stat totals (`+N MULT · +N CHIPS`)
/// sit right-aligned on the same line. A thin antique rule separates
/// header from description. The control hint lives in a brass footer
/// strip inside the plaque's rim so it reads as an affordance rather
/// than orphaned caption text.
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
    let plaque_x = w * 0.06;
    let plaque_w = w * 0.88;
    let plaque_h = h * 0.34;
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
    // Parchment face — stops short of the bottom to leave room for the
    // brass footer strip (which carries the control hint).
    let pad = (14.0 * shadow_scale).max(10.0);
    let hint_font = typography::size(typography::CAPTION, h, ui_scale).max(14.0);
    let footer_h = hint_font * 2.2;
    let face_x = plaque_x + pad;
    let face_y = plaque_y + pad;
    let face_w = plaque_w - pad * 2.0;
    let face_h = plaque_h - pad - footer_h - bevel;
    frame.quad(GpuInstance {
        rect: [face_x, face_y, face_w, face_h],
        color: color::PARCHMENT,
    });

    // ── Header ───────────────────────────────────────────────────
    // Left: yaku name as the title + a brass level-badge pill to its
    // right. Right: stat strip "+N MULT · +N CHIPS". A thin antique
    // rule separates header from description.
    let header_pad = (18.0 * shadow_scale).max(12.0);
    let header_x = face_x + header_pad;
    let header_w = face_w - header_pad * 2.0;
    let header_y = face_y + header_pad * 0.6;

    let mult = yk.mult_bonus_at(lvl);
    let chip = yk.chip_bonus_at(lvl);

    // Title — yaku name in big, ink-dark ANTIQUE, left-aligned. Given
    // a fixed-ish left lane so the level pill below has a predictable
    // landing point regardless of how long the name is.
    let title_font = typography::size(typography::TITLE, h, ui_scale).max(34.0);
    let title_h = title_font * 1.05;
    let title_lane_w = header_w * 0.5;
    frame.text(TextLabel {
        rect: [header_x, header_y, title_lane_w, title_h],
        text: yk.name().into(),
        color: color::darken(color::ANTIQUE, 0.4),
        align: TextAlign::Left,
        font_px: Some(title_font),
        ..Default::default()
    });

    // Level pill — brass background, bold mono text. Sits flush
    // underneath the title (not beside it) so variable name widths
    // can't crash into it. Visually it tags the title and pairs
    // with the stat strip's right-aligned neighborhood.
    let pill_font = typography::size(typography::CAPTION, h, ui_scale).max(14.0);
    let pill_h = pill_font * 1.7;
    let pill_text = format!("Lv  {lvl}");
    let pill_w = pill_font * 5.4;
    let pill_x = header_x;
    let pill_y = header_y + title_h * 0.94;
    let (pill_bg, pill_fg) = match state {
        ProgressionState::Leveled => (color::GOLD, color::OBSIDIAN),
        ProgressionState::Unseen => (color::darken(color::ANTIQUE, 0.3), color::alpha(color::CHAMPAGNE, 0.75)),
        ProgressionState::Played => (color::BRASS, color::OBSIDIAN),
    };
    // Pill drop shadow.
    frame.quad(GpuInstance {
        rect: [pill_x + 1.5 * shadow_scale, pill_y + 2.0 * shadow_scale, pill_w, pill_h],
        color: color::alpha([0.08, 0.04, 0.02, 1.0], 0.35),
    });
    frame.quad(GpuInstance {
        rect: [pill_x, pill_y, pill_w, pill_h],
        color: pill_bg,
    });
    frame.text(TextLabel {
        rect: [pill_x, pill_y + pill_h * 0.18, pill_w, pill_h * 0.8],
        text: pill_text,
        color: pill_fg,
        align: TextAlign::Center,
        font_px: Some(pill_font * 1.15),
        ..Default::default()
    });

    // Stat strip — right-aligned, single line, "+N MULT · +N CHIPS".
    // Shares the header row with the title, with its own 50%-width
    // lane on the right. Locked yaku hide score numbers (no spoilers
    // on bonus scaling until the player has unlocked the yaku).
    let stat_font = typography::size(typography::HEADING, h, ui_scale).max(26.0);
    let stat_y = header_y + (title_h - stat_font * 1.05) * 0.45;
    let stat_text = match state {
        ProgressionState::Unseen => "— — —".into(),
        _ => format!("+{mult}  MULT   ·   +{chip}  CHIPS"),
    };
    let stat_color = match state {
        ProgressionState::Unseen => color::alpha(color::darken(color::ANTIQUE, 0.1), 0.5),
        ProgressionState::Leveled => color::darken(color::GOLD, 0.2),
        _ => color::darken(color::ANTIQUE, 0.25),
    };
    frame.text(TextLabel {
        rect: [header_x + header_w * 0.5, stat_y, header_w * 0.5, stat_font * 1.2],
        text: stat_text,
        color: stat_color,
        align: TextAlign::Right,
        font_px: Some(stat_font * 0.95),
        ..Default::default()
    });

    // Rule line under the header — 1-2px ANTIQUE strip, separates the
    // title + pill + stat row from the description/hand below. Must
    // clear the pill's bottom (pill is stacked under the title now).
    let header_bottom = (pill_y + pill_h).max(header_y + title_h);
    let rule_y = header_bottom + header_pad * 0.4;
    let rule_h = (1.5 * shadow_scale).max(1.0);
    frame.quad(GpuInstance {
        rect: [header_x, rule_y, header_w, rule_h],
        color: color::alpha(color::ANTIQUE, 0.45),
    });

    // ── Description ──────────────────────────────────────────────
    let desc_font = typography::size(typography::BODY, h, ui_scale).max(20.0);
    let desc_h = desc_font * 1.4;
    let desc_y = rule_y + rule_h + header_pad * 0.35;
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
    // Hand sits inside the parchment face only; the footer owns the
    // band below `face_y + face_h`.
    let hand_bot = face_y + face_h - header_pad * 0.2;
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

    // ── Brass footer strip with control hint ────────────────────
    // A 1-line strip along the bottom of the plaque, inside the brass
    // rim but below the parchment face. Reads as a labeled affordance
    // ("press these keys") rather than orphaned caption text.
    let footer_x = plaque_x + bevel;
    let footer_y = plaque_y + plaque_h - footer_h - bevel;
    let footer_w = plaque_w - bevel * 2.0;
    frame.quad(GpuInstance {
        rect: [footer_x, footer_y, footer_w, footer_h],
        color: color::darken(color::BRASS, 0.25),
    });
    // Top hairline for crisp separation between parchment and footer.
    frame.quad(GpuInstance {
        rect: [footer_x, footer_y, footer_w, (1.0 * shadow_scale).max(1.0)],
        color: color::darken(color::ANTIQUE, 0.15),
    });
    let footer_pad = header_pad * 0.8;
    frame.text(TextLabel {
        rect: [
            footer_x + footer_pad,
            footer_y + footer_h * 0.18,
            footer_w - footer_pad * 2.0,
            footer_h * 0.7,
        ],
        text: "← → ↑ ↓  browse".into(),
        color: color::alpha(color::OBSIDIAN, 0.75),
        align: TextAlign::Left,
        font_px: Some(hint_font * 1.1),
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: [
            footer_x + footer_pad,
            footer_y + footer_h * 0.18,
            footer_w - footer_pad * 2.0,
            footer_h * 0.7,
        ],
        text: "Esc  return".into(),
        color: color::alpha(color::OBSIDIAN, 0.75),
        align: TextAlign::Right,
        font_px: Some(hint_font * 1.1),
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
