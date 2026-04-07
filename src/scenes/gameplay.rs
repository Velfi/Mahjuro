//! Gameplay scene — the main tile-playing loop.

use std::time::Instant;

use crate::core::hand::{describe_hand, suggest_completions};
use crate::core::scoring::StepKind;
use crate::core::yaku::yaku_preview;
use crate::game::cascade::ScoringCascade;
use crate::game::run::{STARTING_DISCARDS, STARTING_PLAYS};
use crate::render::animation::ENTITY_SCORE_PANEL;
use crate::render::candle_mesh::{CandlePlacement, WICK_TIP_Y};
use crate::render::particles::ParticleSystem;
use crate::render::wgpu_renderer::{
    GpuInstance, PointLight, TextLabel, build_instances_from_layout,
};
use crate::ui::input::{UiAction, apply_ui_actions};

use super::pause_menu::{PauseMenu, PauseUpdate};
use super::{ButtonDef, DrawCtx, SceneDrawOutput, SceneTransition, UpdateCtx, relic_row};

/// One candle's animated state. The renderer's flame shader does the heavy
/// lifting (procedural fbm noise + flicker), but each candle still needs a
/// stable random phase so neighbouring candles don't beat in lockstep, and a
/// CPU-side flicker accumulator so the matching point light's intensity
/// pulses in sync with what the player sees on screen.
#[derive(Clone, Copy)]
struct CandleState {
    /// Random phase offset in [0, TAU).  Mixed into the flame shader's noise
    /// coordinates and into the CPU flicker term so each candle is unique.
    phase: f32,
    /// Smoothed intensity multiplier — wanders ~[0.7, 1.1] driven by a sum
    /// of two sines so it never repeats audibly.
    flicker: f32,
}

impl CandleState {
    fn new(phase: f32) -> Self {
        Self {
            phase,
            flicker: 1.0,
        }
    }
}

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
    /// Previous frame's displayed score; used to detect changes and fire the
    /// score-pop tween on the score panel.
    prev_displayed_score: u32,
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
    /// When set, the hand has been discarded-from but not yet refilled. The
    /// auto-draw fires once `Instant::now()` reaches this deadline, giving the
    /// discard departure animation time to play out.
    pending_refill: Option<Instant>,
    /// Latest cursor position (window coords), captured each update for hover tooltips.
    cursor_pos: (f32, f32),
    /// Animated state for the four ambient candles flanking the play area.
    /// Updated every frame; consumed in `draw()` to position flames + lights.
    candles: [CandleState; 4],
    /// Wall-clock time used to advance candle flicker (independent of the
    /// game's `Instant::now()` references so the candles keep moving even if
    /// the game logic is paused).
    candle_time: f32,
}

impl GameplayScene {
    pub fn new() -> Self {
        Self {
            cascade: None,
            displayed_score: 0,
            prev_displayed_score: 0,
            particles: ParticleSystem::new(),
            last_frame: Instant::now(),
            pause_menu: PauseMenu::new(),
            button_focus: None,
            pending_departures: Vec::new(),
            pending_refill: None,
            cursor_pos: (0.0, 0.0),
            // Four candles with golden-ratio spaced phases so their flicker
            // never visually syncs up. Phases are in [0, TAU).
            candles: [
                CandleState::new(0.0),
                CandleState::new(1.7),
                CandleState::new(3.9),
                CandleState::new(5.2),
            ],
            candle_time: 0.0,
        }
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.particles.update(dt);
        self.cursor_pos = ctx.cursor_pos;

        // Advance candle flicker. Each candle's `flicker` is a smoothed
        // approach toward a target wave (sum of two sines per candle), so
        // intensity wanders organically in roughly [0.7, 1.1].
        self.candle_time += dt;
        for c in self.candles.iter_mut() {
            let t = self.candle_time;
            let target =
                0.9 + 0.18 * (t * 7.3 + c.phase).sin() + 0.08 * (t * 13.1 + c.phase * 1.7).sin();
            // Exponential smoothing — keeps the light from snapping.
            let k = (1.0 - (-dt * 12.0).exp()).clamp(0.0, 1.0);
            c.flicker += (target - c.flicker) * k;
        }

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
            // Stronger score-pop whenever the displayed value actually
            // advances — this is what makes scoring *feel* like the number
            // is climbing on the cartouche.
            if self.displayed_score != self.prev_displayed_score {
                ctx.anim.score_pop(ENTITY_SCORE_PANEL);
                self.prev_displayed_score = self.displayed_score;
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

        // Catch any score change that didn't come through the cascade path
        // (round resets, instant-grant relic bonuses). Same effect: pop the panel.
        if self.displayed_score != self.prev_displayed_score {
            ctx.anim.score_pop(ENTITY_SCORE_PANEL);
            self.prev_displayed_score = self.displayed_score;
        }

        // If a discard is waiting for its departure animation to play out,
        // hold input until the deadline passes, then auto-draw replacements.
        if let Some(deadline) = self.pending_refill {
            if now >= deadline {
                ctx.run.refill_hand(ctx.bus);
                ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
                self.pending_refill = None;
            } else {
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
                    // Remove the tiles immediately, but defer the auto-draw
                    // until the departure animation has had time to play.
                    let discarded = ctx.run.discard_selected_no_refill(ctx.bus);
                    if discarded > 0 {
                        ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
                        let depart_lifetime =
                            std::time::Duration::from_millis(ctx.cascade_tuning.depart_lifetime_ms);
                        self.pending_refill = Some(now + depart_lifetime);
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
        self.cascade.is_some() || self.particles.is_active() || self.pending_refill.is_some()
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
            "{}  ·  R{}  ·  {} / {}  ·  ${}  ·  W{}{}",
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

        // Dynamic readout buffers — when populated by the cascade branch, they
        // replace the legacy single-line cascade text label (which becomes "").
        let mut cascade_instances: Vec<GpuInstance> = Vec::new();
        let mut cascade_labels: Vec<TextLabel> = Vec::new();

        // Build cascade / sets text for left panel.
        let (cascade_text, cascade_color) = if let Some(ref cascade) = self.cascade {
            let frame = cascade.frame(now);
            let score_before = cascade.score_before;
            // ── Two-axis Balatro-style readout ───────────────────────────
            // Layout (within the modifier strip):
            //   top 36%  → source label (the relic / yaku that just fired)
            //   bot 64%  → [ chips pill ]  ×  [ mult pill ]   = total
            let src_h = ms.h * 0.36;
            let pill_y = ms.y + src_h;
            let pill_h = (ms.h - src_h - 2.0).max(8.0);
            // Pills span the centre 80% of the strip, leaving room for "= N".
            let inner_w = ms.w * 0.80;
            let inner_x = ms.x + (ms.w - inner_w) * 0.5;
            let cross_w = pill_h * 0.7;
            let pill_w = ((inner_w - cross_w) * 0.5).max(20.0);

            // Pulse envelope: fast pop-in then settle. Active pill grows ~12%.
            let pulse_strength = (1.0 - frame.phase_t * 1.6).clamp(0.0, 1.0);
            let chip_pulse = if frame.pulse_axis == Some(StepKind::Chips) {
                1.0 + 0.12 * pulse_strength
            } else {
                1.0
            };
            let mult_pulse = if frame.pulse_axis == Some(StepKind::Mult) {
                1.0 + 0.12 * pulse_strength
            } else {
                1.0
            };

            // Chips pill — cool indigo background, gold number.
            let chips_x = inner_x;
            {
                let scaled_w = pill_w * chip_pulse;
                let scaled_h = pill_h * chip_pulse;
                let cx = chips_x + pill_w * 0.5;
                let cy = pill_y + pill_h * 0.5;
                let rect = [cx - scaled_w * 0.5, cy - scaled_h * 0.5, scaled_w, scaled_h];
                cascade_instances.push(GpuInstance {
                    rect,
                    color: [0.18, 0.24, 0.42, 0.95],
                });
                cascade_labels.push(TextLabel {
                    rect,
                    text: format!("{}", frame.displayed_chips),
                    color: [1.0, 0.93, 0.55, 1.0],
                });
            }

            // Mult pill — warm crimson background, soft pink number.
            let mult_x = inner_x + pill_w + cross_w;
            let mult_value = frame.displayed_mult;
            let mult_str = if (mult_value - mult_value.round()).abs() < 0.05 {
                format!("×{}", mult_value.round() as i64)
            } else {
                format!("×{:.1}", mult_value)
            };
            {
                let scaled_w = pill_w * mult_pulse;
                let scaled_h = pill_h * mult_pulse;
                let cx = mult_x + pill_w * 0.5;
                let cy = pill_y + pill_h * 0.5;
                let rect = [cx - scaled_w * 0.5, cy - scaled_h * 0.5, scaled_w, scaled_h];
                cascade_instances.push(GpuInstance {
                    rect,
                    color: [0.45, 0.16, 0.22, 0.95],
                });
                cascade_labels.push(TextLabel {
                    rect,
                    text: mult_str,
                    color: [1.0, 0.85, 0.85, 1.0],
                });
            }

            // Big "×" sigil between the pills.
            cascade_labels.push(TextLabel {
                rect: [inner_x + pill_w, pill_y, cross_w, pill_h],
                text: "×".into(),
                color: [0.85, 0.85, 0.95, 1.0],
            });

            // "= score-this-hand" tail on the right edge of the strip.
            let total_x = inner_x + inner_w + 4.0;
            let total_w = ms.x + ms.w - total_x;
            if total_w > 24.0 {
                let earned_so_far = frame.displayed_score.saturating_sub(score_before);
                cascade_labels.push(TextLabel {
                    rect: [total_x, pill_y, total_w, pill_h],
                    text: format!("= {}", earned_so_far),
                    color: [1.0, 0.92, 0.45, 1.0],
                });
            }

            // Source label — the relic / yaku that just fired. Fades in over
            // phase_t and is colour-coded to the axis it hit.
            if let Some((source, kind)) = frame.latest_step.as_ref() {
                let fade = (frame.phase_t * 2.5).clamp(0.0, 1.0);
                let label_color = match kind {
                    StepKind::Chips => [0.75, 0.88, 1.0, fade],
                    StepKind::Mult => [1.0, 0.78, 0.82, fade],
                    StepKind::Final => [1.0, 0.92, 0.45, fade],
                };
                cascade_labels.push(TextLabel {
                    rect: [ms.x, ms.y + 2.0, ms.w, src_h - 4.0],
                    text: source.clone(),
                    color: label_color,
                });
            }

            // Yaku tag in the left gutter so the player can see what patterns landed.
            if let Some(ref bd) = run.last_breakdown {
                if !bd.detected_yaku.is_empty() {
                    let yaku_names: Vec<_> = bd.detected_yaku.iter().map(|y| y.name()).collect();
                    let gutter_w = (inner_x - ms.x - 8.0).max(0.0);
                    if gutter_w > 30.0 {
                        cascade_labels.push(TextLabel {
                            rect: [ms.x + 4.0, pill_y, gutter_w, pill_h],
                            text: format!("[{}]", yaku_names.join(", ")),
                            color: [0.85, 0.78, 1.0, 0.9],
                        });
                    }
                }
            }

            // Empty text — the dynamic widgets above replace the legacy line.
            (String::new(), [1.0, 1.0, 1.0, 1.0])
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
                let preview = run.preview_selection();
                let valid = preview.is_some();
                let text = if hand_desc.is_empty() {
                    "No valid melds".to_string()
                } else if let Some(p) = preview {
                    let mult_str = if (p.mult - p.mult.round()).abs() < 1e-6 {
                        format!("{}", p.mult.round() as i64)
                    } else {
                        format!("{:.1}", p.mult)
                    };
                    format!("{hand_desc}  ▸  {} × {}", p.chips, mult_str)
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
                    text: format!("{} +{} mult", p.kind.name(), p.kind.mult_bonus()),
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

        // Cascade pill backgrounds (drawn before relics so the relic row reads cleanly).
        instances.extend(cascade_instances);

        // Score-panel text fits inside the narrow centered cartouche painted
        // by `build_instances_from_layout`. Cartouche geometry mirrors that
        // function: 38% width × 78% height of the score-panel region, applied
        // to the same scale-pop transform so the text grows with the
        // cartouche on score changes.
        let sp = layout.score_panel;
        let cart_w = sp.w * 0.38;
        let cart_h = sp.h * 0.78;
        let cart_x = sp.x + (sp.w - cart_w) * 0.5;
        let cart_y = sp.y + (sp.h - cart_h) * 0.5;
        let (ctx_x, ctx_y, ctx_w, ctx_h) =
            crate::render::animation::apply_transform_rect(cart_x, cart_y, cart_w, cart_h, ts);
        let mut text_labels = vec![
            // Score panel — single line of gold serif inside the cartouche.
            TextLabel {
                rect: [ctx_x, ctx_y, ctx_w, ctx_h],
                text: score_text,
                color: crate::render::theme::color::CHAMPAGNE,
            },
        ];
        // Modifier strip: dynamic cascade widgets (when active) or status text (idle).
        if cascade_labels.is_empty() {
            text_labels.push(TextLabel {
                rect: [ms.x, ms.y, ms.w, ms.h],
                text: cascade_text,
                color: cascade_color,
            });
        } else {
            text_labels.extend(cascade_labels);
        }
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

        // Tile hover tooltip — show full info for the tile under the cursor.
        // Anchored to the perspective-projected tile rect (one frame stale,
        // supplied by the renderer) so it tracks the actual visible tile
        // position rather than the flat layout slot. Falls back to the slot
        // rect on the very first frame before the renderer has projected.
        // Suppressed during cascade and while the pause menu is open.
        if self.cascade.is_none() && !self.pause_menu.paused {
            let (cx, cy) = self.cursor_pos;
            // Hit-test against the projected rects when available, since
            // those reflect what the user actually sees. Fall back to the
            // flat slots if the renderer hasn't projected yet.
            let hovered_idx: Option<usize> = if !ctx.projected_hand_rects.is_empty() {
                ctx.projected_hand_rects
                    .iter()
                    .find(|(_, r)| {
                        cx >= r[0] && cx <= r[0] + r[2] && cy >= r[1] && cy <= r[1] + r[3]
                    })
                    .map(|(i, _)| *i)
            } else {
                hand_slots
                    .iter()
                    .enumerate()
                    .find(|&(_, &(sx, sy, sw, sh))| {
                        cx >= sx && cx <= sx + sw && cy >= sy && cy <= sy + sh
                    })
                    .map(|(i, _)| i)
            };

            if let Some(idx) = hovered_idx {
                if let Some(tile) = run.hand.get(idx) {
                    // Resolve the anchor rect: prefer the projected rect for
                    // this index, otherwise the flat slot rect.
                    let anchor: (f32, f32, f32, f32) = ctx
                        .projected_hand_rects
                        .iter()
                        .find(|(i, _)| *i == idx)
                        .map(|(_, r)| (r[0], r[1], r[2], r[3]))
                        .or_else(|| hand_slots.get(idx).copied())
                        .unwrap_or((0.0, 0.0, 0.0, 0.0));
                    let (ax, ay, aw, ah) = anchor;

                    // ── Build the lines ───────────────────────────────
                    let pts = tile.point_value();
                    let name = tile.full_name();
                    let category = tile.category();
                    let is_selected = run.selected.get(idx).copied().unwrap_or(false);

                    let mut lines: Vec<String> = Vec::new();
                    lines.push(name);
                    lines.push(format!("{category} · {pts} pts"));
                    if is_selected {
                        lines.push("selected".to_string());
                    }

                    // ── Geometry ──────────────────────────────────────
                    let line_h = 18.0 * scale;
                    let pad_x = 8.0 * scale;
                    let pad_y = 6.0 * scale;
                    let widest = lines
                        .iter()
                        .map(|s| s.chars().count())
                        .max()
                        .unwrap_or(0) as f32;
                    let tw = (widest * 7.5 * scale + pad_x * 2.0).max(120.0 * scale);
                    let th = line_h * lines.len() as f32 + pad_y * 2.0;

                    // Position: centred horizontally over the anchor,
                    // floating just above its top edge. Flip below the
                    // anchor if there isn't room above.
                    let mut tx = ax + (aw - tw) * 0.5;
                    let mut ty = ay - th - 6.0 * scale;
                    if ty < 4.0 {
                        ty = ay + ah + 6.0 * scale;
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
                    instances.push(GpuInstance {
                        rect: [tx, ty, tw, b],
                        color: bc,
                    });
                    instances.push(GpuInstance {
                        rect: [tx, ty + th - b, tw, b],
                        color: bc,
                    });
                    instances.push(GpuInstance {
                        rect: [tx, ty, b, th],
                        color: bc,
                    });
                    instances.push(GpuInstance {
                        rect: [tx + tw - b, ty, b, th],
                        color: bc,
                    });

                    // Text lines. First line uses the suit colour as a
                    // visual cue (matches the new dragon-by-rank palette);
                    // subsequent lines are the standard ivory-gold.
                    let suit_rgba = tile.suit_color();
                    let title_color = [
                        (suit_rgba[0] * 0.6 + 0.4).min(1.0),
                        (suit_rgba[1] * 0.6 + 0.4).min(1.0),
                        (suit_rgba[2] * 0.6 + 0.4).min(1.0),
                        1.0,
                    ];
                    let body_color = [0.95, 0.85, 0.4, 1.0];
                    for (i, line) in lines.into_iter().enumerate() {
                        let color = if i == 0 { title_color } else { body_color };
                        text_labels.push(TextLabel {
                            rect: [
                                tx + pad_x,
                                ty + pad_y + i as f32 * line_h,
                                tw - pad_x * 2.0,
                                line_h,
                            ],
                            text: line,
                            color,
                        });
                    }
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

        // ── Candles ─────────────────────────────────────────────────────
        // Four ambient candles flank the play area: one on each side of the
        // score panel up top, one on each side of the hand strip down below.
        // Each candle pushes a 3D `CandlePlacement` (rendered via the
        // lit-mesh pipeline), an additive `Flame` quad, and a matching
        // `PointLight` so the 3D tile + table shaders pick up the warmth.
        let mut flame_instances: Vec<GpuInstance> = Vec::new();
        let mut point_lights: Vec<PointLight> = Vec::new();
        let mut candle_placements: Vec<CandlePlacement> = Vec::new();
        let scale_c = (layout.window_w / 600.0).max(0.5);
        // The wax mesh is now ~0.555 tall and ~0.36 wide in local units
        // (votive proportions), so `candle_scale` (= the uniform mesh
        // scale) is the height of the votive in pixels.
        let candle_h = 115.0 * scale_c;
        let candle_w = candle_h * 0.72; // votive footprint, used for layout padding
        let flame_w = 24.0 * scale_c;
        let flame_h = 40.0 * scale_c;
        let radius_px = 360.0 * scale_c;
        let edge_pad = 18.0 * scale_c;
        // Position centres for each candle: (cx, cy_base) where cy_base is
        // the bottom of the candle body. Order matches `self.candles[]`.
        let sp = layout.score_panel;
        let strip_y = if let Some(first) = layout.hand_slots.first() {
            first.y + first.h * 0.5
        } else {
            layout.window_h - 100.0 * scale_c
        };
        let strip_left = layout.hand_slots.first().map(|r| r.x).unwrap_or(0.0);
        let strip_right = layout
            .hand_slots
            .last()
            .map(|r| r.x + r.w)
            .unwrap_or(layout.window_w);
        // Bottom candles are pushed well outboard from the hand strip
        // *and* shifted backward in Z (smaller pixel-y → farther from
        // the camera), so they sit behind the tile strip in depth as
        // well as beside it horizontally. Without the Z shift, even an
        // outboard candle's tall silhouette can creep into the tile
        // sightline because the camera is in front of and slightly
        // above the table — putting them at strip_y meant the candle's
        // wick projected into the same screen band as the tiles.
        let bottom_pad = edge_pad + candle_w * 1.6;
        let bottom_z_back = candle_h * 0.55; // shift back along table-Z
        let candle_centers: [(f32, f32); 4] = [
            // Score panel left
            (
                (sp.x - candle_w * 0.5 - edge_pad).max(candle_w * 0.5 + 4.0),
                sp.y + sp.h * 0.5,
            ),
            // Score panel right
            (
                (sp.x + sp.w + candle_w * 0.5 + edge_pad)
                    .min(layout.window_w - candle_w * 0.5 - 4.0),
                sp.y + sp.h * 0.5,
            ),
            // Hand strip left — far outboard and recessed behind the strip
            (
                (strip_left - candle_w * 0.5 - bottom_pad).max(candle_w * 0.5 + 4.0),
                strip_y - bottom_z_back,
            ),
            // Hand strip right — far outboard and recessed behind the strip
            (
                (strip_right + candle_w * 0.5 + bottom_pad)
                    .min(layout.window_w - candle_w * 0.5 - 4.0),
                strip_y - bottom_z_back,
            ),
        ];

        // The candles stand vertically on the (now horizontal) wood table.
        // We pass them to the renderer in pixel-layout coordinates: the
        // first two components are the layout x/y of the candle's base on
        // the table (renderer maps pixel y → table z = front/back), and
        // the third component is the height above the wood (always 0 so
        // the wax base sits flush on the table).
        //
        // The candle mesh's local Y axis is "up", so once the renderer
        // translates to (table_x, 0, table_z), scaling by `candle_scale`
        // gives a candle of approximately `candle_scale` pixels in
        // visible height. The wick tip lives at WICK_TIP_Y * scale above
        // the base in true world units.
        // Cheap deterministic hash → 3 pseudorandom values in [-1, 1].
        // Used to jitter candle position and scale per index so the four
        // votives don't snap to a perfectly symmetric grid.
        fn candle_jitter(seed: u32) -> (f32, f32, f32) {
            let s = seed.wrapping_add(1);
            let h = |k: u32| -> f32 {
                let mut x = s.wrapping_mul(k) ^ 0x9E3779B9;
                x ^= x >> 16;
                x = x.wrapping_mul(0x7feb352d);
                x ^= x >> 15;
                x = x.wrapping_mul(0x846ca68b);
                x ^= x >> 16;
                ((x as f32) / (u32::MAX as f32)) * 2.0 - 1.0
            };
            (h(0x68E31DA4), h(0xB5297A4D), h(0x1B56C4E9))
        }

        let candle_scale_base = candle_h;
        for (i, &(cx, cy_anchor)) in candle_centers.iter().enumerate() {
            let candle = self.candles[i];

            // Per-candle jitter. Bottom (front) candles get directionally
            // constrained offsets so they never drift forward into the
            // tile sightline; top candles can wander freely.
            let (jx, jy, js) = candle_jitter(i as u32);
            let (jitter_x_pix, jitter_y_pix) = match i {
                0 | 1 => (jx * 22.0 * scale_c, jy * 16.0 * scale_c),
                2 => (-jx.abs() * 26.0 * scale_c, -jy.abs() * 22.0 * scale_c),
                _ => (jx.abs() * 26.0 * scale_c, -jy.abs() * 22.0 * scale_c),
            };
            let cx_j = cx + jitter_x_pix;
            let cy_j = cy_anchor + jitter_y_pix;
            // ±12% scale variation so the four votives read as a real
            // set rather than four identical instances.
            let candle_scale = candle_scale_base * (1.0 + js * 0.12);

            candle_placements.push(CandlePlacement {
                world_pos: [cx_j, cy_j, 0.0],
                scale: candle_scale,
                flicker: candle.flicker,
            });

            // The flame sprite is still drawn by the additive 2D flame
            // pipeline, so it lives in screen pixel space. The renderer
            // overrides each flame's rect with the projected wick tip
            // (using the jittered candle world_pos + scale we just
            // pushed), so the values here are just sane defaults for
            // any code path that bypasses the override.
            let phase01 = (candle.phase / std::f32::consts::TAU).fract().abs();
            let flame_x = cx_j - flame_w * 0.5;
            let flame_y = cy_j - flame_h * 1.2;
            flame_instances.push(GpuInstance {
                rect: [flame_x, flame_y, flame_w, flame_h],
                color: [1.0, 1.0, 1.0, phase01],
            });

            // Point light at the wick tip — sits at world_y =
            // WICK_TIP_Y * candle_scale above the table, at the candle's
            // jittered table-plane (cx_j, cy_j) anchor. The renderer
            // maps the pixel-layout x/y onto the table.
            let wick_world_y = WICK_TIP_Y * candle_scale;
            point_lights.push(PointLight {
                pos: [cx_j, cy_j, wick_world_y],
                radius: radius_px * (0.85 + 0.3 * candle.flicker),
                color: [1.0, 0.55, 0.22],
                intensity: 1.6 * candle.flicker,
            });
            let _ = candle_w;
        }

        // The 3D table + tiles + candles ARE the UI. We don't paint a 2D
        // "tray" or per-slot pockets here — those would overdraw the 3D
        // scene. The only 2D add-on is a thin gold rim around each
        // *selected* tile, drawn as a screen-space overlay above the tile.
        // We use the renderer-provided `projected_hand_rects` (the actual
        // visible rect of the 3D tile this frame) so the rim hugs the tile
        // wherever the perspective camera puts it.
        let mut selection_overlay: Vec<GpuInstance> = Vec::new();
        for (i, &is_sel) in run.selected.iter().enumerate() {
            if !is_sel {
                continue;
            }
            // Prefer the projected 3D rect; fall back to the layout slot if
            // the renderer hasn't projected it yet (first frame).
            let rect = ctx
                .projected_hand_rects
                .iter()
                .find_map(|(idx, r)| if *idx == i { Some(*r) } else { None })
                .or_else(|| layout.hand_slots.get(i).map(|s| [s.x, s.y, s.w, s.h]));
            let Some([rx, ry, rw, rh]) = rect else {
                continue;
            };
            let rt = (rh * 0.020).clamp(2.0, 4.0);
            let rim = crate::render::theme::color::CHAMPAGNE;
            // Top
            selection_overlay.push(GpuInstance {
                rect: [rx, ry, rw, rt],
                color: rim,
            });
            // Bottom
            selection_overlay.push(GpuInstance {
                rect: [rx, ry + rh - rt, rw, rt],
                color: rim,
            });
            // Left
            selection_overlay.push(GpuInstance {
                rect: [rx, ry + rt, rt, rh - 2.0 * rt],
                color: rim,
            });
            // Right
            selection_overlay.push(GpuInstance {
                rect: [rx + rw - rt, ry + rt, rt, rh - 2.0 * rt],
                color: rim,
            });
        }
        // The selection rim should appear *over* the 3D tile, so it goes
        // into the regular `instances` vec (which is drawn after the hand
        // tile backdrop in `into_frame`). Splice it in front of the
        // existing 2D HUD quads so it sits below the score panel etc.
        let mut combined_instances = selection_overlay;
        combined_instances.append(&mut instances);
        let instances = combined_instances;

        SceneDrawOutput {
            background: super::BackgroundId::Gameplay,
            tray_instances: Vec::new(),
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
            flame_instances,
            point_lights,
            candles: candle_placements,
            draw_table: true,
        }
    }
}
