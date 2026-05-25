//! Game over scene — shown when the player exhausts plays without reaching the target.

use std::time::Instant;

use crate::audio::SfxId;
use crate::core::memorial_talisman::{MemorialTalismanKind, select_memorial, snapshot_from_run};
use crate::core::progression::{
    LEVEL_UP_POINTS_FOR_LOSS, LEVEL_UP_POINTS_FOR_WIN, MAX_PROGRESS_LEVEL, POINTS_PER_LEVEL,
    PlayerProgress, meta_depth_roman,
};
use crate::game::engine::GameEngine;
use crate::game::event_bus::{GameEvent, GameOverReason};
use crate::game::run::RunState;
use crate::persistence;
use crate::render::draw_cmd::{ImageQuad, ImageQuadSource, UiFrame};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::widget::wrap_text;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::main_menu_exterior::MainMenuExteriorScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DismissAction;

#[derive(Clone, Debug)]
struct RunSummary {
    best_structure: String,
    most_played_structure: String,
    wing: u32,
    round: String,
}

impl RunSummary {
    fn from_run(run: &RunState) -> Self {
        let gameplay = GameEngine::read(run);
        let best_structure = if run.best_structure_score > 0 {
            format!("{} ({})", run.best_structure_name, run.best_structure_score)
        } else {
            "None".to_string()
        };
        let most_played_structure = run
            .yaku_times_played
            .iter()
            .max_by(|(ya, ca), (yb, cb)| ca.cmp(cb).then_with(|| yb.name().cmp(ya.name())))
            .map(|(yaku, count)| format!("{} ({}x)", yaku.name(), count))
            .unwrap_or_else(|| "None".to_string());

        Self {
            best_structure,
            most_played_structure,
            wing: run.wing,
            round: format!(
                "{} ({})",
                GameEngine::current_run_number(run),
                gameplay.chamber_label
            ),
        }
    }
}

pub struct GameOverScene {
    pub final_score: u64,
    pub target_score: u32,
    pub won: bool,
    pub loss_reason: Option<GameOverReason>,
    /// Remnant the player is becoming (defeat only).
    pub memorial_kind: Option<MemorialTalismanKind>,
    memorial_subtitle: Option<String>,
    summary: RunSummary,
    tree: TreeState,
    opened_at: Instant,
    outcome_sfx_fired: bool,
}

/// Delay between the game-over screen appearing and its outcome stinger.
const OUTCOME_SFX_DELAY_SECS: f32 = 1.0;

/// Stacked depth-well sprites — rim plus one ring per progression step.
const DEPTH_WELL_LAYER_ASSETS: [&str; 6] = [
    "textures/depth_well/depth_well_0.png",
    "textures/depth_well/depth_well_1.png",
    "textures/depth_well/depth_well_2.png",
    "textures/depth_well/depth_well_3.png",
    "textures/depth_well/depth_well_4.png",
    "textures/depth_well/depth_well_5.png",
];

/// Normalized UV bounds of non-transparent pixels shared by all depth-well
/// layers (measured from the 1254² source art). Layout targets this region
/// instead of the full square texture so the medallion fills horizontally.
const DEPTH_WELL_OPAQUE_UV: [f32; 4] = [0.017, 0.155, 0.982, 0.819];

/// Cap the well's on-screen height (× row line height) so the plaque stays compact.
const DEPTH_WELL_MAX_HEIGHT_MUL: f32 = 6.0;

/// Size the well to fill `inner_w` horizontally, then shrink if that exceeds
/// [`DEPTH_WELL_MAX_HEIGHT_MUL`]. Returns `(width, height)` in layout pixels.
fn depth_well_size(inner_w: f32, inset: f32, row_line_h: f32) -> (f32, f32) {
    let uw = DEPTH_WELL_OPAQUE_UV[2] - DEPTH_WELL_OPAQUE_UV[0];
    let vh = DEPTH_WELL_OPAQUE_UV[3] - DEPTH_WELL_OPAQUE_UV[1];
    let max_w = (inner_w - inset * 2.0).max(0.0);
    let max_h = row_line_h * DEPTH_WELL_MAX_HEIGHT_MUL;
    let mut w = max_w;
    let mut h = w * vh / uw;
    if h > max_h {
        h = max_h;
        w = h * uw / vh;
    }
    (w, h)
}

/// Map a layout viewport (where the opaque art should land) to the square
/// [`GpuInstance::rect`] needed for a full-texture [`ImageQuad`].
fn depth_well_draw_rect(viewport: [f32; 4]) -> [f32; 4] {
    let [u0, v0, u1, v1] = DEPTH_WELL_OPAQUE_UV;
    let uw = u1 - u0;
    let vh = v1 - v0;
    let side = viewport[2] / uw;
    let opaque_h = side * vh;
    [
        viewport[0] - side * u0,
        viewport[1] + (viewport[3] - opaque_h) * 0.5 - side * v0,
        side,
        side,
    ]
}

/// Linear interpolation between two RGBA tints.
#[inline]
fn lerp_tint(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

impl GameOverScene {
    pub fn new(run: &RunState, reason: GameOverReason) -> Self {
        let gameplay = GameEngine::read(run);
        let snap = snapshot_from_run(&run.defeat_journal, reason, run);
        let memorial = run
            .defeat_memorial_kind
            .or_else(|| Some(select_memorial(&snap)));
        let memorial_subtitle =
            memorial.map(|k| format!("{} — {}", k.name(), k.defeat_subtitle(&snap)));
        Self {
            final_score: gameplay.round_score,
            target_score: gameplay.target_score,
            won: false,
            loss_reason: Some(reason),
            memorial_kind: memorial,
            memorial_subtitle,
            summary: RunSummary::from_run(run),
            tree: TreeState::new(),
            opened_at: Instant::now(),
            outcome_sfx_fired: false,
        }
    }

    /// Construct a victory screen shown after defeating the final-ante Boss.
    pub fn victory(run: &RunState) -> Self {
        let gameplay = GameEngine::read(run);
        Self {
            final_score: gameplay.round_score,
            target_score: gameplay.target_score,
            won: true,
            loss_reason: None,
            memorial_kind: None,
            memorial_subtitle: None,
            summary: RunSummary::from_run(run),
            tree: TreeState::new(),
            opened_at: Instant::now(),
            outcome_sfx_fired: false,
        }
    }

    fn flat_items(&self, w: f32, h: f32) -> [FlatItem<DismissAction>; 1] {
        [FlatItem::new(FocusId(0), [0.0, 0.0, w, h], DismissAction)]
    }
}

fn wrapped_row_height(text: &str, col_w: f32, font_px: f32, line_h: f32) -> f32 {
    let lines = wrap_text(text, col_w, font_px / 0.99);
    line_h * lines.len().max(1) as f32
}

impl SceneBehavior for GameOverScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        if ctx.headless {
            self.opened_at = Instant::now() - std::time::Duration::from_secs(2);
        }
        if !self.outcome_sfx_fired
            && self.opened_at.elapsed().as_secs_f32() >= OUTCOME_SFX_DELAY_SECS
        {
            let sfx = if self.won {
                if self.final_score.is_multiple_of(2) {
                    SfxId::Victory
                } else {
                    SfxId::Victory2
                }
            } else {
                SfxId::Defeat
            };
            ctx.bus.push(GameEvent::UiSound(sfx));
            self.outcome_sfx_fired = true;
        }
        let items = self.flat_items(ctx.layout.window_w, ctx.layout.window_h);
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }
        if action.is_some() {
            ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
            let settings = persistence::load_settings();
            GameEngine::reset_to_demo(ctx.run, ctx.progress, &settings);
            return Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()));
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let headline = if self.won { "VICTORY" } else { "DEFEAT" };
        let headline_color = if self.won {
            color::CHAMPAGNE
        } else {
            [0.62, 0.12, 0.18, 1.0] // deep crimson — darker than RUBY
        };
        let subtitle = if self.won {
            "Final wing cleared".to_string()
        } else if let Some(ref line) = self.memorial_subtitle {
            line.clone()
        } else {
            format!("{} / {}", self.final_score, self.target_score)
        };
        let loss_reason = self.loss_reason.map(GameOverReason::loss_summary);
        let points_earned = if self.won {
            LEVEL_UP_POINTS_FOR_WIN
        } else {
            LEVEL_UP_POINTS_FOR_LOSS
        };
        let total_points = ctx.progress.level_progress_points;
        let current_level = ctx.progress.current_level();
        let prev_points = total_points.saturating_sub(points_earned);
        let prev_level = 1u32
            .saturating_add(prev_points / POINTS_PER_LEVEL)
            .min(MAX_PROGRESS_LEVEL);
        let min_for_level = PlayerProgress::min_points_for_level(current_level);
        let into_level = total_points
            .saturating_sub(min_for_level)
            .min(POINTS_PER_LEVEL);
        let progress_label = if current_level >= MAX_PROGRESS_LEVEL {
            "You have reached the limits of the house".to_string()
        } else {
            "Onward and downward".to_string()
        };
        let progress_value = if current_level >= MAX_PROGRESS_LEVEL {
            "".to_string()
        } else {
            format!("{into_level}/{POINTS_PER_LEVEL}")
        };
        let level_transition = if current_level > prev_level {
            Some(format!(
                "{} → {}",
                meta_depth_roman(prev_level),
                meta_depth_roman(current_level)
            ))
        } else {
            None
        };

        let mut rows = vec![
            (
                "Best structure".to_string(),
                self.summary.best_structure.clone(),
            ),
            (
                "Most played".to_string(),
                self.summary.most_played_structure.clone(),
            ),
            ("Wing".to_string(), self.summary.wing.to_string()),
            ("Round".to_string(), self.summary.round.clone()),
        ];
        if let Some(reason) = loss_reason {
            rows.push(("Defeat cause".to_string(), reason.to_string()));
        }

        // Layout — left-of-centre composition.  Row heights grow when labels or
        // values wrap; the subtitle uses the same width as the headline.
        let row_font_px = typography::size(typography::H36, h);
        let row_line_h = row_font_px * 1.35;
        let pad_v = row_font_px * 0.8;
        let panel_w = w * 0.25;
        let inner_w = panel_w * 0.90;
        let label_col_w = inner_w * 0.48;
        let value_col_w = inner_w * 0.50;
        let level_well_inset = row_font_px * 0.32;
        let level_inner_w_est = panel_w - 12.0;
        let (_, well_height_px) =
            depth_well_size(level_inner_w_est, level_well_inset, row_line_h);
        let level_header_h = row_line_h * 2.35 + row_font_px * 0.18;
        let level_block_h = level_header_h + well_height_px + row_font_px * 0.22;
        let level_block_gap = row_font_px * 0.60;
        let stats_h = rows
            .iter()
            .map(|(label, value)| {
                wrapped_row_height(label, label_col_w, row_font_px, row_line_h).max(
                    wrapped_row_height(value, value_col_w, row_font_px, row_line_h),
                )
            })
            .sum::<f32>();
        let panel_h = pad_v * 2.0 + level_block_h + level_block_gap + stats_h;
        let panel_x = if self.won { w * 0.67 } else { w * 0.08 };

        let gap = row_font_px * 0.5;
        let sub_font = typography::size(typography::H32, h);
        let sub_line_h = sub_font * 1.3;
        let headline_font = typography::size(typography::H5, h);
        let headline_h = headline_font * 1.25;
        let top_pad = (h * 0.04).max(row_font_px * 0.6);
        let content_w = if self.won { panel_w } else { w * 0.60 };
        let sub_lines = wrap_text(&subtitle, content_w, sub_font / 0.99);
        let sub_h = sub_line_h * sub_lines.len().max(1) as f32;

        let mut panel_y = h * 0.50 - panel_h * 0.5;
        let headline_y = panel_y - gap - sub_h - gap - headline_h;
        if headline_y < top_pad {
            panel_y += top_pad - headline_y;
        }

        let panel_rect = [panel_x, panel_y, panel_w, panel_h];
        let sub_y = panel_y - gap - sub_h;
        let subtitle_rect = [panel_x, sub_y, content_w, sub_h];
        let headline_y = sub_y - gap - headline_h;
        let headline_x = if self.won {
            panel_x + panel_w - content_w
        } else {
            panel_x
        };
        let headline_rect = [headline_x, headline_y, content_w, headline_h];

        // Thin rule sits right on the panel top edge.
        let rule_rect = [panel_x, panel_y - 2.0, panel_w, 2.0];

        // Border rim — 1-px WALNUT_BRIGHT strip around the panel.
        let border_rect = [
            panel_rect[0] + 2.0,
            panel_rect[1] + 2.0,
            panel_rect[2] - 4.0,
            panel_rect[3] - 4.0,
        ];

        // Hint sits below the panel, left-aligned.
        let hint_font = typography::size(typography::H42, h);
        let hint_h = crate::ui::colored_keywords::colored_row_line_step(hint_font);
        let hint_rect = [
            panel_x,
            panel_y + panel_h + hint_font * 0.6,
            panel_w,
            hint_h,
        ];

        let items = self.flat_items(w, h);
        let mut buttons = Vec::new();
        self.tree.register_flat_buttons(&items, &mut buttons);

        let mut frame = UiFrame::new();
        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::WALNUT_INK,
            user: 0,
        });
        if self.won {
            if ctx.effect_layers.fullscreen_water_backdrop {
                frame.moonlit_water();
            }
        } else if let Some(kind) = self.memorial_kind {
            super::game_over_tableau::push_defeat_memorial_tableau(&mut frame, ctx.layout, kind);
        } else if ctx.effect_layers.fullscreen_water_backdrop {
            frame.sunlit_water();
        }

        // Panel body — slightly deeper than before so it reads as a card.
        frame.quad(GpuInstance {
            rect: panel_rect,
            color: color::alpha(color::WALNUT_DEEP, 0.88),
            user: 0,
        });
        // 1-px border in WALNUT_BRIGHT gives the card a crisp edge.
        frame.quad(GpuInstance {
            rect: border_rect,
            color: color::alpha(color::WALNUT_BRIGHT, 0.60),
            user: 0,
        });
        // Overprint the interior so the border is just the thin rim.
        frame.quad(GpuInstance {
            rect: [
                panel_rect[0] + 3.0,
                panel_rect[1] + 3.0,
                panel_rect[2] - 6.0,
                panel_rect[3] - 6.0,
            ],
            color: color::alpha(color::WALNUT_DEEP, 0.88),
            user: 0,
        });

        let text_align = if self.won {
            TextAlign::Right
        } else {
            TextAlign::Left
        };

        frame.text(TextLabel {
            rect: headline_rect,
            text: headline.to_string(),
            color: headline_color,
            font_px: Some(headline_font),
            align: text_align,
            ..Default::default()
        });

        frame.text(TextLabel {
            rect: subtitle_rect,
            text: sub_lines.join("\n"),
            color: color::CHAMPAGNE,
            font_px: Some(sub_font),
            align: text_align,
            ..Default::default()
        });

        // Decorative rule under the subtitle.
        let rule_color = if self.won {
            color::alpha(color::CHAMPAGNE, 0.35)
        } else {
            color::alpha(color::RUBY, 0.35)
        };
        frame.quad(GpuInstance {
            rect: rule_rect,
            color: rule_color,
            user: 0,
        });

        let inner_x = panel_rect[0] + panel_rect[2] * 0.05;
        let inner_y = panel_rect[1] + pad_v;
        let level_rect = [
            panel_rect[0] + 3.0,
            inner_y,
            panel_rect[2] - 6.0,
            level_block_h,
        ];
        let level_border_rect = [
            level_rect[0] + 2.0,
            level_rect[1] + 2.0,
            level_rect[2] - 4.0,
            level_rect[3] - 4.0,
        ];
        let level_inner_rect = [
            level_rect[0] + 3.0,
            level_rect[1] + 3.0,
            level_rect[2] - 6.0,
            level_rect[3] - 6.0,
        ];
        let level_title_font = row_font_px * 1.45;
        let level_body_font = row_font_px * 0.82;
        let points_chip_font = row_font_px * 0.90;
        let points_chip_w = level_rect[2] * 0.30;
        let points_chip_h = row_line_h * 0.92;
        let points_chip_rect = [
            level_inner_rect[0] + level_inner_rect[2] - points_chip_w - row_font_px * 0.35,
            level_inner_rect[1] + row_font_px * 0.26,
            points_chip_w,
            points_chip_h,
        ];
        let progress_label_rect = [
            level_inner_rect[0] + row_font_px * 0.32,
            level_inner_rect[1] + row_line_h * 1.18,
            level_inner_rect[2] * 0.58,
            row_line_h,
        ];
        let progress_value_rect = [
            level_inner_rect[0] + level_inner_rect[2] - level_rect[2] * 0.32,
            progress_label_rect[1],
            level_rect[2] * 0.28,
            row_line_h,
        ];
        let (well_width_px, well_height_px) =
            depth_well_size(level_inner_rect[2], level_well_inset, row_line_h);
        let well_slot_w = (level_inner_rect[2] - level_well_inset * 2.0).max(0.0);
        let well_x = level_inner_rect[0]
            + level_well_inset
            + (well_slot_w - well_width_px).max(0.0) * 0.5;
        let well_y = level_inner_rect[1] + row_line_h * 2.35;
        let well_viewport_rect = [well_x, well_y, well_width_px, well_height_px];
        let well_draw_rect = depth_well_draw_rect(well_viewport_rect);
        let transition_rect = [
            level_inner_rect[0] + row_font_px * 0.32,
            well_viewport_rect[1] - row_line_h * 0.82,
            level_inner_rect[2] - row_font_px * 0.64,
            row_line_h,
        ];
        let mut row_y = inner_y + level_block_h + level_block_gap;

        // Depth plaque: visual progression summary.
        frame.quad(GpuInstance {
            rect: level_rect,
            color: color::alpha(color::WALNUT_SOFT, 0.70),
            user: 0,
        });
        frame.quad(GpuInstance {
            rect: level_border_rect,
            color: color::alpha(color::WALNUT_BRIGHT, 0.75),
            user: 0,
        });
        frame.quad(GpuInstance {
            rect: level_inner_rect,
            color: color::alpha(color::WALNUT_RAISED, 0.72),
            user: 0,
        });

        frame.text(TextLabel {
            rect: [
                level_inner_rect[0] + row_font_px * 0.32,
                level_inner_rect[1] + row_font_px * 0.12,
                level_inner_rect[2] * 0.62,
                row_line_h * 1.35,
            ],
            text: format!("DEPTH {}", meta_depth_roman(current_level)),
            color: color::CHAMPAGNE,
            font_px: Some(level_title_font),
            align: TextAlign::Left,
            ..Default::default()
        });

        frame.quad(GpuInstance {
            rect: points_chip_rect,
            color: color::alpha(color::WALNUT_BRIGHT, 0.85),
            user: 0,
        });
        frame.text(TextLabel {
            rect: points_chip_rect,
            text: format!("{points_earned} steps down"),
            color: color::PARCHMENT,
            font_px: Some(points_chip_font),
            align: TextAlign::Center,
            ..Default::default()
        });

        if let Some(transition) = level_transition {
            frame.text(TextLabel {
                rect: transition_rect,
                text: transition,
                color: color::alpha(color::GOLD, 0.92),
                font_px: Some(level_body_font),
                align: TextAlign::Left,
                ..Default::default()
            });
        }
        frame.text(TextLabel {
            rect: progress_label_rect,
            text: progress_label,
            color: color::STONE,
            font_px: Some(level_body_font),
            align: TextAlign::Left,
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: progress_value_rect,
            text: progress_value,
            color: color::PARCHMENT,
            font_px: Some(level_body_font),
            align: TextAlign::Right,
            ..Default::default()
        });

        if current_level > prev_level {
            let ribbon_h = row_line_h * 0.86;
            let ribbon_w = level_rect[2] * 0.34;
            let ribbon_rect = [
                level_rect[0] + level_rect[2] - ribbon_w - row_font_px * 0.20,
                level_rect[1] - ribbon_h * 0.36,
                ribbon_w,
                ribbon_h,
            ];
            frame.quad(GpuInstance {
                rect: ribbon_rect,
                color: color::alpha(color::WALNUT_SOFT, 0.96),
                user: 0,
            });
            frame.quad(GpuInstance {
                rect: [
                    ribbon_rect[0] + 1.0,
                    ribbon_rect[1] + 1.0,
                    ribbon_rect[2] - 2.0,
                    ribbon_rect[3] - 2.0,
                ],
                color: color::alpha(color::BRASS, 0.88),
                user: 0,
            });
            frame.text(TextLabel {
                rect: ribbon_rect,
                text: "DEEPER".to_string(),
                color: color::WALNUT_DEEP,
                font_px: Some(level_body_font * 0.92),
                align: TextAlign::Center,
                ..Default::default()
            });
        }

        for (idx, (label, value)) in rows.iter().enumerate() {
            let label_lines = wrap_text(label, label_col_w, row_font_px / 0.99);
            let value_lines = wrap_text(value, value_col_w, row_font_px / 0.99);
            let row_h = row_line_h * label_lines.len().max(value_lines.len()).max(1) as f32;

            if idx % 2 == 0 {
                frame.quad(GpuInstance {
                    rect: [panel_rect[0] + 3.0, row_y, panel_rect[2] - 6.0, row_h],
                    color: color::alpha(color::WALNUT_RAISED, 0.25),
                    user: 0,
                });
            }

            frame.text(TextLabel {
                rect: [inner_x, row_y, label_col_w, row_h],
                text: label_lines.join("\n"),
                color: color::STONE,
                font_px: Some(row_font_px),
                align: TextAlign::Left,
                ..Default::default()
            });
            frame.text(TextLabel {
                rect: [inner_x + inner_w * 0.50, row_y, value_col_w, row_h],
                text: value_lines.join("\n"),
                color: color::PARCHMENT,
                font_px: Some(row_font_px),
                align: TextAlign::Right,
                ..Default::default()
            });
            row_y += row_h;
        }

        frame.text(TextLabel {
            rect: hint_rect,
            text: "Is your fate settled then?".to_string(),
            color: color::alpha(color::CHAMPAGNE, 0.70),
            font_px: Some(hint_font),
            align: text_align,
            ..Default::default()
        });

        // ── Depth-well medallion (stacked UI sprites) ────────────────────────
        {
            let elapsed = self.opened_at.elapsed().as_secs_f32();
            let displayed_fill = (elapsed * 4.0).min(into_level as f32);
            let unlit_tint: [f32; 4] = [0.32, 0.24, 0.16, 1.0];
            let lit_tint: [f32; 4] = [1.15, 0.98, 0.72, 1.0];
            let mut well_images: Vec<ImageQuad> = Vec::with_capacity(DEPTH_WELL_LAYER_ASSETS.len());

            well_images.push(ImageQuad {
                inst: GpuInstance {
                    rect: well_draw_rect,
                    color: [1.0, 1.0, 1.0, 1.0],
                    user: 0,
                },
                source: ImageQuadSource::Asset {
                    path: DEPTH_WELL_LAYER_ASSETS[0],
                },
            });
            for idx in 0..POINTS_PER_LEVEL {
                let t = (displayed_fill - idx as f32).clamp(0.0, 1.0);
                well_images.push(ImageQuad {
                    inst: GpuInstance {
                        rect: well_draw_rect,
                        color: lerp_tint(unlit_tint, lit_tint, t),
                        user: 0,
                    },
                    source: ImageQuadSource::Asset {
                        path: DEPTH_WELL_LAYER_ASSETS[(idx + 1) as usize],
                    },
                });
            }

            frame.image_quads(well_images);
        }

        frame.buttons = buttons;
        frame.window_title = if self.won {
            "Victory! — Final wing cleared".to_string()
        } else {
            format!(
                "Game Over — {} / {}",
                self.final_score, self.target_score
            )
        };
        frame
    }
}
