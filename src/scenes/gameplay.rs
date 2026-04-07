//! Gameplay scene — the main tile-playing loop.

use std::time::Instant;

use crate::core::hand::{describe_hand, suggest_completions};
use crate::core::yaku::yaku_preview;
use crate::game::cascade::{CascadeColor, ScoringCascade};
use crate::game::run::{STARTING_DISCARDS, STARTING_PLAYS};
use crate::render::animation::ENTITY_SCORE_PANEL;
use crate::render::particles::ParticleSystem;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel, build_instances_from_layout};
use crate::ui::input::{UiAction, apply_ui_actions};

use super::pause_menu::{PauseMenu, PauseUpdate};
use super::{ButtonDef, DrawCtx, SceneDrawOutput, SceneTransition, UpdateCtx, relic_row};

/// Bottom button indices in gameplay.
const BTN_SORT_SUIT: usize = 0;
const BTN_SORT_RANK: usize = 1;
const BTN_PLAY: usize = 2;
const BTN_DISCARD: usize = 3;
const BTN_COUNT: usize = 4;

pub struct GameplayScene {
    /// Active scoring cascade animation (None when idle).
    cascade: Option<ScoringCascade>,
    /// Displayed score — ticked by the cascade, snaps to real score when idle.
    displayed_score: u32,
    /// Particle effects for scoring.
    particles: ParticleSystem,
    /// Timestamp of last frame for dt calculation.
    last_frame: Instant,
    /// Shared pause menu overlay.
    pause_menu: PauseMenu,
    /// Which bottom button is focused (None = hand tiles have focus).
    button_focus: Option<usize>,
    /// Tile indices that should depart this frame (set during update, consumed during draw).
    pending_departures: Vec<usize>,
    /// Latest cursor position (window coords), captured each update for hover tooltips.
    cursor_pos: (f32, f32),
}

impl GameplayScene {
    pub fn new() -> Self {
        Self {
            cascade: None,
            displayed_score: 0,
            particles: ParticleSystem::new(),
            last_frame: Instant::now(),
            pause_menu: PauseMenu::new(),
            button_focus: None,
            pending_departures: Vec::new(),
            cursor_pos: (0.0, 0.0),
        }
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.particles.update(dt);
        self.cursor_pos = ctx.cursor_pos;

        // Pause menu handling.
        if self.pause_menu.paused {
            match self.pause_menu.update(
                ctx.actions,
                ctx.run,
                ctx.cursor_pos,
                ctx.layout.window_w,
                ctx.layout.window_h,
            ) {
                PauseUpdate::StayPaused | PauseUpdate::Resume => return None,
                PauseUpdate::Transition(t) => return t,
                PauseUpdate::Quit => {
                    *ctx.quit_requested = true;
                    return None;
                }
            }
        }

        // Check for pause trigger.
        for a in ctx.actions {
            if matches!(a, UiAction::Pause) {
                self.pause_menu.open();
                return None;
            }
        }

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

        // Handle button-bar focus navigation.
        let mut actions_for_scene: Vec<UiAction> = Vec::new();
        for &a in ctx.actions.iter() {
            match a {
                UiAction::FocusDown => {
                    if self.button_focus.is_none() {
                        self.button_focus = Some(BTN_PLAY); // default to Play button
                    }
                    continue;
                }
                UiAction::FocusUp => {
                    if self.button_focus.is_some() {
                        self.button_focus = None;
                        continue;
                    }
                }
                UiAction::FocusNext if self.button_focus.is_some() => {
                    let cur = self.button_focus.unwrap();
                    self.button_focus = Some((cur + 1).min(BTN_COUNT - 1));
                    continue;
                }
                UiAction::FocusPrev if self.button_focus.is_some() => {
                    let cur = self.button_focus.unwrap();
                    self.button_focus = Some(cur.saturating_sub(1));
                    continue;
                }
                UiAction::Confirm if self.button_focus.is_some() => {
                    // Translate button press into the corresponding action.
                    let mapped = match self.button_focus.unwrap() {
                        BTN_SORT_SUIT => UiAction::SortBySuit,
                        BTN_SORT_RANK => UiAction::SortByRank,
                        BTN_PLAY => UiAction::ScoreHand,
                        BTN_DISCARD => UiAction::CommitDiscard,
                        _ => continue,
                    };
                    actions_for_scene.push(mapped);
                    continue;
                }
                _ => {}
            }
            actions_for_scene.push(a);
        }

        // Clear any previous frame's departures.
        self.pending_departures.clear();

        // Normal input handling when no cascade is active.
        for a in &actions_for_scene {
            match a {
                UiAction::ScoreHand => {
                    let had_selection = ctx.run.selected_count() > 0;
                    let score_before = ctx.run.round_score;
                    let pts = ctx.run.score_selected_tiles(ctx.bus);

                    if pts == 0 && had_selection {
                        // Invalid hand — shake feedback.
                        ctx.anim
                            .shake(crate::render::animation::ENTITY_HAND_STRIP, 8.0, 200);
                    } else {
                        ctx.anim.pulse(ENTITY_SCORE_PANEL);
                    }

                    if let Some(breakdown) = ctx.run.last_breakdown.clone() {
                        if !breakdown.steps.is_empty() || breakdown.base_points > 0 {
                            self.cascade = Some(ScoringCascade::with_tuning(
                                breakdown,
                                score_before,
                                pts,
                                ctx.cascade_tuning.clone(),
                            ));
                            // Emit particles on successful score.
                            let sp = ctx.layout.score_panel;
                            let px = sp.x + sp.w * 0.5;
                            let py = sp.y + sp.h * 0.5;
                            let count = (pts as usize / 20).clamp(5, 40);
                            self.particles
                                .emit(px, py, count, [1.0, 0.85, 0.3, 1.0], 0.8);
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
                UiAction::CommitDiscard => {
                    // Capture selected indices BEFORE discard so we can animate them departing.
                    if ctx.run.selected_count() > 0 && ctx.run.discards_remaining > 0 {
                        let selected_indices: Vec<usize> = ctx
                            .run
                            .selected
                            .iter()
                            .enumerate()
                            .filter(|&(_, &s)| s)
                            .map(|(i, _)| i)
                            .collect();
                        self.pending_departures = selected_indices;
                    }
                    let discarded = ctx.run.discard_selected(ctx.bus);
                    if discarded > 0 {
                        ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
                    }
                }
                _ => {}
            }
        }
        // Let apply_ui_actions handle toggle-select, cancel, and focus movement.
        let non_handled: Vec<_> = actions_for_scene
            .iter()
            .filter(|a| {
                !matches!(
                    a,
                    UiAction::ScoreHand
                        | UiAction::SortBySuit
                        | UiAction::SortByRank
                        | UiAction::CommitDiscard
                )
            })
            .copied()
            .collect();
        apply_ui_actions(
            &non_handled,
            ctx.run,
            ctx.bus,
            ctx.anim,
            ctx.focus_tile_index,
        );
        None
    }

    /// Whether the cascade is actively animating (for redraw requests).
    pub fn is_animating(&self) -> bool {
        self.cascade.is_some() || self.particles.is_active()
    }

    pub fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let layout = ctx.layout;
        let run = ctx.run;
        let focus = ctx.focus_tile_index.min(run.hand.len().saturating_sub(1));
        let now = Instant::now();

        let ts = ctx.anim.transform_for(ENTITY_SCORE_PANEL);

        let instances = build_instances_from_layout(
            (
                layout.score_panel.x,
                layout.score_panel.y,
                layout.score_panel.w,
                layout.score_panel.h,
            ),
            (
                layout.modifier_strip.x,
                layout.modifier_strip.y,
                layout.modifier_strip.w,
                layout.modifier_strip.h,
            ),
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
        let dora_section = if ctx.progress.dora_enabled() {
            let indicator_text: String = run
                .wall
                .dora_indicator_tiles()
                .iter()
                .map(|t| t.label())
                .collect::<Vec<_>>()
                .join(",");
            let face_text: String = run
                .wall
                .dora_faces()
                .iter()
                .map(|(suit, rank)| {
                    use crate::core::tile::Tile;
                    Tile::new(*suit, *rank, 0).label()
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("   Dora: {} (ind: {})", face_text, indicator_text)
        } else {
            String::new()
        };
        let score_text = format!(
            "{}  Round {}   {} / {}   Gold: {}   Wall: {}{}",
            run.blind.name(),
            run.run_number,
            shown_score,
            run.target_score,
            run.gold,
            tiles_left,
            dora_section
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
            // Append yaku names if any were detected.
            if let Some(ref bd) = run.last_breakdown {
                if !bd.detected_yaku.is_empty() {
                    let yaku_names: Vec<_> = bd.detected_yaku.iter().map(|y| y.name()).collect();
                    parts.push(format!("[{}]", yaku_names.join(", ")));
                }
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

        // Relic row in its own strip below the modifier strip.
        let (relic_insts, relic_labels, relic_icons) =
            relic_row(&run.relics, &layout.relic_strip, layout.window_w);

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

        // ── Yaku progress panel (above the bottom button bar) ────────────
        // Builds one card per available yaku showing how close the current
        // selection is to qualifying. Active yaku glow gold; partial progress
        // fills a horizontal bar across the card.
        let selected_tiles_for_yaku: Vec<_> = run
            .hand
            .iter()
            .zip(run.selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(t, _)| *t)
            .collect();
        let previews = yaku_preview(&selected_tiles_for_yaku, &run.available_yaku);
        let mut yaku_text_labels: Vec<TextLabel> = Vec::new();
        if !previews.is_empty() {
            let panel_h = (54.0 * scale).max(36.0);
            let panel_gap = 8.0 * scale;
            let panel_y = btn_y - panel_h - panel_gap;
            let panel_w = container_w;
            let panel_x = container_x;
            let n = previews.len() as f32;
            let card_gap = 4.0 * scale;
            let card_w = (panel_w - card_gap * (n - 1.0)) / n;
            for (i, p) in previews.iter().enumerate() {
                let cx = panel_x + i as f32 * (card_w + card_gap);
                let cy = panel_y;
                // Card background — dim slate, brighter when active.
                let bg_color = if p.active {
                    [0.28, 0.24, 0.10, 0.95]
                } else {
                    [0.10, 0.12, 0.20, 0.85]
                };
                instances.push(GpuInstance {
                    rect: [cx, cy, card_w, panel_h],
                    color: bg_color,
                });
                // Progress fill — bottom strip of the card.
                let fill_h = (8.0 * scale).max(5.0);
                let fill_y = cy + panel_h - fill_h - 2.0;
                // Track behind the fill.
                instances.push(GpuInstance {
                    rect: [cx + 4.0, fill_y, card_w - 8.0, fill_h],
                    color: [0.06, 0.07, 0.12, 0.9],
                });
                // Filled portion — gold when active, indigo-blue otherwise.
                let fill_w = (card_w - 8.0) * p.progress;
                if fill_w > 0.5 {
                    let fill_color = if p.active {
                        [0.95, 0.78, 0.25, 1.0]
                    } else {
                        [0.45, 0.55, 0.85, 0.9]
                    };
                    instances.push(GpuInstance {
                        rect: [cx + 4.0, fill_y, fill_w, fill_h],
                        color: fill_color,
                    });
                }
                // Top text: yaku name + bonus value.
                let name_h = panel_h * 0.42;
                let name_color = if p.active {
                    [1.0, 0.92, 0.55, 1.0]
                } else {
                    [0.78, 0.80, 0.90, 1.0]
                };
                yaku_text_labels.push(TextLabel {
                    rect: [cx + 4.0, cy + 2.0, card_w - 8.0, name_h],
                    text: format!("{} +{}", p.kind.name(), p.kind.bonus_points()),
                    color: name_color,
                });
                // Hint text below the name, above the progress bar.
                let hint_h = panel_h - name_h - fill_h - 6.0;
                let hint_y = cy + name_h + 2.0;
                let hint_color = if p.active {
                    [1.0, 0.85, 0.45, 1.0]
                } else {
                    [0.62, 0.66, 0.78, 1.0]
                };
                yaku_text_labels.push(TextLabel {
                    rect: [cx + 4.0, hint_y, card_w - 8.0, hint_h.max(8.0)],
                    text: p.hint.clone(),
                    color: hint_color,
                });
            }
        }

        let paused = self.pause_menu.paused;
        let btn_rects = [
            suit_btn_rect,
            rank_btn_rect,
            play_btn_rect,
            discard_btn_rect,
        ];
        let base_colors: [[f32; 4]; BTN_COUNT] = [
            [0.22, 0.38, 0.55, 0.92], // sort suit
            [0.22, 0.38, 0.55, 0.92], // sort rank
            if selection_valid && run.plays_remaining > 0 {
                [0.18, 0.55, 0.25, 0.92]
            } else {
                [0.35, 0.35, 0.35, 0.60]
            },
            if selected_count > 0 && run.discards_remaining > 0 {
                [0.65, 0.18, 0.18, 0.92]
            } else {
                [0.35, 0.35, 0.35, 0.60]
            },
        ];
        for (i, &(bx, by, bw, bh)) in btn_rects.iter().enumerate() {
            // Draw a highlight border if this button is focused.
            if self.button_focus == Some(i) {
                let pad = 3.0;
                instances.push(GpuInstance {
                    rect: [bx - pad, by - pad, bw + pad * 2.0, bh + pad * 2.0],
                    color: [0.9, 0.8, 0.2, 0.95],
                });
            }
            instances.push(GpuInstance {
                rect: [bx, by, bw, bh],
                color: base_colors[i],
            });
        }

        let mut text_labels = vec![
            // Score panel
            TextLabel {
                rect: [
                    ms.x,
                    layout.score_panel.y,
                    ms.w * 0.75,
                    layout.score_panel.h,
                ],
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
        text_labels.extend(yaku_text_labels);
        text_labels.push(TextLabel {
            rect: [
                suit_btn_rect.0,
                suit_btn_rect.1,
                suit_btn_rect.2,
                suit_btn_rect.3,
            ],
            text: "Sort by Suit".into(),
            color: [1.0, 1.0, 1.0, 1.0],
        });
        text_labels.push(TextLabel {
            rect: [
                rank_btn_rect.0,
                rank_btn_rect.1,
                rank_btn_rect.2,
                rank_btn_rect.3,
            ],
            text: "Sort by Rank".into(),
            color: [1.0, 1.0, 1.0, 1.0],
        });
        let play_label = "Play Hand".into();
        text_labels.push(TextLabel {
            rect: [
                play_btn_rect.0,
                play_btn_rect.1,
                play_btn_rect.2,
                play_btn_rect.3,
            ],
            text: play_label,
            color: [1.0, 1.0, 1.0, 1.0],
        });
        let discard_label = "Discard".into();
        text_labels.push(TextLabel {
            rect: [
                discard_btn_rect.0,
                discard_btn_rect.1,
                discard_btn_rect.2,
                discard_btn_rect.3,
            ],
            text: discard_label,
            color: [1.0, 1.0, 1.0, 1.0],
        });

        // While paused, don't expose gameplay buttons as clickable — the
        // pause overlay (added below) swallows all input via its own buttons
        // plus a fullscreen blocker. Otherwise a click on Discard while
        // paused would push CommitDiscard, which the pause menu interprets
        // as "activate focused item".
        let mut buttons: Vec<ButtonDef> = if paused {
            Vec::new()
        } else {
            vec![
                ButtonDef::ui(suit_btn_rect, UiAction::SortBySuit),
                ButtonDef::ui(rank_btn_rect, UiAction::SortByRank),
                ButtonDef::ui(play_btn_rect, UiAction::ScoreHand),
                ButtonDef::ui(discard_btn_rect, UiAction::CommitDiscard),
            ]
        };

        // Particle instances.
        for (rect, color) in self.particles.instances() {
            instances.push(GpuInstance { rect, color });
        }

        // Hint: compute tile indices that would complete a meld with current selection.
        let selected_indices: Vec<usize> = run
            .selected
            .iter()
            .enumerate()
            .filter(|&(_, &sel)| sel)
            .map(|(i, _)| i)
            .collect();
        let hint_indices = if !selected_indices.is_empty() && self.cascade.is_none() {
            suggest_completions(&run.hand, &selected_indices)
        } else {
            vec![]
        };

        // Tile hover tooltip — show point value of the tile under the cursor.
        // Suppressed during cascade and while the pause menu is open.
        if self.cascade.is_none() && !self.pause_menu.paused {
            let (cx, cy) = self.cursor_pos;
            let hovered = hand_slots
                .iter()
                .enumerate()
                .find(|&(_, &(sx, sy, sw, sh))| {
                    cx >= sx && cx <= sx + sw && cy >= sy && cy <= sy + sh
                });
            if let Some((idx, &(sx, sy, _sw, sh))) = hovered {
                if let Some(tile) = run.hand.get(idx) {
                    let pts = tile.point_value();
                    let kind = match tile.suit {
                        crate::core::tile::Suit::Wind | crate::core::tile::Suit::Dragon => {
                            format!("{} (honor)", tile.label())
                        }
                        _ => tile.label(),
                    };
                    let text = format!("{kind}: {pts} pts");

                    let tw = (text.len() as f32 * 8.0 * scale).max(80.0 * scale);
                    let th = 22.0 * scale;
                    let mut tx = sx;
                    let mut ty = sy - th - 6.0 * scale;
                    if ty < 4.0 {
                        ty = sy + sh + 6.0 * scale;
                    }
                    if tx + tw > layout.window_w - 4.0 {
                        tx = layout.window_w - tw - 4.0;
                    }
                    if tx < 4.0 {
                        tx = 4.0;
                    }

                    // Background.
                    instances.push(GpuInstance {
                        rect: [tx, ty, tw, th],
                        color: [0.06, 0.06, 0.12, 0.95],
                    });
                    // Gold border.
                    let bc = [0.65, 0.55, 0.25, 0.85];
                    let b = 1.5;
                    instances.push(GpuInstance { rect: [tx, ty, tw, b], color: bc });
                    instances.push(GpuInstance { rect: [tx, ty + th - b, tw, b], color: bc });
                    instances.push(GpuInstance { rect: [tx, ty, b, th], color: bc });
                    instances.push(GpuInstance { rect: [tx + tw - b, ty, b, th], color: bc });

                    text_labels.push(TextLabel {
                        rect: [tx + 4.0 * scale, ty, tw - 8.0 * scale, th],
                        text,
                        color: [0.95, 0.85, 0.4, 1.0],
                    });
                }
            }
        }

        // Pause overlay.
        self.pause_menu.draw(
            layout.window_w,
            layout.window_h,
            scale,
            &mut instances,
            &mut text_labels,
            &mut buttons,
        );

        // Fullscreen click-blocker behind the pause menu's own buttons.
        // Buttons are hit-tested in vec order with first-match-wins, and the
        // pause menu just appended its buttons above, so this blocker (added
        // last) only catches clicks that miss every pause-menu button. It
        // uses an unused Scene id so the gameplay scene treats the click as
        // a no-op instead of toggling tile selection or starting a drag.
        if paused {
            buttons.push(ButtonDef::scene(
                (0.0, 0.0, layout.window_w, layout.window_h),
                u32::MAX,
            ));
        }

        SceneDrawOutput {
            background: super::BackgroundId::Gameplay,
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
                run.blind.name(),
                run.run_number,
                shown_score,
                run.target_score,
                run.gold,
                run.plays_remaining,
                run.discards_remaining
            ),
            departing_indices: self.pending_departures.clone(),
            hint_indices,
        }
    }
}
