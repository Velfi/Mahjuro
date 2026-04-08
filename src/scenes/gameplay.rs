//! Gameplay scene — the main tile-playing loop.

use std::time::Instant;

use crate::core::hand::{describe_hand, suggest_completions};
use crate::core::scoring::StepKind;
use crate::core::yaku::yaku_preview;
use crate::game::cascade::ScoringCascade;
use crate::game::run::{STARTING_DISCARDS, STARTING_PLAYS};
use crate::render::animation::ENTITY_SCORE_PANEL;
use crate::render::candle_mesh::{CandlePlacement, WICK_TIP_Y};
use crate::render::draw_cmd::{CascadeTokenKind, DrawCmd, UiFrame};
use crate::render::falling_bones::FallingBoneSystem;
use crate::render::particles::ParticleSystem;
use crate::render::theme::typography;
use crate::render::wgpu_renderer::{
    GpuInstance, PointLight, TextAlign, TextLabel, build_instances_from_layout,
};
use crate::ui::input::{UiAction, apply_ui_actions};
use crate::ui::widget::{self, TextStyle};

use super::pause_menu::PauseMenu;
use super::{ButtonDef, DrawCtx, SceneBehavior, SceneDrawOutput, SceneTransition, UpdateCtx};

/// `pick_id` for the consumable inventory dish (Zodiacs + Talismans). Used
/// to look up the dish's projected screen rect from `ctx.aux_dish_rects`
/// so the per-slot hit-test, focus ring, click target, and tooltip
/// anchor track the visible (perspective-projected) dish position
/// instead of the raw pixel anchor we hand the renderer.
const PICK_CONSUMABLE_DISH: u32 = 1;

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
    /// Physical scoring bones that tumble onto the play space during a
    /// cascade. Spawned by each step reveal, integrated under gravity, and
    /// cleared when the cascade ends.
    falling_bones: FallingBoneSystem,
    /// Timestamp of last frame for dt calculation.
    last_frame: Instant,
    /// Shared pause menu overlay.
    pause_menu: PauseMenu,
    /// Which bottom button is focused (None = hand tiles have focus).
    button_focus: Option<GameplayButton>,
    /// Cross-input consumable focus. When `Some(slot)`, the consumable
    /// strip in the top-right is the active focus target — Confirm uses
    /// the focused consumable, Cancel clears the focus, and the focused
    /// slot is rendered with a brass highlight ring. Cycled by
    /// `NavigateHudNext` / `NavigateHudPrev` (LB / RB on controller,
    /// `[` / `]` on the keyboard). Lets controller and keyboard players
    /// activate Zodiacs and Talismans without ever needing the mouse.
    consumable_focus: Option<usize>,
    /// Tile indices that should depart this frame (set during update, consumed during draw).
    pending_departures: Vec<usize>,
    /// When set, the hand has been discarded-from but not yet refilled. The
    /// auto-draw fires once `Instant::now()` reaches this deadline, giving the
    /// discard departure animation time to play out.
    pending_refill: Option<Instant>,
    /// Latest cursor position (window coords), captured each update for hover tooltips.
    cursor_pos: (f32, f32),
    /// Animated state for the ambient candles flanking the play area. The
    /// first four are the original score-panel + hand-strip lanterns; the
    /// fifth is a "footlight" candle in front of the camera that lights the
    /// bottom row of yaku tablets and action buttons.
    /// Updated every frame; consumed in `draw()` to position flames + lights.
    candles: [CandleState; 5],
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
    /// `wind_delay_secs` after this and tapers off over `wind_duration_secs`.
    /// `None` between rounds and after the gust has finished playing.
    last_deal_at: Option<Instant>,
    /// Cached copy of `cascade_tuning.wind_delay_ms` (in seconds), updated
    /// each `update()` so live tweaks in the debug tuning overlay take
    /// effect on the next deal — and so `draw()` (which has no
    /// `cascade_tuning` access) can read it without re-borrowing.
    wind_delay_secs: f32,
    /// Cached copy of `cascade_tuning.wind_duration_ms` (in seconds).
    wind_duration_secs: f32,
    /// Debug-only: when set, the draw step injects a strong directional
    /// wind gust at every candle position for `DEBUG_WIND_DURATION` so the
    /// flame's wind response is visible on demand. Triggered by `B`.
    debug_wind_at: Option<Instant>,
    /// True until the opening blind transition has finished. While set, the
    /// draw step floods the smoke grid with positive-density impulses across
    /// the table during the `wind_delay_secs` window after the first deal,
    /// so the player enters the round inside a curtain of smoke that the
    /// existing post-deal wind sweep then blows away. Cleared once that
    /// sweep completes so subsequent post-discard refills don't re-fill
    /// the screen.
    initial_smoke_fill_active: bool,
    /// Debug-only: when true, the renderer overlays world-axes bars at the
    /// camera target so we can see which direction is +X / +Y / +Z while
    /// dialing in placements. Toggled by `F2`.
    debug_show_axes: bool,
}

/// How long the debug `B` gust stays active after a press.
const DEBUG_WIND_DURATION: f32 = 0.9;

/// Wind-delay override used for the *first* deal of the scene only. The
/// gameplay scene opens behind a fullscreen smoke curtain (a 2D dark
/// overlay backed by positive-density fluid impulses); after this delay
/// the existing post-deal wind sweep fires and blows the curtain off,
/// fading the overlay out in lockstep so the game becomes visible.
const OPENING_WIND_DELAY_SECS: f32 = 1.0;

/// How long a relic glow lingers after activation.
const RELIC_GLOW_LIFETIME: std::time::Duration = std::time::Duration::from_millis(900);

/// Click-id base for the Zodiac inventory bar. `ZODIAC_USE_BASE + slot_idx`
/// is the click id for using the Zodiac in slot `slot_idx`.
const ZODIAC_USE_BASE: u32 = 0x9000;

/// Click id for the `?` glossary badge in the gameplay HUD.
const HELP_BADGE_ID: u32 = 0x9100;

/// Catch-all `ButtonDef::scene` id pushed last so the gameplay scene can
/// route 3D-object clicks (sort/play wood tablets, discard bowl) through
/// `update()` based on `ctx.picked_gameplay_object`. Same pattern as the
/// shop scene's `SHOP_3D_HIT_ID`. Smaller specific buttons (consumables,
/// help badge) are pushed earlier and win the first-hit search in
/// `main.rs`'s `MouseInput` handler.
const GAMEPLAY_3D_HIT_ID: u32 = 0x9200;

impl GameplayScene {
    /// Whether the cascade is actively animating (for redraw requests).
    pub fn is_animating(&self) -> bool {
        self.cascade.is_some()
            || self.particles.is_active()
            || self.falling_bones.is_active()
            || self.pending_refill.is_some()
            || !self.relic_glow_starts.is_empty()
            || self.post_deal_gust_active()
            || self.debug_wind_active()
    }

    /// True while the post-deal smoke breath is either pending or actively
    /// blowing. Keeps the main loop ticking through the delay so the gust
    /// actually fires on idle frames.
    fn post_deal_gust_active(&self) -> bool {
        let Some(t) = self.last_deal_at else {
            return false;
        };
        let elapsed = Instant::now().saturating_duration_since(t).as_secs_f32();
        elapsed < self.wind_delay_secs + self.wind_duration_secs
    }

    /// True while the debug `B` gust is still firing — keeps the main loop
    /// drawing frames so the flame bend animates without needing other input.
    fn debug_wind_active(&self) -> bool {
        let Some(t) = self.debug_wind_at else {
            return false;
        };
        Instant::now().saturating_duration_since(t).as_secs_f32() < DEBUG_WIND_DURATION
    }

    pub fn new() -> Self {
        Self {
            cascade: None,
            displayed_score: 0,
            prev_displayed_score: 0,
            particles: ParticleSystem::new(),
            falling_bones: FallingBoneSystem::new(),
            last_frame: Instant::now(),
            pause_menu: PauseMenu::new(),
            button_focus: None,
            consumable_focus: None,
            pending_departures: Vec::new(),
            pending_refill: None,
            cursor_pos: (0.0, 0.0),
            // Five candles with golden-ratio spaced phases so their flicker
            // never visually syncs up. Phases are in [0, TAU). The fifth
            // entry drives the front-camera footlight candle.
            candles: [
                CandleState::new(0.0),
                CandleState::new(1.7),
                CandleState::new(3.9),
                CandleState::new(5.2),
                CandleState::new(2.6),
            ],
            candle_time: 0.0,
            relic_glow_starts: std::collections::HashMap::new(),
            last_revealed_step: None,
            cascade_final_emitted: false,
            glossary: super::glossary::GlossaryOverlay::new(),
            prev_hand_len: 0,
            last_deal_at: None,
            wind_delay_secs: 3.0,
            wind_duration_secs: 1.4,
            debug_wind_at: None,
            initial_smoke_fill_active: true,
            debug_show_axes: false,
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
        self.falling_bones.update(dt);
        // Cursor-movement guard: if the player has touched the mouse since
        // the last frame, drop any active controller / keyboard consumable
        // focus. Otherwise a mouse click on a hand tile (which generates
        // `UiAction::Confirm`) would be eaten by the consumable handler
        // and activate a Zodiac instead of selecting the tile. The guard
        // uses a tiny pixel threshold so micro-jitter on a stationary
        // cursor doesn't fight the controller focus.
        let cursor_moved = (ctx.cursor_pos.0 - self.cursor_pos.0).abs() > 0.5
            || (ctx.cursor_pos.1 - self.cursor_pos.1).abs() > 0.5;
        if cursor_moved && self.consumable_focus.is_some() {
            self.consumable_focus = None;
        }
        self.cursor_pos = ctx.cursor_pos;

        // Cache the latest wind timing from the cascade tuning so live
        // tweaks in the debug overlay take effect on the next frame and so
        // `draw()` (no `cascade_tuning` access) can read these.
        self.wind_delay_secs = ctx.cascade_tuning.wind_delay_ms as f32 / 1000.0;
        self.wind_duration_secs = ctx.cascade_tuning.wind_duration_ms as f32 / 1000.0;
        // First deal of the scene: shorten the wind_delay so the post-deal
        // sweep fires shortly after the scene fade-in completes, blowing
        // away the dense smoke curtain that the pick-blind scene already
        // pumped into the persistent fluid sim. Subsequent post-discard
        // refills get the normal (cascade-tuned) delay because they're
        // ambient breath, not a transition handoff.
        if self.initial_smoke_fill_active {
            self.wind_delay_secs = OPENING_WIND_DELAY_SECS;
        }

        // Detect deal events: any time the hand grows (initial round deal,
        // post-discard refill) we stamp `last_deal_at` so the post-deal smoke
        // gust can fire `wind_delay_secs` later.
        let cur_hand_len = ctx.run.hand.len();
        if cur_hand_len > self.prev_hand_len {
            self.last_deal_at = Some(now);
        }
        self.prev_hand_len = cur_hand_len;

        // The opening smoke curtain (`initial_smoke_fill_active`) is a
        // one-shot: it floods the screen with positive-density impulses for
        // the `wind_delay_secs` window after the *first* deal of this scene
        // and then gets blown away by the existing post-deal wind sweep.
        // Once that sweep finishes, clear the flag so subsequent
        // post-discard refills don't re-flood the screen.
        if self.initial_smoke_fill_active {
            if let Some(deal_at) = self.last_deal_at {
                let elapsed = now.saturating_duration_since(deal_at).as_secs_f32();
                if elapsed >= self.wind_delay_secs + self.wind_duration_secs {
                    self.initial_smoke_fill_active = false;
                }
            }
        }

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
                let delay = self.wind_delay_secs;
                let dur = self.wind_duration_secs.max(0.001);
                if elapsed < delay || elapsed >= delay + dur {
                    1.0
                } else {
                    let t = (elapsed - delay) / dur;
                    let env = (4.0 * t * (1.0 - t)).clamp(0.0, 1.0);
                    1.0 - 0.88 * env
                }
            })
            .unwrap_or(1.0);
        self.candle_time += dt;
        for c in self.candles.iter_mut() {
            let t = self.candle_time;
            let target =
                (0.9 + 0.18 * (t * 7.3 + c.phase).sin() + 0.08 * (t * 13.1 + c.phase * 1.7).sin())
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
            // The pause menu's "Glossary" entry sets a one-shot flag and
            // closes itself; drain the flag here so the gameplay glossary
            // overlay opens on the very next frame.
            if self.pause_menu.take_glossary_request() {
                self.glossary.toggle();
            }
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
                        // Rain a burst of physical scoring bones onto the
                        // play area below the modifier strip — chips on the
                        // left half of the strip, mult on the right half, so
                        // the falling pile reads back to the HUD token that
                        // just popped. Final-beat steps spawn from center.
                        let bone_kind = match step.kind {
                            StepKind::Chips => Some(CascadeTokenKind::Chips),
                            StepKind::Mult => Some(CascadeTokenKind::Mult),
                            StepKind::Final => None,
                        };
                        if let Some(kind) = bone_kind {
                            let ms = ctx.layout.modifier_strip;
                            let anchor_px = match kind {
                                CascadeTokenKind::Chips => ms.x + ms.w * 0.30,
                                CascadeTokenKind::Mult => ms.x + ms.w * 0.70,
                            };
                            let anchor_py = ms.y + ms.h + 20.0;
                            self.falling_bones.burst(anchor_px, anchor_py, 6, kind);
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
                // Wipe the physical bones the moment scoring ends so the
                // play space clears for the next hand.
                self.falling_bones.clear();
            } else {
                // Allow skip on any key press during cascade.
                if !ctx.actions.is_empty() {
                    cascade.skip();
                    self.displayed_score = ctx.run.round_score;
                    self.cascade = None;
                    self.last_revealed_step = None;
                    self.cascade_final_emitted = false;
                    self.falling_bones.clear();
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
                match ctx.run.use_consumable(idx) {
                    Some(crate::game::run::ConsumableUseResult::Zodiac { yaku, new_level }) => {
                        log::info!("Used Zodiac → {} now level {}", yaku.name(), new_level);
                    }
                    Some(crate::game::run::ConsumableUseResult::Talisman { kind }) => {
                        log::info!(
                            "Used {} — every tile in hand stamped with {:?}",
                            kind.name(),
                            kind.enhancement()
                        );
                    }
                    None => {}
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

        // Clamp consumable focus to the current capacity (the player may
        // have spent the focused consumable last frame, or the slot count
        // could have shrunk after a sale). Out-of-range focus collapses
        // back to None so the next NavigateHudNext re-enters at slot 0.
        let consumable_capacity = ctx.run.consumables.capacity;
        if let Some(idx) = self.consumable_focus {
            if idx >= consumable_capacity {
                self.consumable_focus = None;
            }
        }

        // Handle button-bar focus navigation. Three focus targets exist:
        // hand tiles (default, focus state lives in `ctx.focus_tile_index`),
        // the bottom button bar (`button_focus`), and the consumable strip
        // in the top-right (`consumable_focus`). At most one of the latter
        // two is non-None at a time — `NavigateHudNext` / `NavigateHudPrev`
        // (LB / RB on controller, `[` / `]` on the keyboard) cycle into
        // the consumable strip from any state and step through its slots.
        let mut actions_for_scene: Vec<UiAction> = Vec::new();
        for &a in ctx.actions.iter() {
            match a {
                // Cycle into / through the consumable strip. Stepping past
                // the last slot wraps back to "no consumable focus" so the
                // player can return to hand-tile / button-bar focus
                // without needing a separate keybind.
                UiAction::NavigateHudNext if consumable_capacity > 0 => {
                    self.consumable_focus = match self.consumable_focus {
                        None => Some(0),
                        Some(i) if i + 1 >= consumable_capacity => None,
                        Some(i) => Some(i + 1),
                    };
                    if self.consumable_focus.is_some() {
                        // Entering consumable focus releases button-bar focus
                        // so Confirm unambiguously activates the consumable.
                        self.button_focus = None;
                    }
                    continue;
                }
                UiAction::NavigateHudPrev if consumable_capacity > 0 => {
                    self.consumable_focus = match self.consumable_focus {
                        None => Some(consumable_capacity - 1),
                        Some(0) => None,
                        Some(i) => Some(i - 1),
                    };
                    if self.consumable_focus.is_some() {
                        self.button_focus = None;
                    }
                    continue;
                }
                // Cancel exits consumable focus (like Backspace clearing
                // tile selection — gives the player a way back out).
                UiAction::Cancel if self.consumable_focus.is_some() => {
                    self.consumable_focus = None;
                    continue;
                }
                // Confirm on the consumable strip activates the focused
                // consumable. Drives the same `use_consumable` path as the
                // mouse-click handler above, then clears focus so the next
                // press doesn't double-fire on whatever shifted into the
                // freed slot.
                UiAction::Confirm if self.consumable_focus.is_some() => {
                    let idx = self.consumable_focus.unwrap();
                    if let Some(result) = ctx.run.use_consumable(idx) {
                        match result {
                            crate::game::run::ConsumableUseResult::Zodiac { yaku, new_level } => {
                                log::info!("Used Zodiac → {} now level {}", yaku.name(), new_level,);
                            }
                            crate::game::run::ConsumableUseResult::Talisman { kind } => {
                                log::info!(
                                    "Used {} — every tile in hand stamped with {:?}",
                                    kind.name(),
                                    kind.enhancement(),
                                );
                            }
                        }
                    }
                    self.consumable_focus = None;
                    continue;
                }
                UiAction::FocusDown => {
                    if self.consumable_focus.is_some() {
                        // Drop out of consumable focus into the button bar.
                        self.consumable_focus = None;
                        self.button_focus = Some(GameplayButton::Play);
                        continue;
                    }
                    if self.button_focus.is_none() {
                        self.button_focus = Some(GameplayButton::Play);
                    }
                    continue;
                }
                UiAction::FocusUp => {
                    if self.consumable_focus.is_some() {
                        self.consumable_focus = None;
                        continue;
                    }
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

        // 3D-hit dispatcher: when the catch-all `GAMEPLAY_3D_HIT_ID`
        // button fires, route the click based on what the renderer's
        // raycast picker actually hit this frame. The picker is the
        // single source of truth for which 3D action object the cursor
        // is over — we synthesise the same `UiAction`s that the keyboard
        // shortcuts enqueue and append them to `actions_for_scene`, so
        // the rest of the update flow (cascade-active gating, focus
        // highlighting, animation triggers, etc.) is identical for
        // mouse and keyboard. Yaku tablets are hover-only and don't
        // contribute clicks.
        for &cid in ctx.button_clicks {
            if cid != GAMEPLAY_3D_HIT_ID {
                continue;
            }
            use crate::render::wgpu_renderer::GameplayPick;
            let action = match ctx.picked_gameplay_object {
                Some(GameplayPick::WoodTablet(0)) => Some(UiAction::SortBySuit),
                Some(GameplayPick::WoodTablet(1)) => Some(UiAction::SortByRank),
                Some(GameplayPick::WoodTablet(2)) => Some(UiAction::ScoreHand),
                Some(GameplayPick::DiscardBowl) => Some(UiAction::CommitDiscard),
                _ => None,
            };
            if let Some(a) = action {
                actions_for_scene.push(a);
            }
        }

        // Clear any previous frame's departures.
        self.pending_departures.clear();

        // Debug: `B` blows a strong gust of wind at the candle row so the
        // flame's wind reaction is observable on demand. Stamps a timer the
        // draw step reads to emit the actual `WindGust` impulses.
        if actions_for_scene
            .iter()
            .any(|a| matches!(a, UiAction::DebugBlowWind))
        {
            self.debug_wind_at = Some(now);
            log::info!("[debug] candle wind gust triggered");
        }

        if actions_for_scene
            .iter()
            .any(|a| matches!(a, UiAction::DebugToggleAxes))
        {
            self.debug_show_axes = !self.debug_show_axes;
            log::info!(
                "[debug] world-axes overlay {}",
                if self.debug_show_axes { "ON" } else { "OFF" }
            );
        }

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

    /// Gameplay is migrated off the legacy `SceneDrawOutput` dual-vec
    /// model — its draw output flows through `draw_frame` instead, where
    /// quads, text, and 3D markers all interleave inside a single ordered
    /// `UiFrame.cmds` list. The trait still requires `draw`, but main.rs
    /// only calls `draw_frame` and the default `draw_frame` impl is
    /// overridden here, so this stub is unreachable in normal flow.
    fn draw(&self, _ctx: DrawCtx<'_>) -> SceneDrawOutput {
        unreachable!(
            "GameplayScene uses draw_frame; the legacy dual-vec draw path is not implemented. \
             Call SceneBehavior::draw_frame instead."
        )
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let layout = ctx.layout;
        let run = ctx.run;
        let focus = ctx.focus_tile_index.min(run.hand.len().saturating_sub(1));
        let now = Instant::now();
        let glossary_open = self.glossary.open;

        // The window title is recomputed unconditionally so the OS chrome
        // tracks the current run state even when the glossary takes over
        // the screen.
        let window_title = format!(
            "Mahjuro — {} Round {}  {} / {}  Gold: {}  Hands: {}  Discards: {}",
            run.blind.name(),
            run.run_number,
            if self.cascade.is_some() {
                self.displayed_score
            } else {
                run.round_score
            },
            run.target_score,
            run.gold,
            run.plays_remaining,
            run.discards_remaining,
        );

        // ── Glossary early-return path ──────────────────────────────────
        //
        // When the glossary overlay is open, the entire HUD / 3D scene is
        // suppressed and the glossary panel takes over the screen. The
        // legacy code handled this by drawing the HUD and then having
        // `GlossaryOverlay::draw` blow the per-frame vecs away — that hack
        // is the original source of the tooltip-z-order bug class. The
        // canonical-frame port instead returns a *fresh* `UiFrame`
        // containing only the background and the glossary panel.
        if glossary_open {
            let mut frame = UiFrame::new();
            frame.background(super::BackgroundId::Gameplay);
            self.glossary
                .draw_into_frame(&mut frame, layout.window_w, layout.window_h);
            // Click-blocker behind the glossary's own buttons (added last
            // so it never preempts a real button click).
            frame.buttons.push(ButtonDef::scene(
                (0.0, 0.0, layout.window_w, layout.window_h),
                u32::MAX,
            ));
            frame.focus = focus;
            frame.window_title = window_title;
            return frame;
        }

        let ts = ctx.anim.transform_for(ENTITY_SCORE_PANEL);

        // Score-panel cartouche + modifier strip backplane quads. Returned
        // as a vec because they need to land in the persistent-HUD section
        // of the frame, which is built further down.
        let score_panel_quads = build_instances_from_layout(
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
        // On boss blinds, prefer the boss's themed name over the generic
        // "Boss Blind" label so the player can read what they're up against
        // at a glance.
        let blind_label: String = if run.blind == crate::core::rules::BlindKind::Boss {
            run.upcoming_boss
                .map(|k| k.def().name.to_string())
                .unwrap_or_else(|| run.blind.name().to_string())
        } else {
            run.blind.name().to_string()
        };
        let score_text_top = format!(
            "{}  ·  R{}  ·  {} / {}",
            blind_label, run.run_number, shown_score, run.target_score,
        );
        let score_text_bot = format!(
            "${}  ·  Wall {}  ·  Wind {}  ·  {}{}",
            run.gold, tiles_left, wind_label, shanten_text, dora_section
        );
        // Captured for the hanging plaque cmd built later in `frame.cmds`.
        // The plaque carries the same two-line payload as a per-instance
        // decal texture engraved onto the wood face — no 2D overlay text
        // sits on top of the wood anymore, so the smoke composite can
        // drift over the plaque without text bleeding through it.
        let plaque_top_text = score_text_top.clone();
        let plaque_bot_text = score_text_bot.clone();
        // Boss-rule ofuda payload: derived independently from `run` so the
        // hanging paper always reflects the active boss rule, regardless of
        // whether a cascade is currently animating in the modifier strip.
        let (ofuda_title_text, ofuda_rule_text) =
            if run.blind == crate::core::rules::BlindKind::Boss {
                if let Some(k) = run.upcoming_boss {
                    let d = k.def();
                    let desc: &str = run
                        .upcoming_boss_effect
                        .as_ref()
                        .and_then(|e| e.description_override.as_deref())
                        .unwrap_or(d.description);
                    (d.name.to_string(), desc.to_string())
                } else {
                    (String::new(), String::new())
                }
            } else {
                (String::new(), String::new())
            };

        // Modifier strip: cascade / sets (full width). Relics shown as row below score panel.
        let ms = layout.modifier_strip;

        // Dynamic readout buffers — when populated by the cascade branch, they
        // replace the legacy single-line cascade text label (which becomes "").
        // Phase 6: the chips/mult pills become physical engraved bone tokens
        // pushed via `cascade_token_placements`; the legacy `cascade_instances`
        // flat-quad buffer is no longer populated, only the text labels.
        let mut cascade_token_placements: Vec<crate::render::draw_cmd::CascadeTokenPlacement> =
            Vec::new();
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

            // Chips token — engraved bone, cool indigo tint. The pulse
            // envelope from `chip_pulse` (computed above) drives the
            // renderer's per-instance scale-up so the active axis pops on
            // each scoring step.
            let chips_x = inner_x;
            {
                let cx = chips_x + pill_w * 0.5;
                let cy = pill_y + pill_h * 0.5;
                // Pull the existing chip_pulse value out of the [1.0, 1.12]
                // band the pill code used so the cascade token batch can
                // scale uniformly. (chip_pulse - 1) / 0.12 maps the active
                // band into [0, 1] for the renderer's pulse field.
                let pulse_t = ((chip_pulse - 1.0) / 0.12).clamp(0.0, 1.0);
                cascade_token_placements.push(crate::render::draw_cmd::CascadeTokenPlacement {
                    world_pos: [cx, cy, 4.0],
                    extents: [pill_w, (pill_h * 0.6).max(8.0), pill_h],
                    kind: crate::render::draw_cmd::CascadeTokenKind::Chips,
                    pulse: pulse_t,
                });
                cascade_labels.push(TextLabel {
                    rect: [chips_x, pill_y, pill_w, pill_h],
                    text: format!("{}", frame.displayed_chips),
                    color: [1.0, 0.93, 0.55, 1.0],
                    ..Default::default()
                });
            }

            // Mult token — engraved bone, warm crimson tint.
            let mult_x = inner_x + pill_w + cross_w;
            let mult_value = frame.displayed_mult;
            let mult_str = if (mult_value - mult_value.round()).abs() < 0.05 {
                format!("×{}", mult_value.round() as i64)
            } else {
                format!("×{:.1}", mult_value)
            };
            {
                let cx = mult_x + pill_w * 0.5;
                let cy = pill_y + pill_h * 0.5;
                let pulse_t = ((mult_pulse - 1.0) / 0.12).clamp(0.0, 1.0);
                cascade_token_placements.push(crate::render::draw_cmd::CascadeTokenPlacement {
                    world_pos: [cx, cy, 4.0],
                    extents: [pill_w, (pill_h * 0.6).max(8.0), pill_h],
                    kind: crate::render::draw_cmd::CascadeTokenKind::Mult,
                    pulse: pulse_t,
                });
                cascade_labels.push(TextLabel {
                    rect: [mult_x, pill_y, pill_w, pill_h],
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
                // On boss blinds, surface the boss's effect description here
                // so the player has a constant reminder of what's hurting them.
                let idle_text = if run.blind == crate::core::rules::BlindKind::Boss {
                    run.upcoming_boss
                        .map(|k| {
                            let d = k.def();
                            // Reactive bosses (Mirror, Tax Collector) have a
                            // resolved description set at reveal time — prefer
                            // it so the in-fight reminder shows the actual
                            // chosen variant, not the generic static text.
                            let desc: &str = run
                                .upcoming_boss_effect
                                .as_ref()
                                .and_then(|e| e.description_override.as_deref())
                                .unwrap_or(d.description);
                            format!("{} — {}", d.name, desc)
                        })
                        .unwrap_or_else(|| "Select tiles to play".to_string())
                } else {
                    "Select tiles to play".to_string()
                };
                (idle_text, [0.85, 0.7, 0.4, 1.0])
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

        // ── Frame accumulators ───────────────────────────────────────────
        //
        // The migrated draw_frame separates HUD content into three layers
        // and pushes them into the final `UiFrame` at the end of this
        // function in canonical order:
        //
        //   1. PERSISTENT HUD (`hud_quads` + `hud_text`) — score panel,
        //      yaku cards, button bar, zodiac slots, particles, help
        //      badge. Lives between the 3D backdrop and the
        //      `HandTileFaces` marker so tile faces read on top of HUD
        //      panels they overlap.
        //
        //   2. HOVER LAYER (`hover_quads` + `hover_text`) — tile hover
        //      outline, tile/yaku/zodiac/relic hover tooltips. Lives
        //      *after* the `HandTileFaces` marker so tooltips always sit
        //      on top of tile faces.
        //
        //   3. PAUSE OVERLAY (`pause_quads` + `pause_text`) — built only
        //      when the pause menu is open, sits above the hover layer.
        //
        // Within each accumulator, quads are pushed first then text on
        // top, which preserves the existing visual contract (text reads
        // above panels). The crucial fix vs. the legacy dual-vec model is
        // that hover tooltips' background quads no longer compete with
        // persistent text — they're a separate downstream batch.
        let mut hud_quads: Vec<GpuInstance> = Vec::new();
        let hud_text: Vec<TextLabel> = Vec::new();
        let mut hover_quads: Vec<GpuInstance> = Vec::new();
        let mut hover_text: Vec<TextLabel> = Vec::new();

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
        // Captured during the loop below — `(yaku_kind, anchor_x, anchor_y)`
        // for the card the cursor is currently hovering, if any. The tooltip
        // is pushed into the *hover layer* after the loop completes so it
        // draws on top of every yaku card regardless of which one captured it.
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

        // Phase 3: yaku selectors are now physical bone tablets sitting in
        // a row in front of the hand. The flat slate-blue card quads + the
        // progress-fill bar are gone — replaced by `YakuTabletBatch` that
        // the renderer dispatches through the lit-mesh pipeline. The 2D
        // text labels stay as a screen-space overlay until the engraved
        // decal pass lands; hover tracking still uses the original screen
        // rect (the cards live in the same pixel region as before).
        let mut yaku_tablet_placements: Vec<crate::render::draw_cmd::YakuTabletPlacement> =
            Vec::new();
        if !visible_previews.is_empty() {
            let panel_h = (66.0 * scale).max(48.0);
            let panel_gap = 8.0 * scale;
            let panel_y = btn_y - panel_h - panel_gap;
            let panel_w = container_w;
            let panel_x = container_x;
            let n = visible_previews.len() as f32;
            let card_gap = 6.0 * scale;
            let card_w = (panel_w - card_gap * (n - 1.0)) / n;
            // Tablets are flat-on-table dominoes: extents[0] is width
            // (matches card width), extents[1] is the thickness above the
            // wood, extents[2] is depth (matches card height into the
            // scene).
            let tablet_thickness = (8.0 * scale).max(6.0);
            for (i, p) in visible_previews.iter().enumerate() {
                let cx = panel_x + i as f32 * (card_w + card_gap);
                let cy = panel_y;
                let center_px = cx + card_w * 0.5;
                let center_py = cy + panel_h * 0.5;
                // Hover state comes from the renderer's raycast picker
                // (precomputed local AABB × per-frame model matrix), not
                // from screen-rect hit-testing the projected AABB. The
                // picker is one frame stale, like every other 3D pick
                // path in the engine.
                let hovered_now = matches!(
                    ctx.picked_gameplay_object,
                    Some(crate::render::wgpu_renderer::GameplayPick::YakuTablet(j))
                        if j == i,
                );
                yaku_tablet_placements.push(crate::render::draw_cmd::YakuTabletPlacement {
                    world_pos: [center_px, center_py, 0.0],
                    extents: [card_w, tablet_thickness, panel_h],
                    name: p.kind.name().to_string(),
                    progress: p.progress,
                    active: p.active,
                    hover: if hovered_now { 1.0 } else { 0.0 },
                });
                // The yaku name is now engraved directly on the bone tablet
                // via a per-instance decal texture (see the renderer's tablet
                // pass), so no 2D text overlay is pushed here.

                // Hover tracking for the tooltip pass below the loop.
                // Anchor the tooltip to the *projected* on-screen rect so
                // it pops up next to the tablet the player can actually
                // see — falls back to the input pixel rect on the first
                // frame before projection data is available.
                if hovered_now {
                    let (ax, ay) = match ctx.projected_yaku_tablet_rects.get(i).copied() {
                        Some([px, py, pw, _ph]) if pw > 0.0 && px.is_finite() && py.is_finite() => {
                            (px + pw * 0.5, py)
                        }
                        _ => (center_px, cy),
                    };
                    hovered_yaku = Some((p.kind, ax, ay));
                }
            }
        }
        // Yaku card hover tooltip — pushed into the *hover layer* so its
        // background quad lands AFTER every persistent HUD text label, which
        // is the structural fix for the legacy "tooltip BG renders under
        // parent text" bug class.
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
                        crate::core::yaku::YakuKind::FullHand
                            | crate::core::yaku::YakuKind::Yakuhai
                    )
                {
                    "yes (full strength)"
                } else {
                    "no (scores at 50%)"
                },
            );
            push_tooltip(
                &mut hover_quads,
                &mut hover_text,
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
        // Phase 4: action row is now physical objects.
        //   - Sort by Suit / Sort by Rank → carved wood tablets
        //   - Play Hand                   → carved wood tablet (same mesh)
        //   - Discard                     → lacquered wood bowl
        // The flat slate-blue button background quads are gone; only the
        // focus-highlight border remains as a 2D affordance for keyboard
        // navigation.
        let mut wood_tablet_placements: Vec<crate::render::draw_cmd::WoodTabletPlacement> =
            Vec::new();
        let mut discard_bowl_placement: Option<crate::render::draw_cmd::BowlPlacement> = None;
        let play_enabled = selection_valid && run.plays_remaining > 0;
        let discard_enabled = selected_count > 0 && run.discards_remaining > 0;
        for (i, &(bx, by, bw, bh)) in btn_rects.iter().enumerate() {
            // The hover state for the 3D action objects comes from the
            // renderer's raycast picker against precomputed local AABBs —
            // no projected screen rects, no per-frame intersection of
            // input pixel rects with camera-distorted AABBs. The picker
            // is one frame stale, like every other 3D pick path.
            let pick = ctx.picked_gameplay_object;
            let hovered = match i {
                0 => matches!(
                    pick,
                    Some(crate::render::wgpu_renderer::GameplayPick::WoodTablet(0)),
                ),
                1 => matches!(
                    pick,
                    Some(crate::render::wgpu_renderer::GameplayPick::WoodTablet(1)),
                ),
                2 => matches!(
                    pick,
                    Some(crate::render::wgpu_renderer::GameplayPick::WoodTablet(2)),
                ),
                3 => matches!(
                    pick,
                    Some(crate::render::wgpu_renderer::GameplayPick::DiscardBowl),
                ),
                _ => false,
            };
            // Keyboard focus highlight still uses the input pixel rect:
            // it's a 2D affordance for keyboard nav and never needs to
            // sync with the visible 3D footprint.
            if self.button_focus == Some(ALL_BUTTONS[i]) {
                let pad = 3.0;
                hud_quads.push(GpuInstance {
                    rect: [bx - pad, by - pad, bw + pad * 2.0, bh + pad * 2.0],
                    color: [0.9, 0.8, 0.2, 0.95],
                });
            }
            let center_px = bx + bw * 0.5;
            let center_py = by + bh * 0.5;
            let tablet_thickness = (bh * 0.35).max(8.0);
            match i {
                0 | 1 | 2 => {
                    let label = match i {
                        0 => "Sort by Suit",
                        1 => "Sort by Rank",
                        _ => "Play Hand",
                    };
                    let disabled = i == 2 && !play_enabled;
                    wood_tablet_placements.push(crate::render::draw_cmd::WoodTabletPlacement {
                        world_pos: [center_px, center_py, 0.0],
                        extents: [bw, tablet_thickness, bh],
                        label: label.to_string(),
                        pressed: 0.0,
                        hover: if hovered { 1.0 } else { 0.0 },
                        disabled,
                    });
                }
                3 => {
                    // Discard bowl. The bowl mesh now bakes its low / wide
                    // bowl proportions into local space (height ≈ 0.30 vs
                    // diameter 1.0), so we scale uniformly here and let the
                    // mesh shape it. We size the rim diameter generously
                    // — up to 2.4 × the button height — so the bowl reads
                    // as a clearly-bowl-shaped object next to the wood
                    // tablets, even when the action row is short.
                    let bowl_diam = bw.min(bh * 2.4);
                    // Nudge the bowl to the right of its slot center so it
                    // sits clearly outboard of the Play Hand tablet rather
                    // than crowding into it.
                    let bowl_cx = center_px + bw * 0.45;
                    discard_bowl_placement = Some(crate::render::draw_cmd::BowlPlacement {
                        world_pos: [bowl_cx, center_py, 0.0],
                        extents: [bowl_diam, bowl_diam, bowl_diam],
                        hover: if hovered && discard_enabled { 1.0 } else { 0.0 },
                    });
                }
                _ => {}
            }
        }

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
        // long the strings are. The cartouche header text + the modifier
        // strip cascade/idle text are kept in their own dedicated buffers
        // so the final assembly can place them between the score-panel
        // backplane quads and the rest of the HUD body.
        // Score header text is engraved directly onto the hanging plaque's
        // +Z face via the per-instance decal pipeline (see the plaque draw
        // path in `wgpu_renderer.rs` and `rasterize_plaque_decal` in
        // `decal.rs`). The two-line payload travels in `plaque_top_text` /
        // `plaque_bot_text` on the `PlaquePlacement` cmd pushed below — no
        // 2D overlay text is emitted for the header anymore, so the smoke
        // composite can drift over the wood face without text floating
        // on top of it.
        let _ = (ctx_x, ctx_y, ctx_w, ctx_h, &score_text_top, &score_text_bot);
        // Modifier strip: dynamic cascade widgets (when active) or status
        // text (idle). Anchored to the SECOND projected plaque rect (the
        // status placard pushed below the main score plaque) so the text
        // reads as if engraved on its wood face.
        let placard_rect = ctx
            .projected_plaque_rects
            .get(1)
            .copied()
            .unwrap_or([ms.x, ms.y, ms.w, ms.h]);
        let placard_pad_x = placard_rect[2] * 0.05;
        let placard_pad_y = placard_rect[3] * 0.10;
        let placard_inner = [
            placard_rect[0] + placard_pad_x,
            placard_rect[1] + placard_pad_y,
            (placard_rect[2] - placard_pad_x * 2.0).max(1.0),
            (placard_rect[3] - placard_pad_y * 2.0).max(1.0),
        ];
        let placard_font = (placard_inner[3] * 0.55).max(14.0);
        let mut modifier_strip_text: Vec<TextLabel> = Vec::new();
        if cascade_labels.is_empty() {
            modifier_strip_text.push(TextLabel {
                rect: placard_inner,
                text: cascade_text,
                color: cascade_color,
                font_px: Some(placard_font),
                ..Default::default()
            });
        } else {
            // Cascade labels keep their own pre-laid-out rects.
            modifier_strip_text.extend(cascade_labels);
        }
        // The Sort/Play labels are now engraved directly on the wood tablets
        // (per-instance decals applied in the renderer's tablet pass), and
        // the Discard bowl reads as a bowl visually, so no 2D button text
        // labels are pushed into the HUD overlay anymore.

        // The 3D action objects (sort suit / sort rank / play hand wood
        // tablets + discard bowl) no longer go through `frame.buttons`.
        // Their click routing is driven by `pick_gameplay_object` in
        // `main.rs`'s `MouseInput` handler — clicks land on whichever 3D
        // object the cursor is *actually* over per raycast, not whichever
        // 2D rect happens to overlap the cursor. This avoids the
        // perspective-distortion issues that plagued the projected-rect
        // approach. Keyboard nav (button_focus) still works because the
        // `update()` path enqueues UiActions directly.
        //
        // While paused, no gameplay buttons should be clickable — the
        // pause overlay swallows all input via its own buttons plus a
        // fullscreen blocker.
        let mut buttons: Vec<ButtonDef> = Vec::new();
        let _ = paused;

        // ── Consumable inventory bar (Zodiacs + Talismans) ───────────────
        //
        // Sits in the top-right corner of the screen, away from the score
        // cartouche and relic dish. Each slot is a clickable badge for one
        // consumable — Zodiacs level a yaku for the run, Talismans stamp
        // their enhancement onto every tile in the current hand at once.
        //
        // Phase 5: the flat slot backgrounds + gold rims are gone; the
        // consumable inventory now lives on a brass `DishExplicit` with
        // `TalismanBatch` pendants for each filled slot. The text labels
        // and click handlers stay at the same screen positions so hover +
        // input plumbing is unchanged.
        let consumables = &run.consumables;
        let mut talisman_dish_placements: Vec<crate::render::draw_cmd::TalismanPlacement> =
            Vec::new();
        let mut talisman_dish_strip: Option<(f32, f32, f32, f32)> = None;
        if consumables.capacity > 0 {
            let zscale = (layout.window_w.min(layout.window_h)) / 600.0;
            let slot_w = (140.0 * zscale).max(120.0);
            let slot_h = (56.0 * zscale).max(48.0);
            let gap = (6.0 * zscale).max(3.0);
            let total_w =
                slot_w * consumables.capacity as f32 + gap * (consumables.capacity as f32 - 1.0);
            let strip_x = layout.window_w - total_w - (16.0 * zscale);
            let strip_y = layout.score_panel.y + layout.score_panel.h + (8.0 * zscale);
            talisman_dish_strip = Some((strip_x, strip_y, total_w, slot_h));

            // ── Projection-aware slot rects ──────────────────────────────
            // The brass dish gets projected through the gameplay camera
            // to a different on-screen position than its raw pixel anchor.
            // We look up the previous frame's projected dish rect and
            // remap each per-slot rect through the same affine transform
            // (treating the dish as roughly planar). This keeps the
            // tooltip hit-test, focus ring, click target, and tooltip
            // anchor in lockstep with the visible pendant.
            //
            // Must match the dish-padding values used at the
            // `frame.dish_explicit(...)` push site below.
            let dish_pad_x_ratio = 0.10_f32;
            let dish_pad_y_ratio = 0.40_f32;
            let dish_pad_x = total_w * dish_pad_x_ratio;
            let dish_pad_y = slot_h * dish_pad_y_ratio;
            let orig_dish_x = strip_x - dish_pad_x;
            let orig_dish_y = strip_y - dish_pad_y;
            let orig_dish_w = total_w + dish_pad_x * 2.0;
            let orig_dish_h = slot_h + dish_pad_y * 2.0;
            let projected_dish = ctx
                .aux_dish_rects
                .iter()
                .find_map(|(pid, r)| (*pid == Some(PICK_CONSUMABLE_DISH)).then_some(*r));
            let slot_screen_rect = |slot_idx: usize| -> (f32, f32, f32, f32) {
                let raw_x = strip_x + slot_idx as f32 * (slot_w + gap);
                let raw_y = strip_y;
                if let Some([pdx, pdy, pdw, pdh]) = projected_dish {
                    if pdw > 0.0 && pdh > 0.0 {
                        let tx0 = (raw_x - orig_dish_x) / orig_dish_w;
                        let tx1 = (raw_x + slot_w - orig_dish_x) / orig_dish_w;
                        let ty0 = (raw_y - orig_dish_y) / orig_dish_h;
                        let ty1 = (raw_y + slot_h - orig_dish_y) / orig_dish_h;
                        let psx0 = pdx + pdw * tx0;
                        let psx1 = pdx + pdw * tx1;
                        let psy0 = pdy + pdh * ty0;
                        let psy1 = pdy + pdh * ty1;
                        return (psx0, psy0, psx1 - psx0, psy1 - psy0);
                    }
                }
                // First-frame fallback (no projection data yet) — use the
                // raw pixel rect. One frame of misalignment, then the
                // projected path takes over.
                (raw_x, raw_y, slot_w, slot_h)
            };

            for slot_idx in 0..consumables.capacity {
                // Pendant placement still uses the raw pixel anchors
                // (those get re-projected by the renderer for rendering).
                // The 2D overlays use the projected slot rect derived
                // from the dish.
                let zx = strip_x + slot_idx as f32 * (slot_w + gap);
                let zy = strip_y;
                let (slot_sx, slot_sy, slot_sw, slot_sh) = slot_screen_rect(slot_idx);
                if let Some(&item) = consumables.items.get(slot_idx) {
                    // Physical pendant on the dish — color encodes the
                    // consumable type. Zodiacs read jade-green, talismans
                    // pick up the talisman's enhancement family color.
                    let pendant_color = match item {
                        crate::core::consumable::Consumable::Zodiac(_) => [0.45, 0.78, 0.55, 1.0],
                        crate::core::consumable::Consumable::Talisman(_) => [0.92, 0.78, 0.32, 1.0],
                    };
                    talisman_dish_placements.push(crate::render::draw_cmd::TalismanPlacement {
                        center_pos: [zx + slot_w * 0.5, zy + slot_h * 0.5, 8.0],
                        extents: [slot_w * 0.55, slot_h * 0.85, 6.0],
                        rotation_y_deg: 0.0,
                        // Lay flat on the dish (face up). -90 around
                        // X (CW from the right view) rotates the
                        // tablet's front-face normal from +Z to +Y.
                        rotation_x_deg: -90.0,
                        color: pendant_color,
                    });
                    // The persistent on-slot labels (name + sub) are
                    // gone — the brass dish + colored pendant are the
                    // visual representation, and the hover tooltip below
                    // supplies the full name/description on demand.
                    let (tooltip_title, tooltip_body) = match item {
                        crate::core::consumable::Consumable::Zodiac(z) => {
                            let level = run.yaku_levels.level_of(z.yaku());
                            (
                                format!("{} (Zodiac)", z.name()),
                                format!(
                                    "Click or press to use. Permanently raises {} from level {} to {} for the rest of the run (+0.5 mult, +20 chips per level).",
                                    z.yaku().name(),
                                    level,
                                    level + 1,
                                ),
                            )
                        }
                        crate::core::consumable::Consumable::Talisman(t) => (
                            format!("{} (Talisman)", t.name()),
                            format!(
                                "Click or press to use. {} The enhancement persists on each tile until it's played or discarded.",
                                t.description()
                            ),
                        ),
                    };
                    if !paused {
                        buttons.push(ButtonDef::scene(
                            (slot_sx, slot_sy, slot_sw, slot_sh),
                            ZODIAC_USE_BASE + slot_idx as u32,
                        ));
                    }
                    // Cross-input focus ring: when this slot is the active
                    // controller / keyboard consumable focus, draw a brass
                    // outline around it so the player can see what Confirm
                    // will activate. Pushed into hud_quads so it lands on
                    // the same layer as the dish + pendant, before tooltips.
                    if Some(slot_idx) == self.consumable_focus {
                        let bt = (3.0 * scale).max(2.0);
                        let pad = (4.0 * scale).max(3.0);
                        let rx = slot_sx - pad;
                        let ry = slot_sy - pad;
                        let rw = slot_sw + pad * 2.0;
                        let rh = slot_sh + pad * 2.0;
                        let ring = [0.95, 0.78, 0.32, 1.0];
                        hud_quads.push(GpuInstance {
                            rect: [rx, ry, rw, bt],
                            color: ring,
                        });
                        hud_quads.push(GpuInstance {
                            rect: [rx, ry + rh - bt, rw, bt],
                            color: ring,
                        });
                        hud_quads.push(GpuInstance {
                            rect: [rx, ry, bt, rh],
                            color: ring,
                        });
                        hud_quads.push(GpuInstance {
                            rect: [rx + rw - bt, ry, bt, rh],
                            color: ring,
                        });
                    }
                    let (cx, cy) = self.cursor_pos;
                    if cx >= slot_sx
                        && cx <= slot_sx + slot_sw
                        && cy >= slot_sy
                        && cy <= slot_sy + slot_sh
                    {
                        push_tooltip(
                            &mut hover_quads,
                            &mut hover_text,
                            slot_sx + slot_sw * 0.5,
                            slot_sy,
                            layout.window_w,
                            layout.window_h,
                            &tooltip_title,
                            &tooltip_body,
                        );
                    }
                } else {
                    // Empty slot — no flat backdrop; the brass dish itself
                    // visually represents the empty cradle. Tooltip on hover.
                    let (cx, cy) = self.cursor_pos;
                    if cx >= slot_sx
                        && cx <= slot_sx + slot_sw
                        && cy >= slot_sy
                        && cy <= slot_sy + slot_sh
                    {
                        push_tooltip(
                            &mut hover_quads,
                            &mut hover_text,
                            slot_sx + slot_sw * 0.5,
                            slot_sy,
                            layout.window_w,
                            layout.window_h,
                            "Consumable Slot",
                            "Empty. Earn Zodiac cards from blind clears or buy Zodiacs and Talismans in the shop. Both share these slots.",
                        );
                    }
                }
            }
        }

        // Particle instances. Pushed into the persistent HUD layer (under
        // hand tile faces) to preserve the legacy z-order — score-cascade
        // bursts visually peek out *around* the tiles rather than over
        // them. Move to `hover_quads` if a future design wants particles
        // to fly over tile faces.
        for (rect, color) in self.particles.instances() {
            hud_quads.push(GpuInstance { rect, color });
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
        //
        // Also suppressed when the cursor is over any 2D UI element (a
        // button or a hovered yaku card). The 3D raycast pick happily
        // intersects a hand tile even when the cursor sits visually atop a
        // floating UI panel above the hand, which used to surface the tile
        // tooltip + ▼ pointer underneath the panel's own tooltip. Gating
        // here keeps a single tooltip on screen at a time.
        let cursor_over_ui = {
            let (cx, cy) = self.cursor_pos;
            let in_button = buttons.iter().any(|b| {
                let (bx, by, bw, bh) = b.rect;
                cx >= bx && cx <= bx + bw && cy >= by && cy <= by + bh
            });
            in_button || hovered_yaku.is_some()
        };
        if self.cascade.is_none() && !self.pause_menu.paused && !cursor_over_ui {
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
                    // Show the tile's *effective* value: base point worth
                    // plus every per-tile bonus that doesn't depend on the
                    // surrounding meld structure (talisman enhancements,
                    // dora, owned chip relics). The total chips line is the
                    // headline; the per-source breakdown follows so the
                    // player can see *why* the tile is worth what it is.
                    let dora_faces = run.wall.dora_faces();
                    let eff =
                        crate::core::scoring::tile_effective_value(tile, &run.relics, &dora_faces);
                    let name = tile.full_name();
                    let category = tile.category();
                    let is_selected = run.selected.get(idx).copied().unwrap_or(false);

                    let mut lines: Vec<String> = Vec::new();
                    lines.push(name);
                    if eff.bonus_chips != 0 || eff.mult_bonus != 0.0 {
                        // Effective chips (base + bonuses).
                        lines.push(format!(
                            "{category} · {} pts (base {})",
                            eff.total_chips(),
                            eff.base_chips,
                        ));
                    } else {
                        lines.push(format!("{category} · {} pts", eff.base_chips));
                    }
                    if eff.mult_bonus != 0.0 {
                        lines.push(format!("+{:.1} mult", eff.mult_bonus));
                    }
                    for (src, body) in &eff.sources {
                        lines.push(format!("{src}: {body}"));
                    }
                    if is_selected {
                        lines.push("selected".to_string());
                    }

                    // ── Geometry ──────────────────────────────────────
                    let line_h = 18.0 * scale;
                    let pad_x = 8.0 * scale;
                    let pad_y = 6.0 * scale;
                    let widest = lines.iter().map(|s| s.chars().count()).max().unwrap_or(0) as f32;
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

                    // Background. Pushed into the hover layer so the
                    // tooltip BG always lands ABOVE the persistent HUD
                    // text labels (this is the structural fix).
                    hover_quads.push(GpuInstance {
                        rect: [tx, ty, tw, th],
                        color: [0.06, 0.06, 0.12, 0.95],
                    });
                    // Gold border.
                    let bc = [0.65, 0.55, 0.25, 0.85];
                    let b = 1.5;
                    hover_quads.push(GpuInstance {
                        rect: [tx, ty, tw, b],
                        color: bc,
                    });
                    hover_quads.push(GpuInstance {
                        rect: [tx, ty + th - b, tw, b],
                        color: bc,
                    });
                    hover_quads.push(GpuInstance {
                        rect: [tx, ty, b, th],
                        color: bc,
                    });
                    hover_quads.push(GpuInstance {
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
                        hover_text.push(TextLabel {
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

        // Phase 8: the `?` glossary badge has been removed from the
        // gameplay HUD. The glossary is now reachable from the pause menu's
        // "Glossary" entry. The keyboard `Help` action shortcut still works
        // as a hidden affordance for power users.

        // Pause overlay — built into its own dedicated layer so it lands
        // ABOVE the hover layer in canonical push order. Reuses the
        // existing dual-vec `PauseMenu::draw` API with fresh local
        // accumulators (the pause menu has no internal interleaving
        // hazards — it's a dim panel + buttons + text where text-on-top is
        // the desired contract).
        let mut pause_quads: Vec<GpuInstance> = Vec::new();
        let mut pause_text: Vec<TextLabel> = Vec::new();
        self.pause_menu.draw(
            layout.window_w,
            layout.window_h,
            scale,
            &mut pause_quads,
            &mut pause_text,
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
        // The glossary overlay path has its own early-return at the top of
        // this function, so it does not appear here.

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
        let candle_centers: [(f32, f32); 5] = [
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
            // Footlight: a fifth candle planted *behind* the camera so
            // its pool spills forward across the bottom row of yaku and
            // wood tablets without the candle mesh itself floating into
            // the player's view. The renderer's `pixel_to_world` maps
            // pixel-y → table-z linearly, and the gameplay camera sits
            // around z = 0.95 * window_h (i.e. pixel-y ≈ 1.45 * window_h
            // in this mapping), so anything past ~1.5 * window_h is
            // safely clipped out of the view frustum. The point light
            // attached to the wick is what does the actual illumination.
            (layout.window_w * 0.5, layout.window_h * 1.55),
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
                3 => (jx.abs() * 26.0 * scale_c, -jy.abs() * 22.0 * scale_c),
                // Footlight (index 4): small symmetric jitter, never
                // pulled backward into the action row's footprint.
                _ => (jx * 14.0 * scale_c, jy.abs() * 8.0 * scale_c),
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
            // The footlight (index 4) sits well behind the camera, so its
            // wick is much farther from the action row than any of the
            // table-edge candles — bump its radius and intensity to
            // compensate, otherwise the front row stays in shadow.
            let (light_radius_mul, light_intensity) = if i == 4 { (2.2, 1.0) } else { (1.0, 2.3) };
            point_lights.push(PointLight {
                pos: [cx_j, cy_j, wick_world_y],
                radius: radius_px * light_radius_mul * (1.05 + 0.3 * candle.flicker),
                color: [1.0, 0.55, 0.22],
                intensity: light_intensity * candle.flicker,
            });
            let _ = candle_w;
        }

        // ── Hint lights ──────────────────────────────────────────────────
        // For each tile that the suggester flagged as a completion, push a
        // real green PointLight high above it. This replaces the old 2D
        // green-quad "light beam" overlay with an actual scene light: the
        // hinted tile (and its immediate neighbours) get a top-down green
        // pool through the same lighting model the candles use, including
        // the AABB occlusion + per-light Lambertian + clearcoat lobes.
        //
        // The light is positioned ~1.5 tile widths above the table with a
        // tight radius (~1.6 tile widths) so it stays localized to the
        // hinted tile rather than washing the whole hand. A slow sine pulse
        // on intensity makes it read as an active hint instead of a
        // permanent fixture, and the candle-style flicker hook lets the
        // post-deal wind gust dim hints alongside the real candles.
        if !hint_indices.is_empty() {
            let pulse = 0.75 + 0.25 * (self.candle_time * 4.0).sin();
            // Cap to whatever budget remains in the point-light buffer
            // after the candle plumes already pushed above. The shader
            // array is sized for 16, so the five candles plus up to ~11
            // hints fits comfortably; this clamp is defensive only.
            let hint_budget =
                crate::render::wgpu_renderer::MAX_POINT_LIGHTS.saturating_sub(point_lights.len());
            for &idx in hint_indices.iter().take(hint_budget) {
                let Some(&(sx, sy, sw, sh)) = hand_slots.get(idx) else {
                    continue;
                };
                let cx = sx + sw * 0.5;
                let cy = sy + sh * 0.5;
                point_lights.push(PointLight {
                    pos: [cx, cy, sw * 1.5],
                    radius: sw * 1.6,
                    color: [0.30, 1.00, 0.45],
                    intensity: 3.4 * pulse,
                });
            }
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
            // Scale relic-dish geometry with window size so the dish reads the
            // same proportionally at every resolution. The constants below were
            // tuned at the 1920x1080 design size, so a window-height ratio of
            // 1.0 at 1080 keeps them unchanged and shrinks them on smaller
            // windows along with the rest of the layout.
            let dish_scale = layout.window_h / 1080.0;
            // Each cell ~160px wide (at design size) so the relics read as
            // substantial physical objects rather than dice.
            let cell_w = 160.0_f32 * dish_scale;
            let total_w = cell_w * n;
            let start_x = row_center_x - total_w * 0.5 + cell_w * 0.5;
            for (i, &rid) in active_ids.iter().enumerate() {
                // Pseudo-random per-relic size variation, deterministic on id.
                let seed = (rid as u32).wrapping_mul(2654435761) ^ 0x9E3779B9;
                let r0 = ((seed >> 8) & 0xFF) as f32 / 255.0;
                let r1 = ((seed >> 16) & 0xFF) as f32 / 255.0;
                let r2 = ((seed >> 24) & 0xFF) as f32 / 255.0;
                let half_x = (55.0 + r0 * 18.0) * dish_scale;
                let half_y = (40.0 + r1 * 22.0) * dish_scale;
                let half_z = (38.0 + r2 * 16.0) * dish_scale;

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
                hover_quads.push(GpuInstance {
                    rect: [rx - t, ry - t, rw + t * 2.0, t],
                    color: rim,
                });
                hover_quads.push(GpuInstance {
                    rect: [rx - t, ry + rh, rw + t * 2.0, t],
                    color: rim,
                });
                hover_quads.push(GpuInstance {
                    rect: [rx - t, ry, t, rh],
                    color: rim,
                });
                hover_quads.push(GpuInstance {
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
                    let wrapped_lines =
                        widget::wrap_text(def.description, body_inner_w, body_line_h);
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
                    hover_quads.push(GpuInstance {
                        rect: [tip_x, tip_y, tip_w, tip_h],
                        color: bg,
                    });
                    // Gold border (4 thin quads).
                    let bt = 1.5_f32;
                    let border = crate::render::theme::color::BRASS;
                    hover_quads.push(GpuInstance {
                        rect: [tip_x, tip_y, tip_w, bt],
                        color: border,
                    });
                    hover_quads.push(GpuInstance {
                        rect: [tip_x, tip_y + tip_h - bt, tip_w, bt],
                        color: border,
                    });
                    hover_quads.push(GpuInstance {
                        rect: [tip_x, tip_y + bt, bt, tip_h - bt * 2.0],
                        color: border,
                    });
                    hover_quads.push(GpuInstance {
                        rect: [tip_x + tip_w - bt, tip_y + bt, bt, tip_h - bt * 2.0],
                        color: border,
                    });
                    hover_text.push(TextLabel {
                        rect: [tip_x + pad, tip_y + pad, tip_w - pad * 2.0, title_h],
                        text: def.name.to_string(),
                        color: crate::render::theme::color::CHAMPAGNE,
                        ..Default::default()
                    });
                    widget::push_text_block(
                        &mut hover_text,
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

        // The glossary-suppression branch from the legacy path is gone:
        // when the glossary is open we early-return a dedicated frame at
        // the very top of `draw_frame`, so by the time we reach this
        // point the glossary is *not* open and every variable below uses
        // its real value directly.

        // Post-deal smoke breath. `wind_delay_secs` after the most
        // recent deal we exhale a soft sweep of impulses across the hand
        // strip — a few evenly spaced points pushed back-and-up — so the
        // smoke that built up while the tiles were sliding in drifts off
        // toward the back of the table. The strength follows a 4t(1-t)
        // bell so the breath fades in and out instead of snapping on.
        let mut wind_gusts: Vec<crate::render::draw_cmd::WindGust> = Vec::new();
        let wind_delay = self.wind_delay_secs;
        let wind_duration = self.wind_duration_secs.max(0.001);
        {
            if let Some(deal_at) = self.last_deal_at {
                let elapsed = now.saturating_duration_since(deal_at).as_secs_f32();
                if elapsed >= wind_delay && elapsed < wind_delay + wind_duration {
                    let t = (elapsed - wind_delay) / wind_duration;
                    let envelope = (4.0 * t * (1.0 - t)).clamp(0.0, 1.0);
                    if !hand_slots.is_empty() {
                        // Sweep a 2D grid of impulses across the screen so
                        // the breath covers both the hand strip *and* the
                        // table interior above it. Horizontal axis: full
                        // window width with edge overshoot, so corner smoke
                        // gets shoved off-stage. Vertical axis: starts just
                        // below the hand strip (where smoke pools above the
                        // tile faces) and extends upward into the table all
                        // the way to the candle row, with each row lifted a
                        // bit higher off the table than the last so the
                        // sweep also catches the rising candle plumes.
                        let sw = hand_slots[0].2;
                        let sy = hand_slots[0].1;
                        // 6×4 = 24 impulses keeps us under MAX_INJECTIONS
                        // (32) with headroom for the 4 candle plumes and
                        // the cursor wind that share the same per-frame
                        // budget on the fluid sim.
                        const COLS: usize = 6;
                        const ROWS: usize = 4;
                        let win_w = layout.window_w;
                        let win_h = layout.window_h;
                        let x_pad = win_w * 0.12;
                        let span_min = -x_pad;
                        let span_max = win_w + x_pad;
                        // Vertical span: from a touch under the hand strip
                        // up to the back of the playable table area (~22%
                        // of window height from the top, where the candles
                        // and dish sit). Anything further back is outside
                        // the smoke grid anyway.
                        let y_bottom = sy + sw * 0.5;
                        let y_top = win_h * 0.22;
                        let radius =
                            ((win_w / COLS as f32) * 1.55).max((sy - y_top) / ROWS as f32 * 1.6);
                        for r in 0..ROWS {
                            // 0..1 across rows, 0 = nearest the player
                            let rf = (r as f32 + 0.5) / ROWS as f32;
                            let cy = y_bottom + (y_top - y_bottom) * rf;
                            // Lift higher for back rows so the gust also
                            // reaches the upper part of the candle plumes,
                            // not just the table surface.
                            let lift = 18.0 + 32.0 * rf;
                            // Back rows fade slightly so the sweep reads
                            // as a directional breath rolling forward, not
                            // a uniform wall of wind.
                            let row_strength = 1.0 - 0.25 * rf;
                            for c in 0..COLS {
                                let f = (c as f32 + 0.5) / COLS as f32;
                                let cx = span_min + (span_max - span_min) * f;
                                let edge_bias = (f - 0.5) * 2.0; // -1..1
                                let lateral = 28.0 * edge_bias * envelope * row_strength;
                                wind_gusts.push(crate::render::draw_cmd::WindGust {
                                    center_px: (cx, cy),
                                    lift,
                                    velocity: [
                                        lateral,
                                        (6.0 + 4.0 * rf) * envelope * row_strength,
                                        -55.0 * envelope * row_strength,
                                    ],
                                    radius,
                                    density: -0.04 * envelope * row_strength,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Opening smoke curtain. For the `wind_delay_secs` window after the
        // first deal of the scene, flood the table with positive-density
        // impulses so the player enters the round inside a wall of smoke.
        // The post-deal wind sweep above then blows it away on schedule.
        // Strength ramps in fast (~0.15s) so the curtain forms quickly,
        // holds, then tapers off in the last ~0.6s before the sweep so we
        // give the breath fresh smoke to clear instead of fighting it.
        if self.initial_smoke_fill_active {
            if let Some(deal_at) = self.last_deal_at {
                let elapsed = now.saturating_duration_since(deal_at).as_secs_f32();
                if elapsed < wind_delay {
                    // Top-up curtain runs during the brief window between
                    // the gameplay fade-in and the wind sweep, refreshing
                    // the smoke that pick_blind already pumped into the
                    // persistent fluid sim. Ramp in fast (~0.1s) so the
                    // curtain stays opaque, taper down in the last 0.15s
                    // so we hand fresh smoke off to the sweep instead of
                    // fighting it.
                    let ramp_in = (elapsed / 0.1).clamp(0.0, 1.0);
                    let ramp_out = ((wind_delay - elapsed) / 0.15).clamp(0.0, 1.0);
                    let strength = ramp_in.min(ramp_out);
                    if strength > 0.0 {
                        // 6×4 = 24 impulses, same budget shape as the sweep
                        // grid below — leaves headroom under MAX_INJECTIONS
                        // for candle plumes and the cursor wind.
                        const COLS: usize = 6;
                        const ROWS: usize = 4;
                        let win_w = layout.window_w;
                        let win_h = layout.window_h;
                        let x_pad = win_w * 0.15;
                        let span_min = -x_pad;
                        let span_max = win_w + x_pad;
                        let y_top = win_h * 0.22;
                        let y_bottom = if !hand_slots.is_empty() {
                            hand_slots[0].1 + hand_slots[0].3
                        } else {
                            win_h * 0.85
                        };
                        // Generous radius so neighbouring impulses overlap
                        // and there are no visible gaps in the curtain.
                        let radius = ((span_max - span_min) / COLS as f32 * 0.95)
                            .max((y_bottom - y_top) / ROWS as f32 * 1.6);
                        for r in 0..ROWS {
                            let rf = (r as f32 + 0.5) / ROWS as f32;
                            let cy = y_bottom + (y_top - y_bottom) * rf;
                            // Lift higher for back rows so the curtain
                            // climbs into the upper part of the volume the
                            // ray-march pass covers, not just the table.
                            let lift = 22.0 + 30.0 * rf;
                            for c in 0..COLS {
                                let f = (c as f32 + 0.5) / COLS as f32;
                                let cx = span_min + (span_max - span_min) * f;
                                wind_gusts.push(crate::render::draw_cmd::WindGust {
                                    center_px: (cx, cy),
                                    lift,
                                    // Tiny upward drift so the curtain
                                    // gently churns instead of looking
                                    // perfectly static while it holds.
                                    velocity: [0.0, 3.0, 0.0],
                                    radius,
                                    density: 0.35 * strength,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Debug: while the `B` gust timer is live, sweep a strong lateral
        // gust across the *same* grid the opening smoke curtain and the
        // post-deal breath use, so the wind footprint matches the smoke
        // footprint exactly — every cell that can hold smoke gets a
        // negative-density impulse with a big lateral velocity, instead of
        // only the four candle bases. The shape envelope is a 4t(1-t) bell
        // so the gust ramps in/out instead of popping. Velocity is
        // intentionally large compared to the post-deal sweep (~28 lateral)
        // — this is a "did the wiring work?" hammer, not a subtle ambient
        // effect, and the radius * 3.0 falloff inside
        // `wgpu_renderer::flame_anchors` still picks the grid impulses up
        // to bend the flames as the gust rolls across the candle row.
        if let Some(debug_at) = self.debug_wind_at {
            let elapsed = now.saturating_duration_since(debug_at).as_secs_f32();
            if elapsed < DEBUG_WIND_DURATION {
                let t = (elapsed / DEBUG_WIND_DURATION).clamp(0.0, 1.0);
                let envelope = (4.0 * t * (1.0 - t)).clamp(0.0, 1.0);
                // Mirror the curtain grid in `initial_smoke_fill_active`
                // above: same COLS/ROWS, same x_pad, same y_top/y_bottom,
                // same radius formula. If you change one, change both.
                const COLS: usize = 6;
                const ROWS: usize = 4;
                let win_w = layout.window_w;
                let win_h = layout.window_h;
                let x_pad = win_w * 0.15;
                let span_min = -x_pad;
                let span_max = win_w + x_pad;
                let y_top = win_h * 0.22;
                let y_bottom = if !hand_slots.is_empty() {
                    hand_slots[0].1 + hand_slots[0].3
                } else {
                    win_h * 0.85
                };
                let radius = ((span_max - span_min) / COLS as f32 * 0.95)
                    .max((y_bottom - y_top) / ROWS as f32 * 1.6);
                for r in 0..ROWS {
                    let rf = (r as f32 + 0.5) / ROWS as f32;
                    let cy = y_bottom + (y_top - y_bottom) * rf;
                    let lift = 22.0 + 30.0 * rf;
                    for c in 0..COLS {
                        let f = (c as f32 + 0.5) / COLS as f32;
                        let cx = span_min + (span_max - span_min) * f;
                        wind_gusts.push(crate::render::draw_cmd::WindGust {
                            center_px: (cx, cy),
                            lift,
                            velocity: [1400.0 * envelope, 0.0, -120.0 * envelope],
                            radius,
                            density: -0.04 * envelope,
                        });
                    }
                }
            }
        }

        // ── Frame assembly ──────────────────────────────────────────────
        //
        // Now push every layer into a fresh `UiFrame` in canonical order.
        // The single ordered cmd list (push order = z order) is what kills
        // the legacy "tooltip BG renders under parent text" bug class:
        // every hover-layer push happens *after* every persistent-HUD
        // text push, so a tooltip background can never compete with a
        // parent text label drawn in the same flush.
        let _ = relic_icons; // gameplay no longer renders 2D relic icons.
        let mut frame = UiFrame::new();
        frame.background(super::BackgroundId::Gameplay);
        frame.table();
        frame.hand_tile_backdrop();
        if !candle_placements.is_empty() {
            frame.candles(candle_placements.clone());
        }
        if !relic_placements.is_empty() {
            frame.dish();
            frame.relic_batch(relic_placements);
        }
        frame.flames(flame_instances);

        // PERSISTENT HUD: hanging plaque + ofuda (3D wood/paper) → score
        // panel pip indicators → score header text → modifier strip text →
        // yaku card bodies + button bar quads + zodiac slots + particles +
        // help badge → button labels + zodiac labels + help text.
        // Persistent quads first, then text on top of them — exactly the
        // behaviour the legacy flush had, just scoped to the persistent
        // layer instead of mixing with hover content.
        //
        // The wooden plaque replaces the legacy slate-blue cartouche.
        // Positioned in pixel space matching the score panel rect, with a
        // modest world-Y lift so it reads as hanging above the table. The
        // header text is engraved directly onto the +Z face via a
        // per-instance decal — see the plaque draw path in
        // `wgpu_renderer.rs` and `rasterize_plaque_decal` in `decal.rs`.
        let sp = layout.score_panel;
        let plaque_thickness = 8.0_f32;
        // Lift is proportional to window height so the plaque tracks the
        // camera (which also scales with `h` — see `eye_height = h * 0.55`
        // in the renderer). A fixed world-unit lift drifts downward as the
        // window grows because the table grows around a constant lift.
        let plaque_lift = layout.window_h * 0.30;
        // Push the plaque deeper into the scene (more negative world_z) so
        // it reads as hanging at the back of the room rather than right
        // above the player. pixel_y → world_z is a direct mapping in the
        // renderer's `pixel_to_world`, so subtracting from pixel_y here
        // moves the plaque back along the table's depth axis.
        let plaque_back_offset = layout.window_h * 0.18;
        frame.plaque(crate::render::draw_cmd::PlaquePlacement {
            center_pos: [
                sp.x + sp.w * 0.5,
                sp.y + sp.h * 0.5 - plaque_back_offset,
                plaque_lift,
            ],
            extents: [sp.w * 0.95, sp.h * 1.8, plaque_thickness],
            rotation_y_deg: 0.0,
            top_text: plaque_top_text,
            bot_text: plaque_bot_text,
        });
        // ── Status placard ───────────────────────────────────────────
        // A second, smaller wooden placard hanging just below the main
        // score plaque. Carries the modifier strip text — the contextual
        // "Select tiles to play" instruction at idle, the live hand
        // preview when tiles are selected, and the cascade breakdown
        // during scoring. The 2D modifier_strip_text labels are anchored
        // to this placard's projected screen rect via
        // `projected_plaque_rects[1]` below.
        let ms_for_placard = layout.modifier_strip;
        let placard_lift = layout.window_h * 0.22;
        frame.plaque(crate::render::draw_cmd::PlaquePlacement {
            center_pos: [
                ms_for_placard.x + ms_for_placard.w * 0.5,
                ms_for_placard.y + ms_for_placard.h * 0.5 - plaque_back_offset,
                placard_lift,
            ],
            extents: [
                ms_for_placard.w * 0.65,
                ms_for_placard.h * 2.6,
                plaque_thickness * 0.7,
            ],
            rotation_y_deg: 0.0,
            top_text: String::new(),
            bot_text: String::new(),
        });
        // Ofuda only appears on boss blinds (where there's a rule to show).
        if !ofuda_title_text.is_empty() {
            let ms_rect = layout.modifier_strip;
            let ofuda_lift = layout.window_h * 0.085;
            frame.ofuda(crate::render::draw_cmd::OfudaPlacement {
                center_pos: [
                    ms_rect.x + ms_rect.w * 0.85,
                    ms_rect.y + ms_rect.h * 0.5,
                    ofuda_lift,
                ],
                extents: [ms_rect.w * 0.28, ms_rect.h * 1.4, 4.0],
                rotation_y_deg: 0.0,
                title: ofuda_title_text,
                rule: ofuda_rule_text,
            });
        }
        // Plays/discards pip indicators are a *physical* peg block
        // floating in front of the hanging score plaque. Two rows of
        // cylinder pegs sit in it — top row = jade plays remaining,
        // bottom row = amber discards remaining — wired through
        // `PegBlockPlacement` and rendered by the lit-mesh pipeline
        // alongside the rest of the table.
        //
        // Placement note: an earlier version positioned the block at the
        // same pixel-y and world-Y as the plaque, intending to "hang it
        // off the plaque's right side". The plaque is tilted 35° forward
        // and scaled to ~`sp.h * 1.8` tall, so its tilted bounding box
        // swallows that whole region — the block ended up depth-tested
        // *behind* the plaque face and never showed up on screen. The
        // fix is to push the block forward (larger pixel-y → larger
        // world Z, toward camera) and lower (smaller world Y) so it
        // hangs clearly below + in front of the plaque, like a physical
        // counter sitting on the table edge instead of buried in the
        // plaque's volume.
        // Drop the legacy `score_panel_quads` (the old top-of-screen pip
        // strip) — its data is now driven from this peg block.
        let _ = score_panel_quads;
        {
            let peg_block_w = (sp.w * 0.30).max(160.0);
            let peg_block_h = (28.0 * scale).max(20.0);
            let peg_block_d = (52.0 * scale).max(32.0);
            // Center horizontally under the plaque so both rows of pegs
            // are visible regardless of how wide the panel renders.
            let peg_block_x = sp.x + sp.w * 0.5;
            // Pixel-y well below the score panel rect → forward in world Z,
            // clear of the tilted plaque's forward extent.
            let peg_block_y = sp.y + sp.h * 1.20;
            // Hang low above the table — well below the plaque's lift —
            // so the camera (looking down) sees the top face and the pegs
            // poking up out of it.
            let peg_block_lift = layout.window_h * 0.06;
            frame.peg_block(crate::render::draw_cmd::PegBlockPlacement {
                world_pos: [peg_block_x, peg_block_y, peg_block_lift],
                extents: [peg_block_w, peg_block_h, peg_block_d],
                plays_left: run.plays_remaining,
                plays_max: STARTING_PLAYS,
                discards_left: run.discards_remaining,
                discards_max: STARTING_DISCARDS,
            });
        }
        // (Score header text is now engraved on the plaque mesh as a
        // per-instance decal — no overlay TextLabels are pushed here.)
        // Phase 6: cascade scoring tokens (engraved bone, chips + mult)
        // pop in during a scoring cascade. Pushed before the cascade text
        // labels so the numbers read on top of the wood.
        if !cascade_token_placements.is_empty() {
            frame.cascade_token_batch(cascade_token_placements);
        }
        // Physical scoring bones tumbling onto the play space during a
        // cascade. Spawned by the reveal-edge handler in `update()` and
        // cleared the moment the cascade ends, so they only appear while
        // scoring is mid-flight.
        if self.falling_bones.is_active() {
            frame.falling_bone_batch(self.falling_bones.placements());
        }
        frame.texts(modifier_strip_text);
        frame.quads(hud_quads);
        // Phase 3: bone yaku tablets sit just below the hand. Pushed before
        // hud_text so the 2D yaku name labels render on top of the wood.
        if !yaku_tablet_placements.is_empty() {
            frame.yaku_tablet_batch(yaku_tablet_placements);
        }
        // Phase 4: action row. Wood sort/play tablets + lacquered discard
        // bowl. The 2D button text labels in `hud_text` sit on top of the
        // wood, same hybrid pattern as the yaku tablets.
        if !wood_tablet_placements.is_empty() {
            frame.wood_tablet_batch(wood_tablet_placements);
        }
        if let Some(bowl) = discard_bowl_placement {
            frame.bowl(bowl);
        }
        // Phase 5: brass talisman/zodiac dish on the right side of the
        // table. Dish is sized to wrap the consumables strip; pendants are
        // pushed in slot order.
        if let Some((sx, sy, sw, sh)) = talisman_dish_strip {
            let dish_pad_x = sw * 0.10;
            let dish_pad_y = sh * 0.40;
            frame.dish_explicit(crate::render::draw_cmd::DishExplicit {
                center_pos: [sx + sw * 0.5, sy + sh * 0.5, 0.0],
                extents: [sw + dish_pad_x * 2.0, 14.0, sh + dish_pad_y * 2.0],
                pick_id: Some(PICK_CONSUMABLE_DISH),
            });
            if !talisman_dish_placements.is_empty() {
                frame.talisman_batch(talisman_dish_placements);
            }
        }

        // Phase 7: ambient table objects — physical coin pile (gold),
        // facedown wall stack (tiles remaining), and the dora indicator
        // stand. None of these are clickable; they're pure atmosphere that
        // makes the score line and wall counter physically present.
        //
        // Coin pile — sits to the left of the action row at the front of
        // the table. Coin count = min(gold, MAX_COIN_SLOTS) so the pile
        // visibly grows as the player accumulates gold but caps before
        // overflowing the slot pool.
        if run.gold > 0 {
            let coin_count = (run.gold as usize).min(48);
            let coin_radius = (4.0 * scale).max(3.0);
            let coin_thickness = (1.5 * scale).max(1.2);
            // Pile center is tucked closer to the player than the table
            // edge — between the left candle and the action row, where it
            // catches candlelight cleanly without crowding the wall stack.
            let pile_cx = layout.window_w * 0.16;
            let pile_cy = layout.window_h * 0.82;
            let cols: i32 = 6;
            let dx = coin_radius * 1.9;
            let dz = coin_radius * 1.9;
            let per_layer = (cols * cols) as usize;
            let mut coins: Vec<crate::render::draw_cmd::CoinPlacement> =
                Vec::with_capacity(coin_count);
            for i in 0..coin_count {
                let layer = (i / per_layer.max(1)) as i32;
                let in_layer = (i % per_layer.max(1)) as i32;
                let r = in_layer / cols;
                let c = in_layer % cols;
                let row_offset = if r % 2 == 0 { 0.0 } else { dx * 0.5 };
                let lx = (c as f32 - cols as f32 * 0.5) * dx + row_offset;
                let lz = (r as f32 - cols as f32 * 0.5) * dz;
                let world_y = coin_thickness * 0.5 + layer as f32 * coin_thickness;
                // Simple integer hash for per-coin yaw jitter.
                let hash = (i as u32).wrapping_mul(2654435761) ^ 0x9E3779B9;
                let rot_y = ((hash & 0xFFFF) as f32 / 65535.0 - 0.5) * std::f32::consts::TAU;
                coins.push(crate::render::draw_cmd::CoinPlacement {
                    world_pos: [pile_cx + lx, pile_cy + lz, world_y],
                    rotation_y: rot_y,
                    radius: coin_radius,
                    thickness: coin_thickness,
                    color: [1.00, 0.78, 0.30, 1.0],
                });
            }
            if !coins.is_empty() {
                frame.coin_batch(coins);
            }
            // Hover region for the coin pile → "Gold" glossary entry. The
            // pile is rendered as 3D coins projected via `pixel_to_world`,
            // so the screen footprint roughly tracks (pile_cx, pile_cy);
            // a generously-sized rect around that anchor catches the
            // cursor across the visible mound without needing the exact
            // projected hull.
            let pile_half_w = (cols as f32 * dx) * 0.6 + coin_radius * 2.0;
            let pile_half_h = (cols as f32 * dz) * 0.6 + coin_radius * 2.0;
            frame.glossary_anchor(
                [
                    pile_cx - pile_half_w,
                    pile_cy - pile_half_h,
                    pile_half_w * 2.0,
                    pile_half_h * 2.0,
                ],
                "Gold",
            );
        }

        // Facedown wall stack at the back-left of the table. Capped at the
        // renderer's MAX_WALL_TILE_SLOTS (80). Reuses the bone tablet mesh
        // (the renderer's WallStack dispatch instances bone tablets in a
        // grid based on `remaining` and `row_len`).
        let wall_remaining = run.wall.remaining() as u32;
        if wall_remaining > 0 {
            let tile_w = (12.0 * scale).max(8.0);
            let tile_h = (4.0 * scale).max(3.0);
            let tile_d = (16.0 * scale).max(11.0);
            let row_len: u32 = 17;
            let wall_x = layout.window_w * 0.02;
            let wall_y = layout.window_h * 0.18;
            frame.wall_stack(crate::render::draw_cmd::WallStackPlacement {
                world_pos: [wall_x, wall_y, 0.0],
                tile_extents: [tile_w, tile_h, tile_d],
                remaining: wall_remaining,
                row_len,
            });
        }

        // Dora indicator stand — small brass plinth at the back-right of
        // the table. Phase 7 just draws the plinth; the actual face-up
        // indicator tile mesh will be added when the tile pipeline gains
        // an "arbitrary world-space face tile" entrypoint.
        let dora_x = layout.window_w * 0.94;
        let dora_y = layout.window_h * 0.22;
        frame.dora_stand(crate::render::draw_cmd::DoraStandPlacement {
            world_pos: [dora_x, dora_y, 0.0],
            extents: [
                (20.0 * scale).max(14.0),
                (24.0 * scale).max(16.0),
                (14.0 * scale).max(10.0),
            ],
        });

        // Volumetric smoke pass. Pushed *after* every persistent 3D scene
        // object (plaques, ofuda, peg block, yaku/wood tablets, bowl,
        // talisman dish, coins, wall stack, dora stand) so all of them
        // land in pass A and the smoke draws over them — mirroring the
        // shop scene's order. Pushing this earlier (next to the candles)
        // dropped most of the table HUD into pass B, where it painted
        // right over the smoke and hid it. The 2D HUD text below, plus
        // `hand_tile_faces` and the hover/pause overlays, remain after
        // this marker so they stay readable on top of the haze.
        frame.fluid_smoke();

        frame.texts(hud_text);

        // Hand tile face labels — read on top of the persistent HUD
        // (score cartouche, button bar, zodiac strip) but under any
        // hover overlay so tooltips can still cover the tiles cleanly.
        frame.hand_tile_faces();

        // HOVER LAYER: tile hover outline + tooltip, yaku card hover
        // tooltip, zodiac slot hover tooltip, relic hover outline +
        // tooltip. Pushed *after* `hand_tile_faces` so they always sit
        // on top of the visible tile rank text, not under it.
        frame.quads(hover_quads);
        frame.texts(hover_text);

        // PAUSE OVERLAY: dim panel + buttons + text built earlier into
        // its own buffers. Sits above the hover layer so the pause menu
        // always visually wins.
        frame.quads(pause_quads);
        frame.texts(pause_text);

        // Non-cmd state — consumed by the renderer's 3D pass and the
        // main loop's input plumbing.
        frame.hand_tiles = run.hand.to_vec();
        frame.hand_slots = hand_slots;
        frame.focus = focus;
        frame.selected_tiles = run.selected.clone();
        frame.hint_indices = hint_indices;
        frame.departing_indices = self.pending_departures.clone();
        frame.point_lights = point_lights;
        frame.wind_gusts = wind_gusts;
        // Catch-all 3D-hit dispatcher: a full-screen `ButtonDef::scene`
        // pushed last so it only wins the first-hit search if no other
        // (smaller) button matched the cursor first. The matching click
        // routes through `update()` via `ctx.picked_gameplay_object`.
        //
        // Only push it when the cursor is *actually* over a 3D pickable
        // this frame. Otherwise the fullscreen rect intercepts clicks
        // meant for hand tiles (which aren't buttons — they're routed
        // through `pointer_slot` in `main.rs`'s `MouseInput` handler) and
        // silently drops them, since `picked_gameplay_object` is `None`
        // over a tile and the dispatch loop produces no action.
        //
        // Also suppressed while paused — the pause overlay already pushed
        // its own buttons + fullscreen blocker into `buttons` earlier in
        // this function, and the catch-all would otherwise intercept
        // clicks meant for the pause menu. Crucially we do *not* clear
        // `buttons` here: that would also wipe the pause-menu buttons we
        // just added, leaving the pause overlay completely unclickable.
        if !self.pause_menu.paused && ctx.picked_gameplay_object.is_some() {
            buttons.push(ButtonDef::scene(
                (0.0, 0.0, layout.window_w, layout.window_h),
                GAMEPLAY_3D_HIT_ID,
            ));
        }
        frame.buttons = buttons;
        frame.window_title = window_title;
        frame.debug_axes = self.debug_show_axes;

        // Opening smoke curtain overlay. The volumetric smoke alone can't
        // reliably hide the freshly-loaded game elements during the brief
        // post-transition window (the smoke pass composites depth-aware
        // and the HUD lives in front of it), so we lay a flat dark quad
        // over *everything* during the same window the fill impulses are
        // pumping. The overlay holds at near-opaque while the curtain
        // fills, then fades out in lockstep with the wind sweep so the
        // game and the dispersing smoke are revealed together.
        if self.initial_smoke_fill_active {
            if let Some(deal_at) = self.last_deal_at {
                let elapsed = now.saturating_duration_since(deal_at).as_secs_f32();
                let total = self.wind_delay_secs + self.wind_duration_secs;
                if elapsed < total {
                    let alpha = if elapsed < self.wind_delay_secs {
                        // Fill phase: ramp in over 0.1s, then hold near
                        // opaque so the game underneath is hidden.
                        (elapsed / 0.1).clamp(0.0, 1.0) * 0.96
                    } else {
                        // Wind phase: linear fade from full to 0 across
                        // the wind duration so the overlay disappears as
                        // the smoke is blown away.
                        let t = ((elapsed - self.wind_delay_secs)
                            / self.wind_duration_secs.max(0.001))
                        .clamp(0.0, 1.0);
                        (1.0 - t) * 0.96
                    };
                    if alpha > 0.0 {
                        // OBSIDIAN with the computed alpha — matches the
                        // indigo+gold theme so the curtain reads as the
                        // same dark space the rest of the HUD lives in,
                        // not as a neutral grey wash.
                        let base = crate::render::theme::color::OBSIDIAN;
                        frame.quad(GpuInstance {
                            rect: [0.0, 0.0, layout.window_w, layout.window_h],
                            color: [base[0], base[1], base[2], alpha],
                        });
                    }
                }
            }
        }

        // Cheap invariant check — catches future migration mistakes that
        // accidentally push two `HandTileFaces` markers (or zero of one)
        // into the cmds list, which would silently break tile-face z
        // order. Compiled out of release builds.
        debug_assert_marker_uniqueness(&frame);

        frame
    }
}

/// Debug-only invariant: a frame should contain at most one
/// `HandTileBackdrop` and at most one `HandTileFaces` marker. Two of
/// either silently break tile face z-ordering, and zero of either skips
/// the corresponding 3D pass — both bugs are easy to introduce when
/// editing a long `draw_frame` impl. Costs nothing in release.
#[inline]
fn debug_assert_marker_uniqueness(frame: &UiFrame) {
    if cfg!(debug_assertions) {
        let backdrops = frame
            .cmds
            .iter()
            .filter(|c| matches!(c, DrawCmd::HandTileBackdrop))
            .count();
        let faces = frame
            .cmds
            .iter()
            .filter(|c| matches!(c, DrawCmd::HandTileFaces))
            .count();
        debug_assert!(
            backdrops <= 1,
            "UiFrame contains {backdrops} HandTileBackdrop markers (expected ≤ 1)"
        );
        debug_assert!(
            faces <= 1,
            "UiFrame contains {faces} HandTileFaces markers (expected ≤ 1)"
        );
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
