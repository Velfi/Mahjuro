//! Game over scene — shown when the player exhausts plays without reaching the target.

use std::time::Instant;

use crate::audio::SfxId;
use crate::core::memorial_talisman::{MemorialTalismanKind, select_memorial, snapshot_from_run};
use crate::game::engine::GameEngine;
use crate::game::event_bus::{GameEvent, GameOverReason};
use crate::game::run::RunState;
use crate::persistence;
use crate::render::decal::{load_ui_font_bytes, measure_label_advances};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use crate::render::draw_cmd::UiFrame;

use super::main_menu_exterior::MainMenuExteriorScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DismissAction;

#[derive(Clone, Debug)]
struct RunSummary {
    best_structure: String,
    most_played_structure: String,
    tiles_played: u32,
    tiles_discarded: u32,
    times_restocked: u32,
    ante: u32,
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
            tiles_played: run.tiles_played,
            tiles_discarded: run.tiles_discarded,
            times_restocked: run.times_restocked,
            ante: run.ante,
            round: format!(
                "{} ({})",
                GameEngine::current_run_number(run),
                gameplay.blind_label
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
    ui_font: Option<fontdue::Font>,
}

/// Delay between the game-over screen appearing and its outcome stinger.
const OUTCOME_SFX_DELAY_SECS: f32 = 1.0;

impl GameOverScene {
    pub fn new(run: &RunState, reason: GameOverReason) -> Self {
        let gameplay = GameEngine::read(run);
        let snap = snapshot_from_run(&run.defeat_journal, reason, run);
        let memorial = run.defeat_memorial_kind.or_else(|| Some(select_memorial(&snap)));
        let memorial_subtitle = memorial.map(|k| {
            format!(
                "{} — {}",
                k.name(),
                k.defeat_subtitle(&snap)
            )
        });
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
            ui_font: load_ui_font_bytes().and_then(|bytes| {
                fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok()
            }),
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
            ui_font: load_ui_font_bytes().and_then(|bytes| {
                fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok()
            }),
        }
    }

    fn flat_items(&self, w: f32, h: f32) -> [FlatItem<DismissAction>; 1] {
        [FlatItem::new(FocusId(0), [0.0, 0.0, w, h], DismissAction)]
    }

    fn marquee_offset(&self, text: &str, width: f32, font_px: f32) -> f32 {
        let Some(font) = &self.ui_font else {
            return 0.0;
        };
        let (_, _, advances) = measure_label_advances(
            font,
            text,
            width.max(1.0).round() as u32,
            font_px.max(1.0).round() as u32,
            Some(font_px),
        );
        let text_w: f32 = advances.iter().sum();
        let overflow = (text_w - width).max(0.0);
        if overflow <= 4.0 {
            return 0.0;
        }

        let hold_secs = 0.9;
        let speed_px_per_sec = 36.0;
        let travel_secs = overflow / speed_px_per_sec;
        let cycle_secs = hold_secs * 2.0 + travel_secs * 2.0;
        let t = self
            .opened_at
            .elapsed()
            .as_secs_f32()
            .rem_euclid(cycle_secs);

        if t < hold_secs {
            0.0
        } else if t < hold_secs + travel_secs {
            (t - hold_secs) * speed_px_per_sec
        } else if t < hold_secs * 2.0 + travel_secs {
            overflow
        } else {
            overflow - (t - (hold_secs * 2.0 + travel_secs)) * speed_px_per_sec
        }
    }
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
            "Final ante cleared".to_string()
        } else if let Some(ref line) = self.memorial_subtitle {
            line.clone()
        } else {
            format!("{} / {}", self.final_score, self.target_score)
        };
        let loss_reason = self.loss_reason.map(GameOverReason::loss_summary);

        // Layout — left-of-centre composition.  The panel is compact: just tall
        // enough for the current set of tight rows.  All sizes derive from h
        // so they scale.
        let row_font_px = typography::size(typography::H36, h);
        let row_h = row_font_px * 2.0; // 2× leading — dense but breathable
        let n_rows = if self.won { 7.0 } else { 8.0 };
        let pad_v = row_font_px * 0.8; // top/bottom padding inside panel
        let panel_w = w * 0.25;
        let panel_h = row_h * n_rows + pad_v * 2.0;
        // Victory sits on the right, defeat on the left — contrasting compositions.
        let panel_x = if self.won { w * 0.67 } else { w * 0.08 };

        // Build upward from panel_y so nothing overlaps:
        //   panel_y - gap - sub_h - gap - headline_h  = headline top
        let gap = row_font_px * 0.5;
        let sub_font = typography::size(typography::H32, h);
        let sub_h = sub_font * 1.3;
        let headline_font = typography::size(typography::H5, h);
        let headline_h = headline_font * 1.25;
        let top_pad = (h * 0.04).max(row_font_px * 0.6);

        // Centre the stats card, then nudge the whole stack down if the H5 headline
        // would clip above the window (common at 1080p with eight stat rows).
        let mut panel_y = h * 0.50 - panel_h * 0.5;
        let headline_y = panel_y - gap - sub_h - gap - headline_h;
        if headline_y < top_pad {
            panel_y += top_pad - headline_y;
        }

        let panel_rect = [panel_x, panel_y, panel_w, panel_h];
        let sub_y = panel_y - gap - sub_h;
        let subtitle_rect = [panel_x, sub_y, panel_w, sub_h];
        let headline_y = sub_y - gap - headline_h;
        // On victory (right side) the headline extends leftward from the panel
        // right edge; on defeat it extends rightward from the panel left edge.
        let headline_x = if self.won {
            panel_x + panel_w - w * 0.60
        } else {
            panel_x
        };
        let headline_rect = [headline_x, headline_y, w * 0.60, headline_h];

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
        let hint_h = hint_font * 1.4;
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
            super::game_over_tableau::push_defeat_memorial_tableau(&mut frame, &ctx.layout, kind);
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
            text: subtitle,
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

        let mut rows = vec![
            (
                "Best structure".to_string(),
                self.summary.best_structure.clone(),
            ),
            (
                "Most played".to_string(),
                self.summary.most_played_structure.clone(),
            ),
            (
                "Tiles played".to_string(),
                self.summary.tiles_played.to_string(),
            ),
            (
                "Tiles discarded".to_string(),
                self.summary.tiles_discarded.to_string(),
            ),
            (
                "Times restocked".to_string(),
                self.summary.times_restocked.to_string(),
            ),
            ("Ante".to_string(), self.summary.ante.to_string()),
            ("Round".to_string(), self.summary.round.clone()),
        ];
        if let Some(reason) = loss_reason {
            rows.push(("Defeat cause".to_string(), reason.to_string()));
        }

        // Inner content area — inset by one padding unit on each side.
        let inner_x = panel_rect[0] + panel_rect[2] * 0.05;
        let inner_w = panel_rect[2] * 0.90;
        let inner_y = panel_rect[1] + pad_v;
        // row_h and row_font_px already computed above.

        for (idx, (label, value)) in rows.iter().enumerate() {
            let y = inner_y + row_h * idx as f32;
            let text_y = y + (row_h - row_font_px) * 0.5;
            let text_h = row_font_px * 1.2;

            // Alternating row tint for scannability.
            if idx % 2 == 0 {
                frame.quad(GpuInstance {
                    rect: [panel_rect[0] + 3.0, y, panel_rect[2] - 6.0, row_h],
                    color: color::alpha(color::WALNUT_RAISED, 0.25),
                    user: 0,
                });
            }

            // Label — left-aligned, muted.
            frame.text(TextLabel {
                rect: [inner_x, text_y, inner_w * 0.48, text_h],
                text: label.clone(),
                color: color::STONE,
                font_px: Some(row_font_px),
                align: TextAlign::Left,
                ..Default::default()
            });
            // Value — right side of the row, brighter.
            let value_rect = [inner_x + inner_w * 0.50, text_y, inner_w * 0.50, text_h];
            frame.text(TextLabel {
                rect: value_rect,
                text: value.clone(),
                color: color::PARCHMENT,
                font_px: Some(row_font_px),
                align: TextAlign::Right,
                scroll_offset: self.marquee_offset(value, value_rect[2], row_font_px),
                ..Default::default()
            });
        }

        frame.text(TextLabel {
            rect: hint_rect,
            text: "Press Enter to continue".to_string(),
            color: color::alpha(color::CHAMPAGNE, 0.70),
            font_px: Some(hint_font),
            align: text_align,
            ..Default::default()
        });

        frame.buttons = buttons;
        frame.window_title = if self.won {
            "Victory! — Final ante cleared — Press Enter to continue".to_string()
        } else {
            format!(
                "Game Over — {} / {} — Press Enter to restart",
                self.final_score, self.target_score
            )
        };
        frame
    }
}
