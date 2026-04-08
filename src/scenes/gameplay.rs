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
    GpuInstance, PointLight, TextAlign, TextLabel, build_instances_from_layout,
};
use crate::render::theme::typography;
use crate::ui::input::{UiAction, apply_ui_actions};
use crate::ui::widget::{self, TextStyle};

use super::pause_menu::PauseMenu;
use super::{ButtonDef, DrawCtx, SceneBehavior, SceneDrawOutput, SceneTransition, UpdateCtx};

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

/// Bottom-row gameplay buttons. Order in `ALL_BUTTONS` is the keyboard nav order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GameplayButton {
    SortSuit,
    SortRank,
    Play,
    Discard,
}

const ALL_BUTTONS: [GameplayButton; 4] = [
    GameplayButton::SortSuit,
    GameplayButton::SortRank,
    GameplayButton::Play,
    GameplayButton::Discard,
];

impl GameplayButton {
    fn ui_action(self) -> UiAction {
        match self {
            GameplayButton::SortSuit => UiAction::SortBySuit,
            GameplayButton::SortRank => UiAction::SortByRank,
            GameplayButton::Play => UiAction::ScoreHand,
            GameplayButton::Discard => UiAction::CommitDiscard,
        }
    }
}

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
    button_focus: Option<GameplayButton>,
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
    /// Per-relic glow start times. Populated as the cascade reveals each step
    /// whose source matches a relic display name. The glow fades over
    /// `RELIC_GLOW_LIFETIME` and the entry is evicted afterward.
    relic_glow_starts: std::collections::HashMap<crate::core::relic::RelicId, Instant>,
    /// Tracks the most recent step index whose reveal edge we've already
    /// processed (relic glow + ScoreStepRevealed bus event). Prevents
    /// re-firing reveal-edge effects every frame while the cascade holds
    /// on the same step.
    last_revealed_step: Option<usize>,
    /// Whether the current cascade has already emitted its `ScoreCascadeFinal`
    /// event. Reset when a new cascade starts.
    cascade_final_emitted: bool,
    /// Cross-input glossary overlay (`?` / F1 / H to toggle).
    glossary: super::glossary::GlossaryOverlay,
    /// Hand size observed last frame. Used to detect deal events: any time
    /// the hand grows (initial round deal, post-discard refill) we stamp
    /// `last_deal_at` so the post-deal smoke breath can fire.
    prev_hand_len: usize,
    /// When the most recent deal landed. The post-deal smoke gust starts
    /// `POST_DEAL_GUST_DELAY` after this and tapers off over
    /// `POST_DEAL_GUST_DURATION`. `None` between rounds and after the gust
    /// has finished playing.
    last_deal_at: Option<Instant>,
}

/// How long after a deal to wait before exhaling on the smoke.
const POST_DEAL_GUST_DELAY: f32 = 3.0;
/// Duration of the post-deal exhale once it begins.
const POST_DEAL_GUST_DURATION: f32 = 1.4;

/// How long a relic glow lingers after activation.
const RELIC_GLOW_LIFETIME: std::time::Duration = std::time::Duration::from_millis(900);

/// Click-id base for the Zodiac inventory bar. `ZODIAC_USE_BASE + slot_idx`
/// is the click id for using the Zodiac in slot `slot_idx`.
const ZODIAC_USE_BASE: u32 = 0x9000;

/// Click id for the `?` glossary badge in the gameplay HUD.
const HELP_BADGE_ID: u32 = 0x9100;

impl GameplayScene {
    /// Whether the cascade is actively animating (for redraw requests).
    pub fn is_animating(&self) -> bool {
        self.cascade.is_some()
            || self.particles.is_active()
            || self.pending_refill.is_some()
            || !self.relic_glow_starts.is_empty()
            || self.post_deal_gust_active()
    }

    /// True while the post-deal smoke breath is either pending or actively
    /// blowing. Keeps the main loop ticking through the delay so the gust
    /// actually fires on idle frames.
    fn post_deal_gust_active(&self) -> bool {
        let Some(t) = self.last_deal_at else {
            return false;
        };
        let elapsed = Instant::now().saturating_duration_since(t).as_secs_f32();
        elapsed < POST_DEAL_GUST_DELAY + POST_DEAL_GUST_DURATION
    }

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
            relic_glow_starts: std::collections::HashMap::new(),
            last_revealed_step: None,
            cascade_final_emitted: false,
            glossary: super::glossary::GlossaryOverlay::new(),
            prev_hand_len: 0,
            last_deal_at: None,
        }
    }

}

impl SceneBehavior for GameplayScene {
    /// Borrow the in-pause-menu options overlay, if the player has opened it.
    /// Used by the main loop to sync settings (volume, smoke, tile preset)
    /// the same way it does for the standalone `OptionsScene`.
    fn pause_options_overlay(&self) -> Option<&super::options::OptionsScene> {
        self.pause_menu.options_overlay()
    }

    /// See [`crate::scenes::SceneBehavior::has_blocking_overlay`]. Reports
    /// `true` when any in-scene modal-like overlay is up: pause menu,
    /// glossary screen, or the scoring cascade animation. The cascade is
    /// included because it already blocks input internally — declaring it
    /// here also kills hover tooltips on hand tiles and relics during the
    /// score reveal.
    fn has_blocking_overlay(&self) -> bool {
        self.pause_menu.paused || self.glossary.open || self.cascade.is_some()
    }

    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.particles.update(dt);
        self.cursor_pos = ctx.cursor_pos;

        // Detect deal events: any time the hand grows (initial round deal,
        // post-discard refill) we stamp `last_deal_at` so the post-deal smoke
        // gust can fire `POST_DEAL_GUST_DELAY` later.
        let cur_hand_len = ctx.run.hand.len();
        if cur_hand_len > self.prev_hand_len {
            self.last_deal_at = Some(now);
        }
        self.prev_hand_len = cur_hand_len;

        // Advance candle flicker. Each candle's `flicker` is a smoothed
        // approach toward a target wave (sum of two sines per candle), so
        // intensity wanders organically in roughly [0.7, 1.1].
        //
        // The post-deal wind gust also nearly snuffs the candles for its
        // duration: we compute the same 4t(1-t) bell envelope used by the
        // gust and multiply the flicker target down to ~12% at the peak,
        // so the candles dim sharply as the breath hits them and recover
        // as it tapers off.
        let candle_dim = self
            .last_deal_at
            .map(|d| {
                let elapsed = now.saturating_duration_since(d).as_secs_f32();
                if elapsed < POST_DEAL_GUST_DELAY
                    || elapsed >= POST_DEAL_GUST_DELAY + POST_DEAL_GUST_DURATION
                {
                    1.0
                } else {
                    let t = (elapsed - POST_DEAL_GUST_DELAY) / POST_DEAL_GUST_DURATION;
                    let env = (4.0 * t * (1.0 - t)).clamp(0.0, 1.0);
                    1.0 - 0.88 * env
                }
            })
            .unwrap_or(1.0);
        self.candle_time += dt;
        for c in self.candles.iter_mut() {
            let t = self.candle_time;
            let target = (0.9
                + 0.18 * (t * 7.3 + c.phase).sin()
                + 0.08 * (t * 13.1 + c.phase * 1.7).sin())
                * candle_dim;
            // Exponential smoothing — keeps the light from snapping. Bump
            // the rate during a gust so the dim/recover edge reads as a
            // wind hit instead of a slow fade.
            let smooth_rate = if candle_dim < 0.95 { 22.0 } else { 12.0 };
            let k = (1.0 - (-dt * smooth_rate).exp()).clamp(0.0, 1.0);
            c.flicker += (target - c.flicker) * k;
        }

        // Glossary overlay (Patch UX): cross-input help. The Help action
        // toggles it; while open, the overlay swallows all other input. The
        // `?` badge in the HUD also routes here via HELP_BADGE_ID.
        for &cid in ctx.button_clicks {
            if cid == HELP_BADGE_ID {
                self.glossary.toggle();
                return None;
            }
        }
        if !self.glossary.open {
            for a in ctx.actions {
                if matches!(a, UiAction::Help) {
                    self.glossary.toggle();
                    return None;
                }
            }
        } else {
            // Overlay is open — let it process input and then bail out.
            self.glossary.handle_input(ctx.actions, ctx.button_clicks);
            return None;
        }

        // Pause menu handling — drives the menu while paused and intercepts
        // the open-on-Pause shortcut. Returns immediately if either applies.
        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            return t;
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

            // Reveal-edge effects: fire once per step on the frame the
            // cascade transitions onto a new step. Drives both the relic
            // glow overlay and the per-step audio beat.
            if let Some(idx) = frame.new_step_index {
                if self.last_revealed_step != Some(idx) {
                    self.last_revealed_step = Some(idx);
                    ctx.bus
                        .push(crate::game::event_bus::GameEvent::ScoreStepRevealed { index: idx });
                    if let Some(step) = cascade.breakdown.steps.get(idx) {
                        if let Some(rid) = crate::core::relic::relic_by_name(&step.source) {
                            self.relic_glow_starts.insert(rid, now);
                        }
                    }
                }
            }

            // Final-beat edge: fire once when the cascade transitions into
            // its ShowTotal phase. Used by the audio dispatcher to play the
            // closing sting on the final number landing.
            if cascade.is_in_total() && !self.cascade_final_emitted {
                self.cascade_final_emitted = true;
                ctx.bus
                    .push(crate::game::event_bus::GameEvent::ScoreCascadeFinal);
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
                self.last_revealed_step = None;
                self.cascade_final_emitted = false;
            } else {
                // Allow skip on any key press during cascade.
                if !ctx.actions.is_empty() {
                    cascade.skip();
                    self.displayed_score = ctx.run.round_score;
                    self.cascade = None;
                    self.last_revealed_step = None;
                    self.cascade_final_emitted = false;
                }
                return None;
            }
        }

        // Evict expired relic glow entries so the map doesn't grow.
        self.relic_glow_starts
            .retain(|_, start| now.saturating_duration_since(*start) < RELIC_GLOW_LIFETIME);

        // Zodiac inventory clicks (Patch B finishing): the player can use a
        // Zodiac card between plays to permanently level its yaku for the
        // run. Click ids are `ZODIAC_USE_BASE + slot_idx`. Only allowed when
        // no cascade is mid-flight, which the early return above already
        // guarantees.
        for &cid in ctx.button_clicks {
            if cid >= ZODIAC_USE_BASE && cid < ZODIAC_USE_BASE + 16 {
                let idx = (cid - ZODIAC_USE_BASE) as usize;
                if let Some((yaku, level)) = ctx.run.use_zodiac(idx) {
                    log::info!("Used Zodiac → {} now level {}", yaku.name(), level);
                }
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
                        self.button_focus = Some(GameplayButton::Play);
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
                    let idx = ALL_BUTTONS.iter().position(|b| *b == cur).unwrap_or(0);
                    self.button_focus = Some(ALL_BUTTONS[(idx + 1).min(ALL_BUTTONS.len() - 1)]);
                    continue;
                }
                UiAction::FocusPrev if self.button_focus.is_some() => {
                    let cur = self.button_focus.unwrap();
                    let idx = ALL_BUTTONS.iter().position(|b| *b == cur).unwrap_or(0);
                    self.button_focus = Some(ALL_BUTTONS[idx.saturating_sub(1)]);
                    continue;
                }
                UiAction::Confirm if self.button_focus.is_some() => {
                    // Translate button press into the corresponding action.
                    actions_for_scene.push(self.button_focus.unwrap().ui_action());
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
                            self.last_revealed_step = None;
                            self.cascade_final_emitted = false;
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

    fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
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

        // Center the hand horizontally when fewer tiles than full slots are present.
        let max_slots = layout.hand_slots.len();
        let visible_count = run.hand.len().min(max_slots);
        let slot_w = layout.hand_slots.first().map(|r| r.w).unwrap_or(0.0);
        let center_offset = if visible_count < max_slots {
            ((max_slots - visible_count) as f32 * slot_w) * 0.5
        } else {
            0.0
        };
        let hand_slots: Vec<(f32, f32, f32, f32)> = layout
            .hand_slots
            .iter()
            .take(run.hand.len())
            .map(|r| (r.x + center_offset, r.y, r.w, r.h))
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
        // Shanten meter (Patch A.5): "tiles away from a complete hand" for the
        // best decomposition the player could form by swapping tiles. Shown
        // unconditionally — the design plan calls for it as a baseline UI
        // affordance, with the Shanten Lens relic providing the chip side.
        let shanten = crate::core::shanten::shanten_estimate(&run.hand);
        let shanten_text = match shanten {
            -1 => "Won".to_string(),
            0 => "Tenpai".to_string(),
            n => format!("Shanten {}", n),
        };
        // Round wind for the current ante (East/South/West/North).
        let round_wind = crate::core::rules::BlindKind::round_wind_for_ante(run.ante);
        let wind_label = crate::core::rules::BlindKind::wind_name(round_wind);
        // Score header is split into two lines so it doesn't get auto-shrunk
        // into the cartouche. The cartouche is only ~38% of the score panel
        // width and the rasterizer's width-based fallback would otherwise
        // squeeze a 100-char single-line string to the 8px floor.
        let score_text_top = format!(
            "{}  ·  R{}  ·  {} / {}",
            run.blind.name(),
            run.run_number,
            shown_score,
            run.target_score,
        );
        let score_text_bot = format!(
            "${}  ·  Wall {}  ·  Wind {}  ·  {}{}",
            run.gold, tiles_left, wind_label, shanten_text, dora_section
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
                    ..Default::default()
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
                    ..Default::default()
                });
            }

            // Big "×" sigil between the pills.
            cascade_labels.push(TextLabel {
                rect: [inner_x + pill_w, pill_y, cross_w, pill_h],
                text: "×".into(),
                color: [0.85, 0.85, 0.95, 1.0],
                ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
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
                            ..Default::default()
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

        // The 2D relic strip is replaced by physical 3D relic boxes sitting
        // on a dish on the table (built later in this function). No 2D
        // relic row is rendered in the gameplay scene anymore.
        let relic_icons: Vec<crate::render::wgpu_renderer::RelicIcon> = Vec::new();

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
        // Captured during the loop below — `(yaku_kind, anchor_x, anchor_y)`
        // for the card the cursor is currently hovering, if any. The tooltip
        // is pushed after the loop completes so it draws on top of all cards.
        let mut hovered_yaku: Option<(crate::core::yaku::YakuKind, f32, f32)> = None;

        // Filter previews so the panel only shows the *meaningful* cards:
        // (1) every yaku in the active loadout (build identity),
        // (2) FullHand and Yakuhai (always full strength),
        // (3) any yaku currently *firing* on the selection (surprise hits).
        // This caps the visible card count at ~6 instead of cramming all 14
        // unlocked yaku into the same width as the 4-button bar — which would
        // force the rasterizer to auto-shrink card text to the 8px floor.
        let visible_previews: Vec<&crate::core::yaku::YakuPreview> = previews
            .iter()
            .filter(|p| {
                run.yaku_loadout.contains(&p.kind)
                    || matches!(
                        p.kind,
                        crate::core::yaku::YakuKind::FullHand
                            | crate::core::yaku::YakuKind::Yakuhai
                    )
                    || p.active
            })
            .collect();

        if !visible_previews.is_empty() {
            let panel_h = (66.0 * scale).max(48.0);
            let panel_gap = 8.0 * scale;
            let panel_y = btn_y - panel_h - panel_gap;
            let panel_w = container_w;
            let panel_x = container_x;
            let n = visible_previews.len() as f32;
            let card_gap = 6.0 * scale;
            let card_w = (panel_w - card_gap * (n - 1.0)) / n;
            // Pinned font sizes — never let the rasterizer auto-shrink card
            // text into illegibility, regardless of how narrow the cards get.
            let name_font = (panel_h * 0.30).clamp(13.0, 20.0);
            let hint_font = (panel_h * 0.24).clamp(11.0, 16.0);
            for (i, p) in visible_previews.iter().enumerate() {
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
                instances.push(GpuInstance {
                    rect: [cx + 4.0, fill_y, card_w - 8.0, fill_h],
                    color: [0.06, 0.07, 0.12, 0.9],
                });
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
                // Top text: yaku name + bonus value, pinned font_px.
                let name_h = panel_h * 0.42;
                let name_color = if p.active {
                    [1.0, 0.92, 0.55, 1.0]
                } else {
                    [0.78, 0.80, 0.90, 1.0]
                };
                yaku_text_labels.push(TextLabel {
                    rect: [cx + 4.0, cy + 2.0, card_w - 8.0, name_h],
                    text: format!("{} +{}", p.kind.name(), p.kind.mult_bonus()),
                    color: name_color,
                    font_px: Some(name_font),
                    ..Default::default()
                });
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
                    font_px: Some(hint_font),
                    ..Default::default()
                });

                // Hover tracking for the tooltip pass below the loop.
                let (cur_x, cur_y) = self.cursor_pos;
                if cur_x >= cx
                    && cur_x <= cx + card_w
                    && cur_y >= cy
                    && cur_y <= cy + panel_h
                {
                    hovered_yaku = Some((p.kind, cx + card_w * 0.5, cy));
                }
            }
        }
        // Yaku card hover tooltip — pushed into `instances` (which exists)
        // and into `yaku_text_labels`, which gets merged into `text_labels`
        // later in this function.
        if let Some((yk, ax, ay)) = hovered_yaku {
            let title = format!(
                "{}  (+{} mult, +{} chips)",
                yk.name(),
                yk.mult_bonus(),
                yk.chip_bonus()
            );
            let body = format!(
                "{}.  Loadout: {}",
                yaku_card_shape_text(yk),
                if run.yaku_loadout.contains(&yk)
                    || matches!(
                        yk,
                        crate::core::yaku::YakuKind::FullHand | crate::core::yaku::YakuKind::Yakuhai
                    )
                {
                    "yes (full strength)"
                } else {
                    "no (scores at 50%)"
                },
            );
            push_tooltip(
                &mut instances,
                &mut yaku_text_labels,
                ax,
                ay,
                layout.window_w,
                layout.window_h,
                &title,
                &body,
            );
        }

        let paused = self.pause_menu.paused;
        let btn_rects = [
            suit_btn_rect,
            rank_btn_rect,
            play_btn_rect,
            discard_btn_rect,
        ];
        let base_colors: [[f32; 4]; 4] = [
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
            if self.button_focus == Some(ALL_BUTTONS[i]) {
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
        // Two-line stack inside the cartouche: top = blind/round/score, bot =
        // status row (gold/wall/wind/shanten). Each line gets its own pinned
        // font_px so they render at the same readable size regardless of how
        // long the strings are.
        let header_top_font = (ctx_h * 0.42).clamp(14.0, 28.0);
        let header_bot_font = (ctx_h * 0.30).clamp(12.0, 22.0);
        let line_top_h = ctx_h * 0.55;
        let line_bot_h = ctx_h * 0.45;
        let mut text_labels = vec![
            TextLabel {
                rect: [ctx_x, ctx_y, ctx_w, line_top_h],
                text: score_text_top,
                color: crate::render::theme::color::CHAMPAGNE,
                font_px: Some(header_top_font),
                ..Default::default()
            },
            TextLabel {
                rect: [ctx_x, ctx_y + line_top_h, ctx_w, line_bot_h],
                text: score_text_bot,
                color: crate::render::theme::color::PARCHMENT,
                font_px: Some(header_bot_font),
                ..Default::default()
            },
        ];
        // Modifier strip: dynamic cascade widgets (when active) or status text (idle).
        if cascade_labels.is_empty() {
            text_labels.push(TextLabel {
                rect: [ms.x, ms.y, ms.w, ms.h],
                text: cascade_text,
                color: cascade_color,
                ..Default::default()
            });
        } else {
            text_labels.extend(cascade_labels);
        }
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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

        // ── Zodiac inventory bar (Patch B finishing) ─────────────────────
        //
        // Sits in the top-right corner of the screen, away from the score
        // cartouche and relic dish. Each slot is a clickable badge showing
        // the Zodiac's name and the yaku it levels; clicking it consumes the
        // card and bumps the yaku level for the rest of the run.
        //
        // Slot dimensions are sized to comfortably fit two pinned-font lines
        // plus the longest yaku name + level suffix (e.g. "→ Sanshoku L9").
        let zodiac_inv = &run.zodiac_inventory;
        if zodiac_inv.capacity > 0 {
            let zscale = (layout.window_w.min(layout.window_h)) / 600.0;
            let slot_w = (140.0 * zscale).max(120.0);
            let slot_h = (56.0 * zscale).max(48.0);
            let gap = (6.0 * zscale).max(3.0);
            let total_w = slot_w * zodiac_inv.capacity as f32
                + gap * (zodiac_inv.capacity as f32 - 1.0);
            let strip_x = layout.window_w - total_w - (16.0 * zscale);
            let strip_y = layout.score_panel.y + layout.score_panel.h + (8.0 * zscale);
            for slot_idx in 0..zodiac_inv.capacity {
                let zx = strip_x + slot_idx as f32 * (slot_w + gap);
                let zy = strip_y;
                if let Some(&z) = zodiac_inv.items.get(slot_idx) {
                    // Filled slot — gold background, click registers the use.
                    instances.push(GpuInstance {
                        rect: [zx, zy, slot_w, slot_h],
                        color: [0.42, 0.32, 0.10, 0.92],
                    });
                    // Gold rim.
                    let rim = 1.5 * zscale;
                    let gold = crate::render::theme::color::GOLD;
                    instances.push(GpuInstance {
                        rect: [zx, zy, slot_w, rim],
                        color: gold,
                    });
                    instances.push(GpuInstance {
                        rect: [zx, zy + slot_h - rim, slot_w, rim],
                        color: gold,
                    });
                    instances.push(GpuInstance {
                        rect: [zx, zy, rim, slot_h],
                        color: gold,
                    });
                    instances.push(GpuInstance {
                        rect: [zx + slot_w - rim, zy, rim, slot_h],
                        color: gold,
                    });
                    // Two pinned-font rows: zodiac name (gold) over the
                    // target yaku + level (parchment). Pinning prevents the
                    // rasterizer from auto-shrinking long yaku names like
                    // "Sanshoku" to the 8px floor when the slot is narrow.
                    let name_font = (slot_h * 0.34).clamp(14.0, 22.0);
                    let yaku_font = (slot_h * 0.28).clamp(12.0, 18.0);
                    let name_h = slot_h * 0.46;
                    text_labels.push(TextLabel {
                        rect: [zx, zy + 2.0, slot_w, name_h],
                        text: z.name().to_string(),
                        color: crate::render::theme::color::CHAMPAGNE,
                        font_px: Some(name_font),
                        ..Default::default()
                    });
                    let level = run.yaku_levels.level_of(z.yaku());
                    let yaku_h = slot_h * 0.40;
                    text_labels.push(TextLabel {
                        rect: [zx, zy + name_h + 2.0, slot_w, yaku_h],
                        text: format!("{} L{}", z.yaku().name(), level),
                        color: crate::render::theme::color::PARCHMENT,
                        font_px: Some(yaku_font),
                        ..Default::default()
                    });
                    if !paused {
                        buttons.push(ButtonDef::scene(
                            (zx, zy, slot_w, slot_h),
                            ZODIAC_USE_BASE + slot_idx as u32,
                        ));
                    }
                    // Hover tooltip — mirrors the relic tooltip pattern.
                    let (cx, cy) = self.cursor_pos;
                    if cx >= zx && cx <= zx + slot_w && cy >= zy && cy <= zy + slot_h {
                        let level = run.yaku_levels.level_of(z.yaku());
                        let title = format!("{} (Zodiac)", z.name());
                        let body = format!(
                            "Click or press to use. Permanently raises {} from level {} to {} for the rest of the run (+0.5 mult, +20 chips per level).",
                            z.yaku().name(),
                            level,
                            level + 1,
                        );
                        push_tooltip(
                            &mut instances,
                            &mut text_labels,
                            zx + slot_w * 0.5,
                            zy,
                            layout.window_w,
                            layout.window_h,
                            &title,
                            &body,
                        );
                    }
                } else {
                    // Empty slot — dim outline.
                    instances.push(GpuInstance {
                        rect: [zx, zy, slot_w, slot_h],
                        color: [0.06, 0.07, 0.12, 0.55],
                    });
                    // Hover tooltip even on empty slots: tells the player what
                    // the strip is for.
                    let (cx, cy) = self.cursor_pos;
                    if cx >= zx && cx <= zx + slot_w && cy >= zy && cy <= zy + slot_h {
                        push_tooltip(
                            &mut instances,
                            &mut text_labels,
                            zx + slot_w * 0.5,
                            zy,
                            layout.window_w,
                            layout.window_h,
                            "Zodiac Slot",
                            "Empty. Earn Zodiac cards by clearing blinds or buying them in the shop. Using one permanently levels its yaku.",
                        );
                    }
                }
            }
        }

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
            // Prefer the renderer's raycast pick (camera ray vs tile OBB);
            // fall back to a flat slot test on the very first frame before
            // any pick data exists.
            let hovered_idx: Option<usize> = ctx.picked_hand_tile.or_else(|| {
                hand_slots
                    .iter()
                    .enumerate()
                    .find(|&(_, &(sx, sy, sw, sh))| {
                        cx >= sx && cx <= sx + sw && cy >= sy && cy <= sy + sh
                    })
                    .map(|(i, _)| i)
            });

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
                            ..Default::default()
                        });
                    }
                }
            }
        }

        // ── Help (`?`) badge — top-left corner ───────────────────────────
        //
        // A small clickable badge that opens the glossary overlay. Sized to
        // match the existing top HUD chrome, positioned outside the score
        // panel so it doesn't crowd the cartouche.
        let help_w = (38.0 * scale).max(24.0);
        let help_h = help_w;
        let help_x = (12.0 * scale).max(8.0);
        let help_y = layout.score_panel.y + (layout.score_panel.h - help_h) * 0.5;
        instances.push(GpuInstance {
            rect: [help_x, help_y, help_w, help_h],
            color: crate::render::theme::color::alpha(
                crate::render::theme::color::INDIGO,
                0.92,
            ),
        });
        // Gold rim so the badge reads as interactive.
        let rim = (1.5 * scale).max(1.0);
        let gold = crate::render::theme::color::GOLD;
        instances.push(GpuInstance {
            rect: [help_x, help_y, help_w, rim],
            color: gold,
        });
        instances.push(GpuInstance {
            rect: [help_x, help_y + help_h - rim, help_w, rim],
            color: gold,
        });
        instances.push(GpuInstance {
            rect: [help_x, help_y, rim, help_h],
            color: gold,
        });
        instances.push(GpuInstance {
            rect: [help_x + help_w - rim, help_y, rim, help_h],
            color: gold,
        });
        text_labels.push(TextLabel {
            rect: [help_x, help_y, help_w, help_h],
            text: "?".into(),
            color: crate::render::theme::color::CHAMPAGNE,
            ..Default::default()
        });
        if !paused {
            buttons.push(ButtonDef::scene(
                (help_x, help_y, help_w, help_h),
                HELP_BADGE_ID,
            ));
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

        // Glossary overlay — drawn last so it sits on top of everything,
        // including the pause menu (closing the glossary returns the player
        // to whatever they were doing underneath).
        self.glossary.draw(
            layout.window_w,
            layout.window_h,
            &mut instances,
            &mut text_labels,
            &mut buttons,
        );
        if self.glossary.open {
            // Click-blocker behind the glossary's own buttons.
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
        let _relic_placements: Vec<crate::scenes::RelicPlacement> = Vec::new();
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

        // The 3D table + tiles + candles ARE the UI. Selection feedback is
        // now a true 3D gold-metal outline shell drawn by the renderer's
        // tile_outline_pipeline (which catches candlelight), so no 2D
        // selection overlay is added here.

        // ── Physical relic placeholders sitting in a dish on the table ──
        // Each active relic becomes a small colored box in a row toward the
        // back of the table. Sizes are deterministic-but-varied so the row
        // reads as a collection of distinct objects rather than a uniform
        // line of cubes. The renderer draws a brass dish under them.
        let mut relic_placements: Vec<crate::render::draw_cmd::RelicPlacement> = Vec::new();
        let active_ids = &run.relics.active;
        if !active_ids.is_empty() {
            use crate::core::relic::all_relic_defs;
            let defs = all_relic_defs();
            // Lay the row out in pixel coordinates so the renderer's
            // pixel_to_world helper places them on the table plane. We sit
            // them just above the score-panel band, in front of the
            // back-edge candles.
            let row_center_x = layout.window_w * 0.5;
            let row_y = layout.score_panel.y + layout.score_panel.h + layout.window_h * 0.08;
            let n = active_ids.len() as f32;
            // Each cell ~160px wide so the relics read as substantial
            // physical objects rather than dice.
            let cell_w = 160.0_f32;
            let total_w = cell_w * n;
            let start_x = row_center_x - total_w * 0.5 + cell_w * 0.5;
            for (i, &rid) in active_ids.iter().enumerate() {
                // Pseudo-random per-relic size variation, deterministic on id.
                let seed = (rid as u32).wrapping_mul(2654435761) ^ 0x9E3779B9;
                let r0 = ((seed >> 8) & 0xFF) as f32 / 255.0;
                let r1 = ((seed >> 16) & 0xFF) as f32 / 255.0;
                let r2 = ((seed >> 24) & 0xFF) as f32 / 255.0;
                let half_x = 55.0 + r0 * 18.0;
                let half_y = 40.0 + r1 * 22.0;
                let half_z = 38.0 + r2 * 16.0;

                // Color tracks the relic's rarity tier so similar-rarity
                // relics share a visual family.
                let rarity = defs
                    .iter()
                    .find(|d| d.id == rid)
                    .map(|d| d.rarity)
                    .unwrap_or(crate::core::relic::Rarity::Common);
                let tier: u8 = match rarity {
                    crate::core::relic::Rarity::Common => 0,
                    crate::core::relic::Rarity::Uncommon => 1,
                    crate::core::relic::Rarity::Rare => 2,
                    crate::core::relic::Rarity::Legendary => 3,
                };
                let color = crate::render::theme::color::rarity(tier);

                // Activation glow: if this relic is in the cascade glow map,
                // compute a fast-attack / smooth-decay envelope and stash it
                // on the placement so the renderer can brighten the box and
                // emit a real additive halo around it. Attack is the first
                // ~12% of the lifetime; the rest is a quadratic decay so the
                // glow flares quickly and lingers in a soft afterglow.
                let glow = if let Some(start) = self.relic_glow_starts.get(&rid) {
                    let now_for_glow = Instant::now();
                    let age = now_for_glow.saturating_duration_since(*start).as_secs_f32();
                    let life = RELIC_GLOW_LIFETIME.as_secs_f32();
                    if age >= life {
                        0.0
                    } else {
                        let t = (age / life).clamp(0.0, 1.0);
                        let attack_end = 0.12_f32;
                        if t < attack_end {
                            (t / attack_end).clamp(0.0, 1.0)
                        } else {
                            let decay_t = (t - attack_end) / (1.0 - attack_end);
                            (1.0 - decay_t).max(0.0).powi(2)
                        }
                    }
                } else {
                    0.0
                };

                let px = start_x + i as f32 * cell_w;
                relic_placements.push(crate::render::draw_cmd::RelicPlacement {
                    world_pos: [px, row_y, 0.0],
                    half_extents: [half_x, half_y, half_z],
                    color,
                    relic_id: rid,
                    glow,
                });
            }
        }

        // Hover detection + outline + tooltip overlay. Uses the
        // renderer-provided projected screen rect from the *previous* frame
        // (one-frame-stale, like hand tile hover) to hit-test the cursor.
        let (cur_x, cur_y) = self.cursor_pos;
        let mut hovered_relic_idx: Option<usize> = None;
        for (i, rect) in ctx.projected_relic_rects.iter().enumerate() {
            let [rx, ry, rw, rh] = *rect;
            if cur_x >= rx && cur_x <= rx + rw && cur_y >= ry && cur_y <= ry + rh {
                hovered_relic_idx = Some(i);
                break;
            }
        }
        if let Some(hi) = hovered_relic_idx {
            if let (Some(rect), Some(rid)) = (
                ctx.projected_relic_rects.get(hi),
                relic_placements.get(hi).map(|p| p.relic_id),
            ) {
                // Gold rim drawn around the projected screen rect — cheap
                // 2D outline that hugs the visible 3D box.
                let [rx, ry, rw, rh] = *rect;
                let t = (rh * 0.04).clamp(2.0, 4.0);
                let rim = crate::render::theme::color::CHAMPAGNE;
                instances.push(GpuInstance {
                    rect: [rx - t, ry - t, rw + t * 2.0, t],
                    color: rim,
                });
                instances.push(GpuInstance {
                    rect: [rx - t, ry + rh, rw + t * 2.0, t],
                    color: rim,
                });
                instances.push(GpuInstance {
                    rect: [rx - t, ry, t, rh],
                    color: rim,
                });
                instances.push(GpuInstance {
                    rect: [rx + rw, ry, t, rh],
                    color: rim,
                });

                // Tooltip: name + description in a small dark panel anchored
                // above the hovered relic.
                use crate::core::relic::all_relic_defs;
                let defs = all_relic_defs();
                if let Some(def) = defs.iter().find(|d| d.id == rid) {
                    let pad = 18.0_f32;
                    let tip_w = 440.0_f32;
                    let title_h = 38.0_f32;
                    // Pre-wrap the description so the tooltip box can grow
                    // tall enough to fit every line.
                    let body_style = TextStyle {
                        tier: typography::BODY,
                        color: crate::render::theme::color::PARCHMENT,
                        padding: 0.0,
                        align: TextAlign::Left,
                    };
                    let body_line_h = typography::size(body_style.tier, layout.window_h);
                    let body_step = body_line_h * 1.6;
                    let body_box = body_line_h * 1.8;
                    let body_inner_w = tip_w - pad * 2.0;
                    let wrapped_lines = widget::wrap_text(def.description, body_inner_w, body_line_h);
                    let body_h = (wrapped_lines.len() as f32 * body_step).max(body_box);
                    let tip_h = pad * 2.0 + title_h + body_h;
                    let mut tip_x = rx + rw * 0.5 - tip_w * 0.5;
                    let mut tip_y = ry - tip_h - 8.0;
                    // Clamp to window so the tooltip stays visible.
                    tip_x = tip_x.clamp(8.0, layout.window_w - tip_w - 8.0);
                    if tip_y < 8.0 {
                        tip_y = ry + rh + 8.0;
                    }
                    let bg = crate::render::theme::color::alpha(
                        crate::render::theme::color::MIDNIGHT,
                        0.96,
                    );
                    instances.push(GpuInstance {
                        rect: [tip_x, tip_y, tip_w, tip_h],
                        color: bg,
                    });
                    // Gold border (4 thin quads).
                    let bt = 1.5_f32;
                    let border = crate::render::theme::color::BRASS;
                    instances.push(GpuInstance {
                        rect: [tip_x, tip_y, tip_w, bt],
                        color: border,
                    });
                    instances.push(GpuInstance {
                        rect: [tip_x, tip_y + tip_h - bt, tip_w, bt],
                        color: border,
                    });
                    instances.push(GpuInstance {
                        rect: [tip_x, tip_y + bt, bt, tip_h - bt * 2.0],
                        color: border,
                    });
                    instances.push(GpuInstance {
                        rect: [tip_x + tip_w - bt, tip_y + bt, bt, tip_h - bt * 2.0],
                        color: border,
                    });
                    text_labels.push(TextLabel {
                        rect: [tip_x + pad, tip_y + pad, tip_w - pad * 2.0, title_h],
                        text: def.name.to_string(),
                        color: crate::render::theme::color::CHAMPAGNE,
                        ..Default::default()
                    });
                    widget::push_text_block(
                        &mut text_labels,
                        [
                            tip_x + pad,
                            tip_y + pad + title_h,
                            tip_w - pad * 2.0,
                            body_h,
                        ],
                        def.description,
                        body_style,
                        layout.window_h,
                    );
                }
            }
        }

        // When the glossary is open we suppress every 3D-pass element so the
        // overlay reads as a clean help screen instead of a transparent panel
        // floating over the table. The quad pass is already self-contained
        // (the glossary cleared `instances`/`text_labels`/`buttons` itself),
        // but the 3D mesh / tile / candle / particle passes use these other
        // SceneDrawOutput fields and need explicit suppression here.
        let glossary_open = self.glossary.open;
        let final_hand_tiles = if glossary_open { Vec::new() } else { run.hand.to_vec() };
        let final_hand_slots = if glossary_open { Vec::new() } else { hand_slots };
        let final_selected_tiles = if glossary_open {
            Vec::new()
        } else {
            run.selected.clone()
        };
        let final_relic_icons = if glossary_open { Vec::new() } else { relic_icons };
        let final_flame_instances = if glossary_open {
            Vec::new()
        } else {
            flame_instances
        };
        let final_point_lights = if glossary_open { Vec::new() } else { point_lights };
        let final_candles = if glossary_open { Vec::new() } else { candle_placements };
        let final_relic_placements = if glossary_open {
            Vec::new()
        } else {
            relic_placements
        };

        // Post-deal smoke breath. `POST_DEAL_GUST_DELAY` after the most
        // recent deal we exhale a soft sweep of impulses across the hand
        // strip — a few evenly spaced points pushed back-and-up — so the
        // smoke that built up while the tiles were sliding in drifts off
        // toward the back of the table. The strength follows a 4t(1-t)
        // bell so the breath fades in and out instead of snapping on.
        let mut wind_gusts: Vec<crate::render::draw_cmd::WindGust> = Vec::new();
        if !glossary_open {
            if let Some(deal_at) = self.last_deal_at {
                let elapsed = now.saturating_duration_since(deal_at).as_secs_f32();
                if elapsed >= POST_DEAL_GUST_DELAY
                    && elapsed < POST_DEAL_GUST_DELAY + POST_DEAL_GUST_DURATION
                {
                    let t = (elapsed - POST_DEAL_GUST_DELAY) / POST_DEAL_GUST_DURATION;
                    let envelope = (4.0 * t * (1.0 - t)).clamp(0.0, 1.0);
                    if !final_hand_slots.is_empty() {
                        // Sample anchor: y from the hand strip, but x is
                        // swept across the FULL window width (with a little
                        // overshoot past the edges) so the gusts also push
                        // smoke off the corners of the table — not just the
                        // smoke directly above the hand. Each impulse uses
                        // a generous radius so neighbouring gusts overlap
                        // into one continuous sheet of wind.
                        let sw = final_hand_slots[0].2;
                        let sy = final_hand_slots[0].1;
                        let cy = sy - sw * 0.25;
                        const SAMPLES: usize = 9;
                        let win_w = layout.window_w;
                        let pad = win_w * 0.12;
                        let span_min = -pad;
                        let span_max = win_w + pad;
                        for i in 0..SAMPLES {
                            let f = (i as f32 + 0.5) / SAMPLES as f32;
                            let cx = span_min + (span_max - span_min) * f;
                            // Slightly tilt the velocity outward at the
                            // edges so smoke at the sides gets pushed both
                            // back AND laterally off-stage instead of just
                            // straight back.
                            let edge_bias = (f - 0.5) * 2.0; // -1..1
                            let lateral = 28.0 * edge_bias * envelope;
                            wind_gusts.push(crate::render::draw_cmd::WindGust {
                                center_px: (cx, cy),
                                lift: 18.0,
                                velocity: [lateral, 6.0 * envelope, -55.0 * envelope],
                                radius: (win_w / SAMPLES as f32) * 1.6,
                                density: -0.04 * envelope,
                            });
                        }
                    }
                }
            }
        }

        SceneDrawOutput {
            background: super::BackgroundId::Gameplay,
            tray_instances: Vec::new(),
            instances,
            hand_tiles: final_hand_tiles,
            hand_slots: final_hand_slots,
            focus,
            selected_tiles: final_selected_tiles,
            text_labels,
            relic_icons: final_relic_icons,
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
            departing_indices: if glossary_open {
                Vec::new()
            } else {
                self.pending_departures.clone()
            },
            hint_indices: if glossary_open { Vec::new() } else { hint_indices },
            flame_instances: final_flame_instances,
            point_lights: final_point_lights,
            candles: final_candles,
            relic_placements: final_relic_placements,
            draw_table: !glossary_open,
            wind_gusts,
        }
    }
}

/// Plain-language hand-shape description for a yaku, mirrored from the
/// glossary so the gameplay tooltip and the help overlay agree.
fn yaku_card_shape_text(yk: crate::core::yaku::YakuKind) -> &'static str {
    use crate::core::yaku::YakuKind;
    match yk {
        YakuKind::Tanyao => "All tiles 2–8, no honors or terminals",
        YakuKind::Toitoi => "All triplets and kongs (no sequences)",
        YakuKind::FullHand => "Complete 14-tile hand: 4 melds + 1 pair",
        YakuKind::Yakuhai => "Triplet of any dragon or the round wind",
        YakuKind::Iipeikou => "Two identical sequences in one suit",
        YakuKind::SanshokuDoujun => "Same sequence in m / s / p suits",
        YakuKind::Ittsu => "1-2-3, 4-5-6, 7-8-9 in one suit",
        YakuKind::Honitsu => "One number suit + honors only",
        YakuKind::Chinitsu => "All one number suit, no honors",
        YakuKind::Junchan => "Every meld contains a terminal (1/9)",
        YakuKind::Honroutou => "Only terminals (1/9) and honors",
        YakuKind::Chiitoitsu => "Seven distinct pairs",
    }
}

/// Render a small tooltip panel anchored above-left of `(anchor_x, anchor_y)`.
/// Used by the gameplay HUD's hover-tooltip pass for zodiac slots and yaku
/// progress cards. Mirrors the styling of the existing relic tooltip block
/// (dark midnight panel + brass border + champagne title + parchment body).
///
/// Title pins its font_px so even long names like "Sanshoku Doujun (+4 mult,
/// +50 chips)" don't get auto-shrunk into illegibility, and the body uses
/// `push_text_block` with a pinned CAPTION-tier font so multi-line wrapping
/// renders at a single readable size instead of squeezing to the 8px floor.
fn push_tooltip(
    instances: &mut Vec<GpuInstance>,
    text_labels: &mut Vec<TextLabel>,
    anchor_x: f32,
    anchor_y: f32,
    window_w: f32,
    window_h: f32,
    title: &str,
    body: &str,
) {
    use crate::render::theme::{color, typography};
    use crate::ui::widget::{self, TextStyle};

    let pad = 14.0_f32;
    let tip_w = (window_w * 0.34).clamp(300.0, 500.0);

    // Pin font sizes — never let the rasterizer auto-shrink below readable.
    let title_font = typography::size(typography::BODY, window_h).max(15.0);
    let body_font = typography::size(typography::CAPTION, window_h).max(13.0);
    let title_h = title_font * 1.6;
    let body_line_step = body_font * 1.4;

    // Estimate body line count from text length / approx chars per line at the
    // pinned body font size. Each glyph ~ body_font * 0.55 wide on average.
    let inner_w = tip_w - pad * 2.0;
    let chars_per_line = (inner_w / (body_font * 0.55)).max(10.0) as usize;
    let est_lines = (body.len() / chars_per_line + 1).max(2) as f32;
    let body_h = (est_lines * body_line_step).min(window_h * 0.5);

    let tip_h = pad * 2.0 + title_h + 4.0 + body_h;
    let mut tip_x = anchor_x - tip_w * 0.5;
    let mut tip_y = anchor_y - tip_h - 8.0;
    tip_x = tip_x.clamp(8.0, window_w - tip_w - 8.0);
    if tip_y < 8.0 {
        tip_y = anchor_y + 8.0;
    }
    let bg = color::alpha(color::MIDNIGHT, 0.96);
    instances.push(GpuInstance {
        rect: [tip_x, tip_y, tip_w, tip_h],
        color: bg,
    });
    let bt = 1.5_f32;
    let border = color::BRASS;
    instances.push(GpuInstance {
        rect: [tip_x, tip_y, tip_w, bt],
        color: border,
    });
    instances.push(GpuInstance {
        rect: [tip_x, tip_y + tip_h - bt, tip_w, bt],
        color: border,
    });
    instances.push(GpuInstance {
        rect: [tip_x, tip_y + bt, bt, tip_h - bt * 2.0],
        color: border,
    });
    instances.push(GpuInstance {
        rect: [tip_x + tip_w - bt, tip_y + bt, bt, tip_h - bt * 2.0],
        color: border,
    });
    text_labels.push(TextLabel {
        rect: [tip_x + pad, tip_y + pad, inner_w, title_h],
        text: title.into(),
        color: color::CHAMPAGNE,
        font_px: Some(title_font),
        ..Default::default()
    });
    widget::push_text_block(
        text_labels,
        [tip_x + pad, tip_y + pad + title_h + 4.0, inner_w, body_h],
        body,
        TextStyle {
            tier: typography::CAPTION,
            color: color::PARCHMENT,
            padding: 0.0,
            align: TextAlign::Left,
        },
        window_h,
    );
}
