//! Gameplay scene — the main tile-playing loop.

use std::time::Instant;

use crate::core::hand::describe_hand;
use crate::game::cascade::{CascadeColor, ScoringCascade};
use crate::game::run::{STARTING_DISCARDS, STARTING_PLAYS};
use crate::render::animation::ENTITY_SCORE_PANEL;
use crate::render::wgpu_renderer::{GpuInstance, build_instances_from_layout, TextLabel};
use crate::ui::input::{UiAction, apply_ui_actions};

use super::{ButtonDef, DrawCtx, SceneDrawOutput, SceneTransition, UpdateCtx, relic_row};

pub struct GameplayScene {
    /// Active scoring cascade animation (None when idle).
    cascade: Option<ScoringCascade>,
    /// Displayed score — ticked by the cascade, snaps to real score when idle.
    displayed_score: u32,
}

impl GameplayScene {
    pub fn new() -> Self {
        Self {
            cascade: None,
            displayed_score: 0,
        }
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let now = Instant::now();

        // If a cascade is running, advance it and block most input.
        if let Some(ref mut cascade) = self.cascade {
            cascade.update(now);
            let frame = cascade.frame(now);
            self.displayed_score = frame.displayed_score;

            // Pulse the score panel on each new step reveal.
            if frame.new_step_index.is_some() {
                ctx.anim.pulse(ENTITY_SCORE_PANEL);
            }

            if !frame.active {
                // Cascade finished — snap to real score and clear.
                self.displayed_score = ctx.run.round_score;
                self.cascade = None;
            } else {
                // Allow skip on any key press during cascade.
                if !ctx.actions.is_empty() {
                    cascade.skip();
                    self.displayed_score = ctx.run.round_score;
                    self.cascade = None;
                }
                return None;
            }
        }

        // Normal input handling when no cascade is active.
        for a in ctx.actions {
            match a {
                UiAction::ScoreHand => {
                    let score_before = ctx.run.round_score;
                    let pts = ctx.run.score_selected_tiles(ctx.bus);
                    ctx.anim.pulse(ENTITY_SCORE_PANEL);

                    if let Some(breakdown) = ctx.run.last_breakdown.clone() {
                        if !breakdown.steps.is_empty() || breakdown.base_points > 0 {
                            self.cascade = Some(ScoringCascade::new(
                                breakdown,
                                score_before,
                                pts,
                            ));
                            // displayed_score will be driven by cascade
                        } else {
                            self.displayed_score = ctx.run.round_score;
                        }
                    } else {
                        self.displayed_score = ctx.run.round_score;
                    }
                }
                UiAction::SortBySuit => {
                    ctx.run.sort_hand_by_suit();
                    ctx.run.clear_selection();
                    ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
                }
                UiAction::SortByRank => {
                    ctx.run.sort_hand_by_rank();
                    ctx.run.clear_selection();
                    ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
                }
                _ => {}
            }
        }
        // Let apply_ui_actions handle toggle-select, commit discard, cancel, and focus movement.
        let non_score: Vec<_> = ctx.actions.iter()
            .filter(|a| !matches!(a, UiAction::ScoreHand | UiAction::SortBySuit | UiAction::SortByRank))
            .copied()
            .collect();
        apply_ui_actions(&non_score, ctx.run, ctx.bus, ctx.anim, ctx.focus_tile_index);
        None
    }

    /// Whether the cascade is actively animating (for redraw requests).
    pub fn is_animating(&self) -> bool {
        self.cascade.is_some()
    }

    pub fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let layout = ctx.layout;
        let run = ctx.run;
        let focus = ctx.focus_tile_index.min(run.hand.len().saturating_sub(1));
        let now = Instant::now();

        let ts = ctx.anim.transform_for(ENTITY_SCORE_PANEL);

        let instances = build_instances_from_layout(
            (layout.score_panel.x, layout.score_panel.y, layout.score_panel.w, layout.score_panel.h),
            (layout.modifier_strip.x, layout.modifier_strip.y, layout.modifier_strip.w, layout.modifier_strip.h),
            ts.scale,
            run.plays_remaining,
            STARTING_PLAYS,
            run.discards_remaining,
            STARTING_DISCARDS,
        );

        let hand_slots: Vec<(f32, f32, f32, f32)> = layout
            .hand_slots
            .iter()
            .take(run.hand.len())
            .map(|r| (r.x, r.y, r.w, r.h))
            .collect();

        // Use cascade's displayed score if active, otherwise real score.
        let shown_score = if self.cascade.is_some() {
            self.displayed_score
        } else {
            run.round_score
        };

        // Score panel text.
        let tiles_left = run.wall.remaining();
        let score_text = format!(
            "{}  Round {}   {} / {}   Gold: {}   Wall: {}",
            run.blind.name(), run.run_number, shown_score, run.target_score, run.gold, tiles_left
        );

        // Modifier strip: cascade / sets (full width). Relics shown as row below score panel.
        let ms = layout.modifier_strip;

        // Build cascade / sets text for left panel.
        let (cascade_text, cascade_color) = if let Some(ref cascade) = self.cascade {
            let frame = cascade.frame(now);
            let mut parts = Vec::new();
            for line in &frame.lines {
                let marker = if line.is_current { "▸ " } else { "  " };
                parts.push(format!("{}{}", marker, line.text));
            }
            let color = if frame.lines.last().map(|l| l.color) == Some(CascadeColor::Total) {
                [1.0, 0.9, 0.3, 1.0] // gold for total
            } else {
                [0.95, 0.85, 0.4, 1.0] // warm yellow for steps
            };
            (parts.join("  →  "), color)
        } else {
            // No cascade active — show detected sets in the selected tiles.
            let selected_tiles: Vec<_> = run
                .hand
                .iter()
                .zip(run.selected.iter())
                .filter(|&(_, &sel)| sel)
                .map(|(t, _)| *t)
                .collect();
            if selected_tiles.is_empty() {
                ("Select tiles to play".to_string(), [0.6, 0.6, 0.6, 1.0])
            } else {
                let hand_desc = describe_hand(&selected_tiles);
                let valid = run.is_selection_valid();
                let text = if hand_desc.is_empty() {
                    "No valid melds".to_string()
                } else if valid {
                    format!("{hand_desc}  [VALID]")
                } else {
                    format!("{hand_desc}  [INVALID]")
                };
                let color = if valid {
                    [0.3, 0.9, 0.4, 1.0] // green for valid
                } else {
                    [0.9, 0.4, 0.3, 1.0] // red for invalid
                };
                (text, color)
            }
        };

        // Relic row below score panel.
        let (relic_insts, relic_labels, relic_icons) = relic_row(&run.relics, &layout.score_panel, layout.window_w);

        // Bottom button bar.
        let scale = (layout.window_w.min(layout.window_h)) / 600.0;
        let btn_w = (120.0 * scale).max(60.0);
        let btn_h = (32.0 * scale).max(20.0);
        let btn_gap = 12.0 * scale;
        let selected_count = run.selected_count();
        let selection_valid = run.is_selection_valid();
        let container_w = btn_w * 4.0 + btn_gap * 3.0;
        let container_x = (layout.window_w - container_w) * 0.5;
        let btn_y = layout.window_h - btn_h - (12.0 * scale);

        let suit_btn_rect = (container_x, btn_y, btn_w, btn_h);
        let rank_btn_rect = (container_x + btn_w + btn_gap, btn_y, btn_w, btn_h);
        let play_btn_rect = (container_x + (btn_w + btn_gap) * 2.0, btn_y, btn_w, btn_h);
        let discard_btn_rect = (container_x + (btn_w + btn_gap) * 3.0, btn_y, btn_w, btn_h);

        let mut instances = instances;
        instances.extend(relic_insts);
        instances.push(GpuInstance {
            rect: [suit_btn_rect.0, suit_btn_rect.1, suit_btn_rect.2, suit_btn_rect.3],
            color: [0.22, 0.38, 0.55, 0.92],
        });
        instances.push(GpuInstance {
            rect: [rank_btn_rect.0, rank_btn_rect.1, rank_btn_rect.2, rank_btn_rect.3],
            color: [0.22, 0.38, 0.55, 0.92],
        });
        let play_color = if selection_valid && run.plays_remaining > 0 {
            [0.18, 0.55, 0.25, 0.92] // green when valid
        } else {
            [0.35, 0.35, 0.35, 0.60] // gray when invalid/disabled
        };
        instances.push(GpuInstance {
            rect: [play_btn_rect.0, play_btn_rect.1, play_btn_rect.2, play_btn_rect.3],
            color: play_color,
        });
        let discard_color = if selected_count > 0 && run.discards_remaining > 0 {
            [0.65, 0.18, 0.18, 0.92]
        } else {
            [0.35, 0.35, 0.35, 0.60]
        };
        instances.push(GpuInstance {
            rect: [discard_btn_rect.0, discard_btn_rect.1, discard_btn_rect.2, discard_btn_rect.3],
            color: discard_color,
        });

        let mut text_labels = vec![
            // Score panel
            TextLabel {
                rect: [ms.x, layout.score_panel.y, ms.w * 0.75, layout.score_panel.h],
                text: score_text,
                color: [1.0, 1.0, 1.0, 1.0],
            },
            // Full-width modifier strip: cascade or detected sets
            TextLabel {
                rect: [ms.x, ms.y, ms.w, ms.h],
                text: cascade_text,
                color: cascade_color,
            },
        ];
        text_labels.extend(relic_labels);
        text_labels.push(TextLabel {
            rect: [suit_btn_rect.0, suit_btn_rect.1, suit_btn_rect.2, suit_btn_rect.3],
            text: "Sort by Suit".into(),
            color: [1.0, 1.0, 1.0, 1.0],
        });
        text_labels.push(TextLabel {
            rect: [rank_btn_rect.0, rank_btn_rect.1, rank_btn_rect.2, rank_btn_rect.3],
            text: "Sort by Rank".into(),
            color: [1.0, 1.0, 1.0, 1.0],
        });
        let play_label = if run.plays_remaining > 0 {
            format!("Play Hand ({})", run.plays_remaining)
        } else {
            "Play Hand".into()
        };
        text_labels.push(TextLabel {
            rect: [play_btn_rect.0, play_btn_rect.1, play_btn_rect.2, play_btn_rect.3],
            text: play_label,
            color: [1.0, 1.0, 1.0, 1.0],
        });
        let discard_label = if selected_count > 0 {
            format!("Discard ({})", selected_count)
        } else {
            "Discard".into()
        };
        text_labels.push(TextLabel {
            rect: [discard_btn_rect.0, discard_btn_rect.1, discard_btn_rect.2, discard_btn_rect.3],
            text: discard_label,
            color: [1.0, 1.0, 1.0, 1.0],
        });

        let buttons = vec![
            ButtonDef { rect: suit_btn_rect, action: UiAction::SortBySuit },
            ButtonDef { rect: rank_btn_rect, action: UiAction::SortByRank },
            ButtonDef { rect: play_btn_rect, action: UiAction::ScoreHand },
            ButtonDef { rect: discard_btn_rect, action: UiAction::CommitDiscard },
        ];

        SceneDrawOutput {
            instances,
            hand_tiles: run.hand.to_vec(),
            hand_slots,
            focus,
            selected_tiles: run.selected.clone(),
            text_labels,
            relic_icons,
            buttons,
            window_title: format!(
                "Mahjuro — {} Round {}  {} / {}  Gold: {}  Hands: {}  Discards: {}",
                run.blind.name(), run.run_number, shown_score, run.target_score,
                run.gold, run.plays_remaining, run.discards_remaining
            ),
        }
    }
}
