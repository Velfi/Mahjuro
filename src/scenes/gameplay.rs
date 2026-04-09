//! Gameplay scene — the main tile-playing loop.

use std::time::Instant;

use crate::core::hand::suggest_completions;
use crate::core::scoring::StepKind;
use crate::core::yaku::yaku_preview;
use crate::game::cascade::ScoringCascade;
use crate::game::run::{STARTING_DISCARDS, STARTING_PLAYS};
use crate::render::animation::ENTITY_SCORE_PANEL;
use crate::render::candle_mesh::{CandlePlacement, WICK_TIP_Y};
use crate::render::draw_cmd::{CascadeTokenKind, DrawCmd, UiFrame};
use crate::render::falling_bones::FallingBoneSystem;
use crate::render::flying_coins::FlyingCoinSystem;
use crate::render::particles::ParticleSystem;
use crate::render::score_popups::ScorePopupSystem;
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
    /// Yaku Journal book — focusable so keyboard / controller players can
    /// reach the journal without a mouse. Has no `UiAction` analogue
    /// because toggling the overlay is a direct call into the scene's
    /// own `JournalOverlay` (see the `Confirm` handler in `update`).
    Journal,
}

/// Which counter peg row a `FocusTarget::Peg` refers to. The peg block on
/// the table holds two distinct groups: jade pegs counting plays remaining
/// (`Hands`) and amber pegs counting discards remaining (`Discards`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PegKind {
    Hands,
    Discards,
}

/// Every gameplay HUD element that the keyboard / controller / cursor can
/// "select". Spatial 2D navigation chooses the next target by nearest-in-
/// direction over the on-screen rect for each variant. Display-only variants
/// (relics, pegs, gold, yaku, dora) are still focusable so the player can
/// read tooltips for them via the keyboard, even though `Confirm` is a
/// no-op for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusTarget {
    HandTile(usize),
    Button(GameplayButton),
    Consumable(usize),
    Relic(usize),
    Peg(PegKind),
    Gold,
    /// One of the bone yaku-progress tablets in the row above the action
    /// bar. Index matches `visible_previews[i]`.
    YakuTablet(usize),
    /// The brass dora indicator stand at the back-right of the table.
    Dora,
}

use crate::ui::focus_nav::{FocusDir, focus_target_at_cursor, pick_neighbor, push_focus_ring};

const ALL_BUTTONS: [GameplayButton; 5] = [
    GameplayButton::SortSuit,
    GameplayButton::SortRank,
    GameplayButton::Discard,
    GameplayButton::Play,
    GameplayButton::Journal,
];

impl GameplayButton {
    /// Maps a focusable action button to its `UiAction`. Returns `None`
    /// for buttons whose activation is *not* expressible as a `UiAction`
    /// — currently only `Journal`, which toggles a scene-local overlay
    /// directly via `self.journal.toggle()` from the `Confirm` handler.
    fn ui_action(self) -> Option<UiAction> {
        Some(match self {
            GameplayButton::SortSuit => UiAction::SortBySuit,
            GameplayButton::SortRank => UiAction::SortByRank,
            GameplayButton::Play => UiAction::ScoreHand,
            GameplayButton::Discard => UiAction::CommitDiscard,
            GameplayButton::Journal => return None,
        })
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
    /// Animated coins that fly into/out of the dish when gold changes.
    flying_coins: FlyingCoinSystem,
    /// Gold value last frame — compared each update to detect changes.
    prev_gold: i32,
    /// Floating 3D extruded-glyph score popups ("+50", "×3", running total).
    /// Spawned at the source of each cascade step (relic rect or modifier
    /// strip centre) and drift toward the score panel before despawning.
    score_popups: ScorePopupSystem,
    /// When set, the gameplay scene draws a fullscreen gold-tinted quad
    /// fading out from this timestamp. Triggered on the cascade's
    /// `ShowTotal` edge so the screen flashes gold as the final beat lands.
    gold_flash_at: Option<Instant>,
    /// Timestamp of last frame for dt calculation.
    last_frame: Instant,
    /// Shared pause menu overlay.
    pause_menu: PauseMenu,
    /// Unified focus across every gameplay HUD element. Replaces the
    /// per-zone fields above as the source of truth for keyboard /
    /// controller / cursor "what is selected". `None` means nothing is
    /// focused (cursor over empty space, or focus explicitly cleared by
    /// `Cancel`).
    focus: Option<FocusTarget>,
    /// Focus rect graph captured at the end of the previous `draw_frame`,
    /// consumed by `update()` for cursor hit-tests and spatial navigation.
    /// One frame stale — same pattern as `projected_hand_rects`. Each
    /// entry is `(target, [x, y, w, h])` in window coordinates. Wrapped in
    /// a `RefCell` because `draw_frame` takes `&self` but needs to update
    /// this stash.
    last_focus_rects: std::cell::RefCell<Vec<(FocusTarget, [f32; 4])>>,
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
    /// Yaku Journal overlay — Balatro-style run-stats page listing every
    /// yaku with its level, leveled bonuses, and run play count. Opened
    /// by clicking the Journal book on the table.
    journal: super::journal::JournalOverlay,
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
    /// Lights-out transition: ramps from 0.0 (dark) to 1.0 (full brightness)
    /// after the opening deal lands. Multiplied into candle intensity, flame
    /// alpha, and point-light brightness so the scene fades in via the
    /// candles "sparking on" rather than a smoke curtain.
    light_ramp: f32,
    /// Debug-only: when true, the renderer overlays world-axes bars at the
    /// camera target so we can see which direction is +X / +Y / +Z while
    /// dialing in placements. Toggled from the native Debug menu.
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

/// How long after the opening deal before the candles begin sparking on.
const LIGHT_RAMP_DELAY_SECS: f32 = 0.3;
/// Duration over which candles ramp from dark to full brightness.
const LIGHT_RAMP_DURATION_SECS: f32 = 0.8;

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
            || self.flying_coins.is_active()
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
            flying_coins: FlyingCoinSystem::new(),
            prev_gold: 0,
            score_popups: ScorePopupSystem::new(),
            gold_flash_at: None,
            last_frame: Instant::now(),
            pause_menu: PauseMenu::new(),
            focus: None,
            last_focus_rects: std::cell::RefCell::new(Vec::new()),
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
            journal: super::journal::JournalOverlay::new(),
            prev_hand_len: 0,
            last_deal_at: None,
            wind_delay_secs: 3.0,
            wind_duration_secs: 1.4,
            debug_wind_at: None,
            initial_smoke_fill_active: true,
            light_ramp: 0.0,
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
        self.pause_menu.paused || self.glossary.open || self.journal.open || self.cascade.is_some()
    }

    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.particles.update(dt);
        self.falling_bones.update(dt);
        self.flying_coins.update(dt);
        self.score_popups.update(now);
        // Cursor position is captured every frame for cursor-mode hit-test
        // and tooltip placement. The legacy `cursor_moved` guard that used
        // to drop stale controller focus on mouse motion is gone — Phase A
        // of the unified focus model (further down) overwrites `self.focus`
        // from the cursor each frame in cursor mode, which handles the
        // same race (mouse click on tile while controller focus was on a
        // consumable) without the heuristic.
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

        // Detect gold changes and spawn flying coin animations.
        let cur_gold = ctx.run.gold;
        let delta = cur_gold - self.prev_gold;
        if delta != 0 && self.prev_gold != 0 {
            // Recompute the dish center from layout (mirrors draw_frame).
            let layout = ctx.layout;
            let coin_radius = layout.mm(11.3);
            let coin_thickness = layout.mm(3.5).max(2.0);
            let coin_candle_w = layout.mm(78.0) * 0.72;
            let coin_edge_pad = layout.mm(12.0);
            let coin_back_z_push = coin_candle_w;
            let scatter_half = coin_radius * 3.0;
            let dish_half_w = scatter_half + coin_radius * 2.0;
            let coin_sp = layout.score_panel;
            let right_candle_x = (coin_sp.x + coin_sp.w + coin_candle_w + coin_edge_pad)
                .min(layout.window_w - coin_candle_w * 0.5 - 4.0);
            let right_candle_y = coin_sp.y + coin_sp.h * 0.5 - coin_back_z_push;
            let pile_cx = right_candle_x + coin_candle_w * 0.5 + dish_half_w + coin_edge_pad;
            let pile_cy = right_candle_y;
            let dish_rim = (coin_thickness * 2.5).max(10.0);
            // Scale coin count with the magnitude of the change so bigger
            // payouts produce a more dramatic shower.
            let count = (delta.unsigned_abs() as usize).clamp(1, 12);
            if delta > 0 {
                self.flying_coins.gain(
                    pile_cx,
                    pile_cy,
                    dish_rim,
                    coin_radius,
                    coin_thickness,
                    count,
                );
            } else {
                self.flying_coins.lose(
                    pile_cx,
                    pile_cy,
                    dish_rim,
                    coin_radius,
                    coin_thickness,
                    count,
                );
            }
            ctx.bus
                .push(crate::game::event_bus::GameEvent::GoldChanged { delta });
        }
        self.prev_gold = cur_gold;

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
                    // Clear `last_deal_at` so the opening deal doesn't
                    // fire a second wind gust at the *normal* delay
                    // (3 s) after the shortened opening delay (1 s)
                    // already ran. Without this, `wind_delay_secs`
                    // reverts to 3.0 once the flag clears, and the
                    // check `elapsed >= 3.0` immediately passes for
                    // the same `last_deal_at` — producing a phantom
                    // second gust that blows away cursor smoke for
                    // several more seconds. The next post-discard
                    // refill will re-stamp `last_deal_at` and trigger
                    // a normal gust on its own schedule.
                    self.last_deal_at = None;
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

        // Light ramp: candles start dark and spark on after the opening deal.
        if self.light_ramp < 1.0 {
            if let Some(deal_at) = self.last_deal_at {
                let elapsed = now.saturating_duration_since(deal_at).as_secs_f32();
                if elapsed > LIGHT_RAMP_DELAY_SECS {
                    let t = ((elapsed - LIGHT_RAMP_DELAY_SECS) / LIGHT_RAMP_DURATION_SECS)
                        .clamp(0.0, 1.0);
                    // Ease-in curve so the candles spark slowly at first
                    // then bloom to full brightness.
                    self.light_ramp = t * t;
                }
            }
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
            self.glossary
                .handle_input(ctx.actions, ctx.button_clicks, ctx.scroll_lines);
            return None;
        }

        // Yaku Journal overlay (Patch UX): same modal pattern as the
        // glossary. Opened by clicking the Journal book on the table
        // (routed via the 3D-hit dispatcher below); while open the
        // overlay swallows all other input.
        if self.journal.open {
            self.journal
                .handle_input(ctx.actions, ctx.button_clicks, ctx.scroll_lines);
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

                        // Floating numeric popup: build a label from this
                        // step's delta against the previous step's running
                        // values so the player sees the *contribution* of
                        // each relic / yaku as a discrete object that flies
                        // toward the score panel and lands in the running
                        // total. The Final beat is intentionally skipped
                        // here — the score panel itself handles the closing
                        // crescendo via screen shake + gold flash.
                        let (prev_chips, prev_mult) = if idx > 0 {
                            let prev = &cascade.breakdown.steps[idx - 1];
                            (prev.running_chips, prev.running_mult)
                        } else {
                            (cascade.breakdown.base_chips, 1.0)
                        };
                        let (label, magnitude) = match step.kind {
                            StepKind::Chips => {
                                let d = step.running_chips - prev_chips;
                                if d == 0 {
                                    (String::new(), 0.0)
                                } else if d > 0 {
                                    (format!("+{}", d), d as f32)
                                } else {
                                    (format!("{}", d), d as f32)
                                }
                            }
                            StepKind::Mult => {
                                let d = step.running_mult - prev_mult;
                                if d.abs() < 1e-3 {
                                    (String::new(), 0.0)
                                } else if (d - d.round()).abs() < 0.05 {
                                    (format!("+{}x", d.round() as i64), d.abs() as f32 * 50.0)
                                } else {
                                    (format!("+{:.1}x", d), d.abs() as f32 * 50.0)
                                }
                            }
                            StepKind::Final => (String::new(), 0.0),
                        };
                        if !label.is_empty() {
                            let ms = ctx.layout.modifier_strip;
                            let source_x = match step.kind {
                                StepKind::Chips => ms.x + ms.w * 0.30,
                                StepKind::Mult => ms.x + ms.w * 0.70,
                                StepKind::Final => ms.x + ms.w * 0.50,
                            };
                            let source_y = ms.y + ms.h * 0.5;
                            let sp = ctx.layout.score_panel;
                            let dest = (sp.x + sp.w * 0.5, sp.y + sp.h * 0.5);
                            self.score_popups.spawn(
                                label,
                                (source_x, source_y),
                                dest,
                                step.kind,
                                magnitude,
                            );
                        }
                    }
                }
            }

            // Final-beat edge: fire once when the cascade transitions into
            // its ShowTotal phase. Used by the audio dispatcher to play the
            // closing sting on the final number landing, and now also drives
            // the screen shake + gold flash crescendo so big hands feel
            // *bigger* than small ones.
            if cascade.is_in_total() && !self.cascade_final_emitted {
                self.cascade_final_emitted = true;
                ctx.bus
                    .push(crate::game::event_bus::GameEvent::ScoreCascadeFinal);
                // Screen shake amplitude scales with the magnitude of this
                // hand — log curve so a 200-point hand still gets a tiny
                // kick and a 50,000-point hand really thumps.
                let earned = cascade.earned.max(1) as f32;
                let amp = (earned.log2() * 1.6).clamp(3.0, 18.0);
                ctx.anim
                    .shake(crate::render::animation::ENTITY_HAND_STRIP, amp, 350);
                ctx.anim.shake(ENTITY_SCORE_PANEL, amp * 0.7, 350);
                // Trigger the gold flash overlay; the draw() loop reads this
                // timestamp and renders a fading gold quad over the scene.
                self.gold_flash_at = Some(now);
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
                // Wipe the physical bones + score popups the moment scoring
                // ends so the play space clears for the next hand.
                self.falling_bones.clear();
                self.score_popups.clear();
            } else {
                // Allow skip on any key press during cascade.
                if !ctx.actions.is_empty() {
                    cascade.skip();
                    self.displayed_score = ctx.run.round_score;
                    self.cascade = None;
                    self.last_revealed_step = None;
                    self.cascade_final_emitted = false;
                    self.falling_bones.clear();
                    self.score_popups.clear();
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

        // ── Unified focus model ──────────────────────────────────────────
        //
        // `self.focus: Option<FocusTarget>` is the single source of truth
        // for "what is currently selected" across cursor, keyboard, and
        // controller. The focus rect graph it walks is built at the end
        // of the previous `draw_frame` and stored in `self.last_focus_rects`,
        // so directional navigation always uses the most recently rendered
        // on-screen positions (one frame stale, like every other projected
        // rect path in this codebase).
        //
        // Per-frame focus invariants — clamped at the top so a consumable
        // / relic / hand tile that vanished mid-frame doesn't leave a
        // dangling index pointing past the end of its collection.
        let consumable_capacity = ctx.run.consumables.capacity;
        let hand_len = ctx.run.hand.len();
        let relic_count = ctx.run.relics.len();
        if let Some(t) = self.focus {
            let still_valid = match t {
                FocusTarget::HandTile(i) => i < hand_len,
                FocusTarget::Consumable(i) => i < consumable_capacity,
                FocusTarget::Relic(i) => i < relic_count,
                // YakuTablet validity is checked against the projected
                // rect graph downstream — we leave it through here so a
                // newly-disabled yaku doesn't blank the focus mid-frame
                // before the next draw rebuilds the rect list.
                FocusTarget::Button(_)
                | FocusTarget::Peg(_)
                | FocusTarget::Gold
                | FocusTarget::YakuTablet(_)
                | FocusTarget::Dora => true,
            };
            if !still_valid {
                self.focus = None;
            }
        }

        // Phase A: cursor-mode sync. When the player is using the mouse,
        // hover IS focus — overwrite `self.focus` each frame from the
        // cursor's hit-test against the focus rect graph. Hand tiles get
        // the precise raycast path (`picked_hand_tile`); everything else
        // falls back to the projected 2D rect graph stored last frame.
        // This is what guarantees a mouse click on a hand tile while a
        // controller had focus on a consumable still selects the tile —
        // the cursor sync overwrites the stale focus before Confirm runs.
        let focus_rects = self.last_focus_rects.borrow().clone();
        if ctx.input_mode == crate::ui::input::InputMode::Cursor {
            let (cx, cy) = ctx.cursor_pos;
            let new_focus = if let Some(idx) = ctx.picked_hand_tile {
                Some(FocusTarget::HandTile(idx))
            } else {
                focus_target_at_cursor(&focus_rects, cx, cy)
            };
            self.focus = new_focus;
        }

        // Resolve the screen-space rect for the currently focused target
        // (if any) so the spatial picker has a starting position. The
        // target may have been added or removed since last frame, in
        // which case we'll fall back to the first hand tile in the graph
        // when seeding directional input.
        let current_focus_rect = self.focus.and_then(|t| {
            focus_rects
                .iter()
                .find_map(|(t2, r)| (*t2 == t).then_some(*r))
        });

        // Pre-collected list of Consumable targets in focus_rects, in
        // slot order. Used by the legacy `[` / `]` / LB / RB keymap below
        // to keep "shoulder buttons cycle through consumables" working as
        // a muscle-memory affordance even though spatial nav can also
        // reach them.
        let consumable_targets: Vec<FocusTarget> = focus_rects
            .iter()
            .filter_map(|(t, _)| match t {
                FocusTarget::Consumable(_) => Some(*t),
                _ => None,
            })
            .collect();

        // Process actions. Directional input → spatial picker. Confirm →
        // route by self.focus variant. Cancel → clear focus AND fall
        // through so existing `clear_selection` semantics still apply.
        // Everything else flows into `actions_for_scene` for the existing
        // gameplay action handlers below (ScoreHand, SortBySuit, etc.).
        let mut actions_for_scene: Vec<UiAction> = Vec::new();
        for &a in ctx.actions.iter() {
            // Map FocusNext/Prev → Right/Left for back-compat with the
            // existing keymap (Tab / arrows still cycle).
            let dir: Option<FocusDir> = match a {
                UiAction::FocusUp => Some(FocusDir::Up),
                UiAction::FocusDown => Some(FocusDir::Down),
                UiAction::FocusPrev => Some(FocusDir::Left),
                UiAction::FocusNext => Some(FocusDir::Right),
                _ => None,
            };
            if let Some(dir) = dir {
                // Seed focus on first directional press from None: prefer
                // the cursor's last hit, else the first hand tile in the
                // graph, else any first entry.
                let start_rect = current_focus_rect.or_else(|| {
                    focus_rects
                        .iter()
                        .find_map(|(t, r)| matches!(t, FocusTarget::HandTile(_)).then_some(*r))
                });
                if let Some(rect) = start_rect {
                    if let Some(next) = pick_neighbor(rect, dir, &focus_rects) {
                        self.focus = Some(next);
                    }
                } else if let Some((first, _)) = focus_rects.first() {
                    self.focus = Some(*first);
                }
                continue;
            }

            match a {
                // Legacy "shoulder buttons cycle consumables" affordance.
                // Steps through `Consumable` targets in order; wraps back
                // to `None` after the last so the player can exit the
                // strip without a separate keybind.
                UiAction::NavigateHudNext if !consumable_targets.is_empty() => {
                    let cur_idx = match self.focus {
                        Some(FocusTarget::Consumable(i)) => Some(i),
                        _ => None,
                    };
                    self.focus = match cur_idx {
                        None => Some(consumable_targets[0]),
                        Some(i) if i + 1 >= consumable_capacity => None,
                        Some(i) => Some(FocusTarget::Consumable(i + 1)),
                    };
                    continue;
                }
                UiAction::NavigateHudPrev if !consumable_targets.is_empty() => {
                    let cur_idx = match self.focus {
                        Some(FocusTarget::Consumable(i)) => Some(i),
                        _ => None,
                    };
                    self.focus = match cur_idx {
                        None => Some(FocusTarget::Consumable(consumable_capacity - 1)),
                        Some(0) => None,
                        Some(i) => Some(FocusTarget::Consumable(i - 1)),
                    };
                    continue;
                }
                // Confirm: route by focused target.
                //   HandTile → toggle selection
                //   Button   → enqueue the corresponding gameplay action
                //   Consumable → use it
                //   Relic / Peg / Gold → no-op (display-only; focus
                //     exists so the player can read the tooltip from the
                //     keyboard)
                //   None → eat the press (avoids falling through to
                //     apply_ui_actions which would otherwise toggle a
                //     hand tile based on the stale focus_tile_index).
                UiAction::Confirm => {
                    match self.focus {
                        Some(FocusTarget::HandTile(i)) => {
                            if !ctx.run.hand.is_empty() {
                                let idx = i.min(ctx.run.hand.len() - 1);
                                ctx.run.toggle_select(idx);
                            }
                        }
                        Some(FocusTarget::Button(GameplayButton::Journal)) => {
                            // Journal has no `UiAction`; toggle the
                            // overlay in-place. Mirrors the cursor-pick
                            // path that special-cases `WoodTablet(2)`
                            // earlier in `update`.
                            self.journal.toggle();
                        }
                        Some(FocusTarget::Button(b)) => {
                            if let Some(a) = b.ui_action() {
                                actions_for_scene.push(a);
                            }
                        }
                        Some(FocusTarget::Consumable(i)) => {
                            if let Some(result) = ctx.run.use_consumable(i) {
                                match result {
                                    crate::game::run::ConsumableUseResult::Zodiac {
                                        yaku,
                                        new_level,
                                    } => {
                                        log::info!(
                                            "Used Zodiac → {} now level {}",
                                            yaku.name(),
                                            new_level,
                                        );
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
                            // Clear focus so the next press doesn't double-
                            // fire on whatever consumable shifted into the
                            // freed slot.
                            self.focus = None;
                        }
                        Some(FocusTarget::Relic(_))
                        | Some(FocusTarget::Peg(_))
                        | Some(FocusTarget::Gold)
                        | Some(FocusTarget::YakuTablet(_))
                        | Some(FocusTarget::Dora)
                        | None => {}
                    }
                    continue;
                }
                // Cancel: clear focus AND let the existing
                // clear_selection path run via apply_ui_actions.
                UiAction::Cancel => {
                    self.focus = None;
                    actions_for_scene.push(a);
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
            // WoodTablet(2) is the Journal book — pushed at the end of
            // the action-row loop in `draw()` after the two sort tablets
            // (the third action-row slot is now the bronze mirror, not a
            // wood tablet). Click toggles the JournalOverlay; rest of
            // update() bails out next frame because `journal.open` will be
            // true.
            if matches!(
                ctx.picked_gameplay_object,
                Some(GameplayPick::WoodTablet(2))
            ) {
                self.journal.toggle();
                return None;
            }
            let action = match ctx.picked_gameplay_object {
                Some(GameplayPick::WoodTablet(0)) => Some(UiAction::SortBySuit),
                Some(GameplayPick::WoodTablet(1)) => Some(UiAction::SortByRank),
                Some(GameplayPick::BronzeMirror) => Some(UiAction::ScoreHand),
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

                    if pts > 0 {
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
                                // Particle count scales with the magnitude of
                                // the hand. Log curve so a tiny pair still gets
                                // a meaningful burst and a 50,000-point hand
                                // really erupts. `explode()` produces big
                                // chunky shards with a strong upward kick — a
                                // firework, not a polite puff. Layered
                                // two-colour bursts (warm gold core + smaller
                                // white-hot shower) give the explosion depth.
                                let mag = (pts as f32).max(1.0).log2();
                                let count = ((16.0 + mag * 8.0) as usize).clamp(16, 128);
                                self.particles
                                    .explode(px, py, count, [1.0, 0.86, 0.32, 1.0], 1.1);
                                self.particles.explode(
                                    px,
                                    py,
                                    count / 3,
                                    [1.0, 0.97, 0.85, 1.0],
                                    0.9,
                                );
                                // displayed_score will be driven by cascade
                            } else {
                                self.displayed_score = ctx.run.round_score;
                            }
                        } else {
                            self.displayed_score = ctx.run.round_score;
                        }
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
        // Hand-strip focus highlight is driven by the unified `self.focus`
        // model — only render the ring when focus is actually on a hand
        // tile. `usize::MAX` is the renderer's "no highlighted tile"
        // sentinel, so navigating focus onto a button / relic / peg
        // correctly removes the hand-strip ring.
        let focus = match self.focus {
            Some(FocusTarget::HandTile(i)) => i.min(run.hand.len().saturating_sub(1)),
            _ => usize::MAX,
        };
        let now = Instant::now();
        let glossary_open = self.glossary.open;
        let journal_open = self.journal.open;

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
            frame.background(super::BackgroundId::Black);
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

        // Journal overlay early-return — same shape as the glossary path.
        // Returns a fresh frame containing only the background and the
        // journal panel so the live HUD/3D scene doesn't bleed through.
        if journal_open {
            let mut frame = UiFrame::new();
            frame.background(super::BackgroundId::Black);
            self.journal
                .draw_into_frame(&mut frame, layout.window_w, layout.window_h, run);
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

        // Cascade chips/mult bone tokens — populated only while a cascade is
        // active. The numerical readout that used to sit on top of these
        // tokens (and the idle "Select tiles to play" / hand-preview line)
        // is now handled by the floating 3D `ScorePopupSystem`, so no 2D
        // labels are pushed for the modifier strip anymore.
        let mut cascade_token_placements: Vec<crate::render::draw_cmd::CascadeTokenPlacement> =
            Vec::new();
        if let Some(ref cascade) = self.cascade {
            let frame = cascade.frame(now);
            // Layout (within the modifier strip):
            //   top 36%  → reserved (formerly the source label)
            //   bot 64%  → [ chips token ]  [ mult token ]
            let src_h = ms.h * 0.36;
            let pill_y = ms.y + src_h;
            let pill_h = (ms.h - src_h - 2.0).max(8.0);
            let inner_w = ms.w * 0.80;
            let inner_x = ms.x + (ms.w - inner_w) * 0.5;
            let cross_w = pill_h * 0.7;
            let pill_w = ((inner_w - cross_w) * 0.5).max(20.0);

            // Pulse envelope: fast pop-in then settle. Active token grows ~12%.
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
            // envelope drives the renderer's per-instance scale-up so the
            // active axis pops on each scoring step.
            let chips_x = inner_x;
            {
                let cx = chips_x + pill_w * 0.5;
                let cy = pill_y + pill_h * 0.5;
                let pulse_t = ((chip_pulse - 1.0) / 0.12).clamp(0.0, 1.0);
                cascade_token_placements.push(crate::render::draw_cmd::CascadeTokenPlacement {
                    world_pos: [cx, cy, 4.0],
                    extents: [pill_w, (pill_h * 0.6).max(8.0), pill_h],
                    kind: crate::render::draw_cmd::CascadeTokenKind::Chips,
                    pulse: pulse_t,
                });
            }

            // Mult token — engraved bone, warm crimson tint.
            let mult_x = inner_x + pill_w + cross_w;
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
            }
        }

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
        // The yaku tablet panel keeps its old "4 buttons wide" container so
        // the active-yaku row stays roomy when 1-3 cards are visible. The
        // bottom action row now only holds the two sort tablets — the
        // discard bowl and bronze mirror have been promoted up to flank
        // the yaku panel on the table felt, where they read as the
        // physical "throw away" / "play" gestures rather than as buttons
        // jammed into the chrome strip.
        let container_w = btn_w * 4.0 + btn_gap * 3.0;
        let container_x = (layout.window_w - container_w) * 0.5;
        let btn_y = layout.window_h - btn_h - (12.0 * scale);

        // Bottom row: just the two sort tablets, centered on the screen.
        let sort_container_w = btn_w * 2.0 + btn_gap;
        let sort_container_x = (layout.window_w - sort_container_w) * 0.5;
        let suit_btn_rect = (sort_container_x, btn_y, btn_w, btn_h);
        let rank_btn_rect = (sort_container_x + btn_w + btn_gap, btn_y, btn_w, btn_h);

        // Bowl + mirror are positioned in the *yaku row* — at the same
        // vertical level as the bone yaku tablets — flanking the yaku
        // panel on the left and right. Their click rects (`discard_btn_rect`
        // / `play_btn_rect`) are sized to match the bowl/mirror diameter
        // so the focus rect graph + spatial nav land on the visible
        // object instead of an invisible button-shaped slot.
        //
        // The yaku panel geometry below is *recomputed* in the
        // `visible_previews.is_empty()` branch — we duplicate just enough
        // here (panel_h + panel_y + a panel_gap) to keep the bowl/mirror
        // anchored to the same vertical band even when the yaku panel is
        // empty (so they don't snap up/down as the player selects tiles
        // and active yaku appear / disappear).
        let yaku_panel_h = (66.0 * scale).max(48.0);
        let yaku_panel_gap = 8.0 * scale;
        let yaku_panel_y = btn_y - yaku_panel_h - yaku_panel_gap;
        let bowl_diam = (yaku_panel_h * 2.4).min(layout.window_h * 0.18);
        // Inset the bowl + mirror *inside* the yaku panel's horizontal
        // span so they overlap the panel edges instead of floating off in
        // the table corners. Combined with the forward push below this
        // reads as the bowl/mirror sitting in front of the yaku tablets
        // rather than alongside them.
        let bowl_inset = bowl_diam * 0.30;
        // Push the bowl/mirror toward the camera by sliding them down in
        // pixel-y. In `pixel_to_world` higher pixel_y = closer to the
        // camera (the front of the table), so adding a fraction of the
        // yaku panel height bumps them downstage of the yaku row.
        let bowl_forward_push = yaku_panel_h * 0.55;
        let bowl_cy = yaku_panel_y + yaku_panel_h * 0.5 + bowl_forward_push;
        let bowl_cx = container_x + bowl_inset - bowl_diam * 0.5;
        let mirror_cx = container_x + container_w - bowl_inset + bowl_diam * 0.5;
        let mirror_cy = bowl_cy;
        // Synthesize square click rects centered on the bowl / mirror.
        // Downstream the placement loop reads `bw, bh` from these rects to
        // size + center the meshes — square rects keep `bw.min(bh * 2.4)`
        // collapsing to the bowl diameter.
        let discard_btn_rect = (
            bowl_cx - bowl_diam * 0.5,
            bowl_cy - bowl_diam * 0.5,
            bowl_diam,
            bowl_diam,
        );
        let play_btn_rect = (
            mirror_cx - bowl_diam * 0.5,
            mirror_cy - bowl_diam * 0.5,
            bowl_diam,
            bowl_diam,
        );

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

        // Focus rect graph: every focusable HUD element pushes its
        // screen-space rect here as it's laid out below. Stashed in
        // `self.last_focus_rects` at the end of `draw_frame` so the next
        // frame's `update()` can hit-test the cursor and run spatial
        // navigation against the freshest on-screen positions.
        let mut focus_rect_graph: Vec<(FocusTarget, [f32; 4])> = Vec::new();

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
        let mut hovered_chicken: Option<(f32, f32)> = None;

        // Phase 9: with the Yaku Journal taking over the "browse all
        // yaku" job, the in-play tablet row collapses to *only firing
        // yaku*. Players who want to study levels, bonuses, or
        // construction hints open the Journal book on the table; the
        // play area is reserved for "what just fired this turn".
        let visible_previews: Vec<&crate::core::yaku::YakuPreview> =
            previews.iter().filter(|p| p.active).collect();

        // If the selection is a valid hand but triggers no yaku, show a
        // chicken-hand tablet so the player knows the hand is legal.
        let is_chicken_hand = visible_previews.is_empty()
            && crate::core::hand::validate_selection(&selected_tiles_for_yaku).is_some();

        // Phase 3: yaku selectors are now physical bone tablets sitting in
        // a row in front of the hand. The flat slate-blue card quads + the
        // progress-fill bar are gone — replaced by `YakuTabletBatch` that
        // the renderer dispatches through the lit-mesh pipeline. The 2D
        // text labels stay as a screen-space overlay until the engraved
        // decal pass lands; hover tracking still uses the original screen
        // rect (the cards live in the same pixel region as before).
        let mut yaku_tablet_placements: Vec<crate::render::draw_cmd::YakuTabletPlacement> =
            Vec::new();
        if !visible_previews.is_empty() || is_chicken_hand {
            let panel_h = (66.0 * scale).max(48.0);
            let panel_gap = 8.0 * scale;
            let panel_y = btn_y - panel_h - panel_gap;
            let panel_w = container_w;
            let panel_x = container_x;
            let tablet_count = if is_chicken_hand {
                1
            } else {
                visible_previews.len()
            };
            let n = tablet_count as f32;
            let card_gap = 6.0 * scale;
            // Cap individual card width to what a full loadout (3 yaku) would
            // get — otherwise a lone active yaku stretches across the entire
            // container, which reads as a UI bug. Single/few yaku stay
            // left-aligned at their natural size.
            let natural_card_w = (panel_w - card_gap * 2.0) / 3.0;
            let card_w = ((panel_w - card_gap * (n - 1.0)) / n).min(natural_card_w);
            // Tablets are flat-on-table dominoes: extents[0] is width
            // (matches card width), extents[1] is the thickness above the
            // wood, extents[2] is depth (matches card height into the
            // scene).
            let tablet_thickness = (8.0 * scale).max(6.0);
            if is_chicken_hand {
                // Chicken hand: valid meld but no yaku fired. Show a single
                // tablet with a chicken emoji so the player knows the hand
                // is legal (just worth very little).
                let center_px = panel_x + card_w * 0.5;
                let center_py = panel_y + panel_h * 0.5;
                let hovered_now = matches!(
                    ctx.picked_gameplay_object,
                    Some(crate::render::wgpu_renderer::GameplayPick::YakuTablet(0))
                );
                yaku_tablet_placements.push(crate::render::draw_cmd::YakuTabletPlacement {
                    world_pos: [center_px, center_py, 0.0],
                    extents: [card_w, tablet_thickness, panel_h],
                    name: "\u{1F414} Chicken Hand".to_string(),
                    progress: 0.0,
                    active: true,
                    hover: if hovered_now { 1.0 } else { 0.0 },
                });
                if hovered_now {
                    let (ax, ay) = match ctx.projected_yaku_tablet_rects.first().copied() {
                        Some([px, py, pw, _ph]) if pw > 0.0 && px.is_finite() && py.is_finite() => {
                            (px + pw * 0.5, py)
                        }
                        _ => (center_px, panel_y),
                    };
                    hovered_chicken = Some((ax, ay));
                }
            } else {
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
                            Some([px, py, pw, _ph])
                                if pw > 0.0 && px.is_finite() && py.is_finite() =>
                            {
                                (px + pw * 0.5, py)
                            }
                            _ => (center_px, cy),
                        };
                        hovered_yaku = Some((p.kind, ax, ay));
                    }
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
        if let Some((ax, ay)) = hovered_chicken {
            push_tooltip(
                &mut hover_quads,
                &mut hover_text,
                ax,
                ay,
                layout.window_w,
                layout.window_h,
                "\u{1F414} Chicken Hand",
                "A valid hand with no yaku. Scores base chips \u{00D7} 1 mult. \
                 Build toward a yaku to multiply your score.",
            );
        }

        let paused = self.pause_menu.paused;
        let btn_rects = [
            suit_btn_rect,
            rank_btn_rect,
            discard_btn_rect,
            play_btn_rect,
        ];
        // Phase 4: action row is now physical objects.
        //   - Sort by Suit / Sort by Rank → carved wood tablets
        //   - Discard                     → lacquered wood bowl (left)
        //   - Play Hand                   → polished bronze mirror (right)
        // The flat slate-blue button background quads are gone; only the
        // focus-highlight border remains as a 2D affordance for keyboard
        // navigation.
        let mut wood_tablet_placements: Vec<crate::render::draw_cmd::WoodTabletPlacement> =
            Vec::new();
        let mut discard_bowl_placement: Option<crate::render::draw_cmd::BowlPlacement> = None;
        let mut bronze_mirror_placement: Option<crate::render::draw_cmd::MirrorPlacement> = None;
        let play_enabled = selection_valid && run.plays_remaining > 0;
        let discard_enabled = selected_count > 0 && run.discards_remaining > 0;
        for (i, &(bx, by, bw, bh)) in btn_rects.iter().enumerate() {
            // Register this button in the focus rect graph so the unified
            // focus model can navigate to it spatially.
            // Anchor the focus hit-test on the renderer's projected mesh
            // rect for this button — the layout pixel rect doesn't
            // account for camera tilt + perspective so it falls out of
            // sync with where the player actually sees the object. On
            // the very first frame the projected rect may be missing
            // (renderer hasn't drawn yet); skip the entry then, the
            // focus picker tolerates absent targets and the next frame
            // repopulates.
            let proj = match ALL_BUTTONS[i] {
                GameplayButton::SortSuit => ctx.projected_wood_tablet_rects.get(0).copied(),
                GameplayButton::SortRank => ctx.projected_wood_tablet_rects.get(1).copied(),
                GameplayButton::Discard => ctx.projected_bowl_rect,
                GameplayButton::Play => ctx.projected_mirror_rect,
                // Pushed separately alongside the journal placement
                // block further down — its slot index in the wood
                // tablet rect vec isn't known until then.
                GameplayButton::Journal => None,
            };
            if let Some(rect) = proj {
                focus_rect_graph.push((FocusTarget::Button(ALL_BUTTONS[i]), rect));
            }
            // The hover state for the 3D action objects comes from the
            // renderer's raycast picker against precomputed local AABBs —
            // no projected screen rects, no per-frame intersection of
            // input pixel rects with camera-distorted AABBs. The picker
            // is one frame stale, like every other 3D pick path.
            let pick = ctx.picked_gameplay_object;
            // The bowl + mirror animate their tilt envelope from the
            // *hover* flag below, so we want it to also light up when the
            // unified focus model lands on them via keyboard nav — not
            // just on cursor hover. Hand tablets keep their existing
            // pick-only behavior since they have no tilt animation.
            let focused_btn = match self.focus {
                Some(FocusTarget::Button(b)) => Some(b),
                _ => None,
            };
            let hovered = match i {
                0 => matches!(
                    pick,
                    Some(crate::render::wgpu_renderer::GameplayPick::WoodTablet(0)),
                ),
                1 => matches!(
                    pick,
                    Some(crate::render::wgpu_renderer::GameplayPick::WoodTablet(1)),
                ),
                2 => {
                    matches!(
                        pick,
                        Some(crate::render::wgpu_renderer::GameplayPick::DiscardBowl),
                    ) || focused_btn == Some(GameplayButton::Discard)
                }
                3 => {
                    matches!(
                        pick,
                        Some(crate::render::wgpu_renderer::GameplayPick::BronzeMirror),
                    ) || focused_btn == Some(GameplayButton::Play)
                }
                _ => false,
            };
            // The per-button focus highlight is gone — the unified focus
            // model draws a single brass ring around the focused target
            // at the end of `draw_frame` via `push_focus_ring`.
            let center_px = bx + bw * 0.5;
            let center_py = by + bh * 0.5;
            let tablet_thickness = (bh * 0.35).max(8.0);
            match i {
                0 | 1 => {
                    let label = match i {
                        0 => "Sort by Suit",
                        _ => "Sort by Rank",
                    };
                    let _tablet_idx = wood_tablet_placements.len();
                    wood_tablet_placements.push(crate::render::draw_cmd::WoodTabletPlacement {
                        world_pos: [center_px, center_py, 0.0],
                        extents: [bw, tablet_thickness, bh],
                        label: label.to_string(),
                        pressed: 0.0,
                        hover: if hovered { 1.0 } else { 0.0 },
                        disabled: false,
                    });
                    // Gold overlay label superimposed on the sort tablet
                    // when it's the active selection. Anchored on the
                    // The label is engraved directly on the wood tablet
                    // via a per-instance decal texture — no 2D overlay.
                }
                2 => {
                    // Discard bowl — flanks the yaku tablet panel on the
                    // far left of the table felt. The synthesized
                    // `discard_btn_rect` above is already a square sized to
                    // the desired bowl diameter and centered at the
                    // (bowl_cx, bowl_cy) anchor, so we just read the rect
                    // back here without re-applying any nudges.
                    //
                    // The `hover` field is a binary 0/1 *target*, not a
                    // pre-eased value: the renderer keeps a per-bowl
                    // `bowl_hover_anim` envelope that exponentially eases
                    // toward this target each frame, driving both the
                    // existing lift and the tilt-toward-camera rotation
                    // applied to the bowl's model matrix in
                    // `wgpu_renderer.rs`. Tilt direction reverses
                    // automatically when the target flips back to 0.
                    let target = if hovered && discard_enabled { 1.0 } else { 0.0 };
                    let diam = bw.min(bh);
                    discard_bowl_placement = Some(crate::render::draw_cmd::BowlPlacement {
                        world_pos: [center_px, center_py, 0.0],
                        extents: [diam, diam, diam],
                        hover: target,
                    });
                    // Gold "Discard tiles" label superimposed on the river
                    // when it's the active selection (cursor hover or
                    // keyboard focus). Anchored purely on the renderer's
                    // projected mesh rect — no layout-rect fallback, so
                    // on the very first frame after a scene transition
                    // the label briefly doesn't appear.
                    if let Some(r) = ctx.projected_bowl_rect.filter(|_| hovered) {
                        let label_h = (r[3] * 0.38).max(28.0);
                        hover_text.push(TextLabel {
                            rect: [r[0], r[1] + r[3] * 0.5 - label_h * 0.5, r[2], label_h],
                            text: "Discard tiles".to_string(),
                            color: [1.0, 0.84, 0.40, 1.0],
                            align: TextAlign::Center,
                            no_glossary: true,
                            ..Default::default()
                        });
                    }
                }
                3 => {
                    // Bronze mirror — flanks the yaku tablet panel on the
                    // far right, mirror image of the bowl on the left.
                    // Same square `play_btn_rect` convention as the bowl,
                    // and the same binary-target → renderer-eased envelope
                    // pattern. The renderer's `mirror_hover_anim` field
                    // handles the smoothed tilt + reverse-on-unhover.
                    let target = if hovered && play_enabled { 1.0 } else { 0.0 };
                    let diam = bw.min(bh);
                    bronze_mirror_placement = Some(crate::render::draw_cmd::MirrorPlacement {
                        world_pos: [center_px, center_py, 0.0],
                        extents: [diam, diam, diam],
                        hover: target,
                    });
                    // Gold "Play hand" label superimposed on the mirror
                    // when it's the active selection. Same projected-mesh
                    // anchoring as the river label above (no layout-rect
                    // fallback).
                    if let Some(r) = ctx.projected_mirror_rect.filter(|_| hovered) {
                        let label_h = (r[3] * 0.38).max(28.0);
                        hover_text.push(TextLabel {
                            rect: [r[0], r[1] + r[3] * 0.5 - label_h * 0.5, r[2], label_h],
                            text: "Play hand".to_string(),
                            color: [1.0, 0.84, 0.40, 1.0],
                            align: TextAlign::Center,
                            no_glossary: true,
                            ..Default::default()
                        });
                    }
                }
                _ => {}
            }
        }

        // Yaku Journal book — an additional wood-tablet placement reusing
        // the existing wood-tablet pipeline + pick path so we don't have
        // to plumb a new mesh through the renderer just for the book.
        // Sits in the *bottom action row* to the right of the two sort
        // tablets (the bowl + mirror moved up to flank the yaku panel,
        // leaving room in the bottom row for the journal). Clicking it
        // toggles the JournalOverlay — the click is dispatched in
        // `update()` via `GameplayPick::WoodTablet(2)`, because after
        // the layout swap only the two sort tablets occupy wood-tablet
        // slots 0 and 1 and the journal lands in slot 2.
        let journal_pick_idx = wood_tablet_placements.len();
        // The journal "lights up" on either cursor pick or keyboard focus,
        // matching how the other action buttons treat hover. The
        // `focused_btn` check piggybacks on the unified focus model added
        // for keyboard / controller nav — see `GameplayButton::Journal`.
        let journal_focused = matches!(
            self.focus,
            Some(FocusTarget::Button(GameplayButton::Journal))
        );
        let journal_hovered = journal_focused
            || matches!(
                ctx.picked_gameplay_object,
                Some(crate::render::wgpu_renderer::GameplayPick::WoodTablet(idx)) if idx == journal_pick_idx,
            );
        let (rbx, rby, rbw, rbh) = rank_btn_rect;
        let book_w = rbw * 0.55;
        let book_h = rbh * 0.95;
        // Anchor the book to the right of the rightmost sort tablet with
        // a generous gap so it reads as a separate object on the felt
        // rather than a third button in the row. Stays inside the window
        // even at narrow widths via a clamp against the right edge.
        let book_cy = rby + rbh * 0.5;
        let desired_cx = rbx + rbw + (32.0 * scale).max(16.0) + book_w * 0.5;
        let max_cx = layout.window_w - book_w * 0.5 - (12.0 * scale);
        let book_cx = desired_cx.min(max_cx);
        let book_thickness = (book_h * 0.45).max(8.0);
        wood_tablet_placements.push(crate::render::draw_cmd::WoodTabletPlacement {
            world_pos: [book_cx, book_cy, 0.0],
            extents: [book_w, book_thickness, book_h],
            label: "Journal".to_string(),
            pressed: 0.0,
            hover: if journal_hovered { 1.0 } else { 0.0 },
            disabled: false,
        });
        // Anchor the Journal button's keyboard-nav focus rect on the
        // renderer's projected wood-tablet rect for the journal slot.
        // Same one-frame stale snapshot pattern as the other action
        // buttons; first-frame absence is harmless.
        if let Some(&rect) = ctx.projected_wood_tablet_rects.get(journal_pick_idx) {
            focus_rect_graph.push((FocusTarget::Button(GameplayButton::Journal), rect));
        }
        // The Journal label is engraved directly on the wood tablet
        // via a per-instance decal texture — no 2D overlay.

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

        // Coin pile screen rect — kept in sync with the actual 3D pile
        // draw down at the bottom of `draw_frame` (search "Coin pile —"
        // around the dish_explicit call). We compute it up here so the
        // `FocusTarget::Gold` focus rect, the gold tooltip anchor, *and*
        // the pile draw all use exactly the same screen-space footprint
        // — keeping the focus ring, hover tooltip, and physical pile
        // visually locked together as the score panel resizes. `None`
        // when there's no gold to display (no pile is drawn).
        // Dora indicator screen rect. Mirrored against the actual draw
        // call further down in `draw_frame` (search "Dora indicator
        // stand —"). Pre-computed up here so the focus rect graph entry
        // and the focus tooltip can both use it. Note: the dora stand is
        // a 3D mesh, but we're not projecting its AABB through the
        // camera — we just use its 2D pixel anchor directly because
        // gameplay's `pixel_to_world` projects the X/Y inputs as on-
        // screen pixel coordinates, which keeps the rect aligned with
        // the visible plinth.
        let dora_rect: [f32; 4] = {
            let dora_x = layout.window_w * 0.94;
            let dora_y = layout.window_h * 0.22;
            let dora_w = layout.mm(30.0);
            let dora_h = layout.mm(40.0);
            [dora_x - dora_w * 0.5, dora_y - dora_h * 0.5, dora_w, dora_h]
        };

        let coin_pile_rect: Option<[f32; 4]> = if run.gold > 0 {
            let coin_radius = layout.mm(11.3);
            let coin_candle_w = layout.mm(78.0) * 0.72;
            let coin_edge_pad = layout.mm(12.0);
            let coin_back_z_push = coin_candle_w;
            let right_candle_x = (sp.x + sp.w + coin_candle_w + coin_edge_pad)
                .min(layout.window_w - coin_candle_w * 0.5 - 4.0);
            let right_candle_y = sp.y + sp.h * 0.5 - coin_back_z_push;
            let scatter_half = coin_radius * 3.0;
            let dish_half_w = scatter_half + coin_radius * 2.0;
            let pile_cx = right_candle_x + coin_candle_w * 0.5 + dish_half_w + coin_edge_pad;
            let pile_cy = right_candle_y;
            let pile_half_w = scatter_half + coin_radius * 2.0;
            let pile_half_h = scatter_half + coin_radius * 2.0;
            Some([
                pile_cx - pile_half_w,
                pile_cy - pile_half_h,
                pile_half_w * 2.0,
                pile_half_h * 2.0,
            ])
        } else {
            None
        };
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
        let mut ribbon_dish_placements: Vec<crate::render::draw_cmd::ZodiacRibbonPlacement> =
            Vec::new();
        let mut talisman_dish_strip: Option<(f32, f32, f32, f32)> = None;
        if consumables.capacity > 0 {
            let zscale = (layout.window_w.min(layout.window_h)) / 600.0;
            let slot_w = (140.0 * zscale).max(120.0);
            let slot_h = (56.0 * zscale).max(48.0);
            let gap = (6.0 * zscale).max(3.0);
            let total_w =
                slot_w * consumables.capacity as f32 + gap * (consumables.capacity as f32 - 1.0);
            let strip_x = layout.window_w - total_w - (48.0 * zscale);
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
                // Shrink the focus rect to match the visual item extents
                // so the hover/click region hugs the pendant, not the
                // full inventory slot rectangle.
                let (fx, fy, fw, fh) = if let Some(&item) = consumables.items.get(slot_idx) {
                    match item {
                        crate::core::consumable::Consumable::Talisman(_) => {
                            // Talisman visual is 55% × 85% of the slot.
                            // Bias upward — the pendant sits on top of the
                            // dish, in the upper portion of the slot rect.
                            let fw = slot_sw * 0.55;
                            let fh = slot_sh * 0.85;
                            (
                                slot_sx + (slot_sw - fw) * 0.5,
                                slot_sy + (slot_sh - fh) * 0.20,
                                fw,
                                fh,
                            )
                        }
                        crate::core::consumable::Consumable::Zodiac(_) => {
                            // Ribbon is narrow — use ~30% width, 85% height.
                            // Same upward bias as talismans.
                            let fw = slot_sw * 0.30;
                            let fh = slot_sh * 0.85;
                            (
                                slot_sx + (slot_sw - fw) * 0.5,
                                slot_sy + (slot_sh - fh) * 0.20,
                                fw,
                                fh,
                            )
                        }
                    }
                } else {
                    // Empty slots keep the full rect for easy clicking.
                    (slot_sx, slot_sy, slot_sw, slot_sh)
                };
                focus_rect_graph.push((FocusTarget::Consumable(slot_idx), [fx, fy, fw, fh]));
                if let Some(&item) = consumables.items.get(slot_idx) {
                    // Physical pendant on the dish — color encodes the
                    // consumable type. Zodiacs read jade-green, talismans
                    // pick up the talisman's enhancement family color.
                    let pendant_color = match item {
                        crate::core::consumable::Consumable::Zodiac(_) => [0.45, 0.78, 0.55, 1.0],
                        crate::core::consumable::Consumable::Talisman(_) => [0.92, 0.78, 0.32, 1.0],
                    };
                    // Lift the pendant above the dish. The dish rim
                    // height is `layout.mm(10.0)` world units (scales
                    // with window size), so we park the pendant center
                    // a few units above that so it rests visibly on top.
                    let dish_h = layout.mm(10.0);
                    let pendant_y = dish_h + 8.0;
                    match item {
                        crate::core::consumable::Consumable::Zodiac(z) => {
                            // Ribbon thickness = width × 0.15 (set by the
                            // renderer); bump width to mm(12) so the silk
                            // reads as ~1.8mm thick at table scale.
                            let ribbon_w = layout.mm(12.0);
                            ribbon_dish_placements.push(
                                crate::render::draw_cmd::ZodiacRibbonPlacement {
                                    anchor_pos: [zx + slot_w * 0.5, zy, pendant_y],
                                    length: slot_h * 0.85,
                                    width: ribbon_w,
                                    rotation_y_deg: 0.0,
                                    rotation_x_deg: -90.0,
                                    rotation_z_deg: 0.0,
                                    color: [1.0, 1.0, 1.0, 1.0],
                                    kind: Some(z),
                                },
                            );
                        }
                        crate::core::consumable::Consumable::Talisman(tk) => {
                            talisman_dish_placements.push(
                                crate::render::draw_cmd::TalismanPlacement {
                                    center_pos: [zx + slot_w * 0.5, zy + slot_h * 0.5, pendant_y],
                                    extents: [slot_w * 0.55, slot_h * 0.85, 14.0],
                                    rotation_y_deg: 0.0,
                                    // Lay flat on the dish (face up). -90
                                    // around X rotates the tablet's front
                                    // normal from +Z to +Y.
                                    rotation_x_deg: -90.0,
                                    rotation_z_deg: 0.0,
                                    color: pendant_color,
                                    kind: tk,
                                },
                            );
                        }
                    }
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
                            format!("Click or press to use. {}", t.description()),
                        ),
                    };
                    if !paused {
                        buttons.push(ButtonDef::scene(
                            (fx, fy, fw, fh),
                            ZODIAC_USE_BASE + slot_idx as u32,
                        ));
                    }
                    // Tooltip is now driven by `self.focus`. In cursor
                    // mode the Phase A sync in `update()` writes
                    // `FocusTarget::Consumable(i)` whenever the cursor is
                    // over a slot rect; in keyboard / controller mode the
                    // player navigates here with spatial nav.
                    if self.focus == Some(FocusTarget::Consumable(slot_idx)) {
                        push_tooltip(
                            &mut hover_quads,
                            &mut hover_text,
                            fx + fw * 0.5,
                            fy,
                            layout.window_w,
                            layout.window_h,
                            &tooltip_title,
                            &tooltip_body,
                        );
                    }
                } else if self.focus == Some(FocusTarget::Consumable(slot_idx)) {
                    push_tooltip(
                        &mut hover_quads,
                        &mut hover_text,
                        fx + fw * 0.5,
                        fy,
                        layout.window_w,
                        layout.window_h,
                        "Consumable Slot",
                        "Empty. Earn Zodiac cards from blind clears or buy Zodiacs and Talismans in the shop. Both share these slots.",
                    );
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
            // The tile tooltip now follows the unified focus model: it
            // shows whenever `self.focus` points at a hand tile. In
            // cursor mode the Phase A cursor sync in `update()` writes
            // `FocusTarget::HandTile(i)` whenever the cursor's raycast
            // pick lands on a tile, so this naturally collapses hover and
            // controller-focus into one path.
            let hovered_idx: Option<usize> = match self.focus {
                Some(FocusTarget::HandTile(i)) if !run.hand.is_empty() => {
                    Some(i.min(run.hand.len() - 1))
                }
                _ => None,
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
        // `scale_c` is still used by per-candle jitter offsets below; the
        // jitters are positional fudges, not physical mesh sizes, so they
        // stay in pixel space.
        let scale_c = (layout.window_w / 600.0).max(0.5);
        // The wax mesh is now ~0.555 tall and ~0.36 wide in local units
        // (votive proportions), so `candle_scale` (= the uniform mesh
        // scale) is the height of the votive in world units. Sized in
        // real-world millimeters via `layout.mm()` so the candle stays
        // at true votive proportions next to the mahjong tiles.
        let candle_h = layout.mm(78.0); // ~78mm tall votive
        let candle_w = candle_h * 0.72; // votive footprint, used for layout padding
        let flame_w = layout.mm(16.0);
        let flame_h = layout.mm(27.0);
        let radius_px = layout.mm(244.0); // candlelight pool radius
        let edge_pad = layout.mm(12.0);
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
        // The two score-panel candles sit at the back of the table. Push
        // them one candle-width further from the camera (smaller pixel-y →
        // greater table-z) so they read as the rear pair in depth.
        let back_z_push = candle_w;
        let candle_centers: [(f32, f32); 5] = [
            // Score panel left
            (
                (sp.x - candle_w - edge_pad).max(candle_w * 0.5 + 4.0),
                sp.y + sp.h * 0.5 - back_z_push,
            ),
            // Score panel right
            (
                (sp.x + sp.w + candle_w + edge_pad).min(layout.window_w - candle_w * 0.5 - 4.0),
                sp.y + sp.h * 0.5 - back_z_push,
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
        // Cheap deterministic hash → 4 pseudorandom values in [-1, 1].
        // Used to jitter candle position, scale, and height per index so the
        // four votives don't snap to a perfectly symmetric grid.
        fn candle_jitter(seed: u32) -> (f32, f32, f32, f32) {
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
            (h(0x68E31DA4), h(0xB5297A4D), h(0x1B56C4E9), h(0xA3D70F8C))
        }

        let candle_scale_base = candle_h;
        // Debug visibility: when the candle row is hidden, skip the entire
        // push loop. We can't filter `CandleBatch` / `Flame` post-hoc because
        // each candle also pushes a `PointLight` and the table shader would
        // keep getting lit by invisible flames.
        let hide_candles = ctx.debug_visibility.hide_candles;
        if !hide_candles {
            for (i, &(cx, cy_anchor)) in candle_centers.iter().enumerate() {
                let candle = self.candles[i];

                // Per-candle jitter. Bottom (front) candles get directionally
                // constrained offsets so they never drift forward into the
                // tile sightline; top candles can wander freely.
                let (jx, jy, js, jh) = candle_jitter(i as u32);
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
                // ±15% height variation so they aren't all the same tallness.
                let height_scale = 1.0 + jh * 0.15;

                candle_placements.push(CandlePlacement {
                    world_pos: [cx_j, cy_j, 0.0],
                    scale: candle_scale,
                    flicker: candle.flicker,
                    height_scale,
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
                    color: [0.0, 0.0, self.light_ramp, phase01],
                });

                // Point light at the wick tip — sits at world_y =
                // WICK_TIP_Y * candle_scale above the table, at the candle's
                // jittered table-plane (cx_j, cy_j) anchor. The renderer
                // maps the pixel-layout x/y onto the table.
                let wick_world_y = WICK_TIP_Y * candle_scale * height_scale;
                // The footlight (index 4) sits well behind the camera, so its
                // wick is much farther from the action row than any of the
                // table-edge candles — bump its radius and intensity to
                // compensate, otherwise the front row stays in shadow.
                let (light_radius_mul, light_intensity) =
                    if i == 4 { (2.2, 1.0) } else { (1.0, 2.3) };
                point_lights.push(PointLight {
                    pos: [cx_j, cy_j, wick_world_y],
                    radius: radius_px * light_radius_mul * (1.05 + 0.3 * candle.flicker),
                    color: [1.0, 0.55, 0.22],
                    intensity: light_intensity * candle.flicker * self.light_ramp,
                });
                let _ = candle_w;
            }
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
            let row_y = layout.score_panel.y + layout.score_panel.h + layout.window_h * 0.08;
            let n = active_ids.len() as f32;
            // Default centered, but if the consumable inventory strip in the
            // top-right would clash with the right end of the row, shift the
            // row left so the two never overlap. The inventory anchors in
            // raw pixel space (line ~1739) — same coordinate system used
            // here before the renderer projects through the gameplay camera.
            let row_center_x_default = layout.window_w * 0.38;
            // Scale relic-dish geometry with window size so the dish reads the
            // same proportionally at every resolution. The constants below were
            // tuned at the 1920x1080 design size, so a window-height ratio of
            // 1.0 at 1080 keeps them unchanged and shrinks them on smaller
            // windows along with the rest of the layout.
            // Each relic is sized like a real desk-trinket: roughly the
            // footprint of a small carved figurine (~30mm cube), with ±10mm
            // jitter on each axis so the row reads as a varied collection
            // rather than uniform blocks. The dish cell is sized to fit
            // the largest possible relic plus a little breathing room.
            let cell_w = layout.mm(45.0);
            let total_w = cell_w * n;
            // Clamp the row's right edge to stay clear of the inventory
            // strip's left edge (with a small breathing margin). If shifting
            // left would push the row off the left side, just clamp to the
            // available space.
            let inv_margin = layout.mm(8.0);
            let max_right = match talisman_dish_strip {
                Some((sx, _, _, _)) => (sx - inv_margin).max(layout.mm(20.0)),
                None => layout.window_w - layout.mm(8.0),
            };
            let mut row_center_x = row_center_x_default;
            let row_right = row_center_x + total_w * 0.5;
            if row_right > max_right {
                row_center_x -= row_right - max_right;
            }
            let start_x = row_center_x - total_w * 0.5 + cell_w * 0.5;
            for (i, &rid) in active_ids.iter().enumerate() {
                // Pseudo-random per-relic size variation, deterministic on id.
                let seed = (rid as u32).wrapping_mul(2654435761) ^ 0x9E3779B9;
                let r0 = ((seed >> 8) & 0xFF) as f32 / 255.0;
                let r1 = ((seed >> 16) & 0xFF) as f32 / 255.0;
                let r2 = ((seed >> 24) & 0xFF) as f32 / 255.0;
                let half_x = layout.mm(15.0 + r0 * 5.0);
                let half_y = layout.mm(11.0 + r1 * 6.0);
                let half_z = layout.mm(10.0 + r2 * 4.0);

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

        // Focus / hover detection: in cursor mode, the Phase A sync in
        // `update()` already wrote `FocusTarget::Relic(i)` whenever the
        // cursor was over a projected relic rect; in keyboard / controller
        // mode the player navigates focus there with directional input.
        // Either way, the relic tooltip and outline show whenever
        // `self.focus` is `Some(Relic(i))`.
        let hovered_relic_idx: Option<usize> = match self.focus {
            Some(FocusTarget::Relic(i)) if i < ctx.projected_relic_rects.len() => Some(i),
            _ => None,
        };
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

        // Display-only focus tooltips: when the player navigates focus
        // onto a counter peg or the gold counter, surface a small info
        // panel with the current count. These are anchored to whatever
        // rect the focus rect graph published for the same target.
        match self.focus {
            Some(FocusTarget::Peg(kind)) => {
                let rect_idx = match kind {
                    PegKind::Hands => 0,
                    PegKind::Discards => 1,
                };
                if let Some(r) = ctx.projected_peg_rects[rect_idx] {
                    let (title, body) = match kind {
                        PegKind::Hands => (
                            "Hands Remaining".to_string(),
                            format!(
                                "{} of {} plays left this round. Each Play Hand consumes one peg.",
                                run.plays_remaining, run.mode.starting_plays,
                            ),
                        ),
                        PegKind::Discards => (
                            "Discards Remaining".to_string(),
                            format!(
                                "{} of {} discards left this round. Each Discard consumes one peg.",
                                run.discards_remaining, run.mode.starting_discards,
                            ),
                        ),
                    };
                    push_tooltip(
                        &mut hover_quads,
                        &mut hover_text,
                        r[0] + r[2] * 0.5,
                        r[1],
                        layout.window_w,
                        layout.window_h,
                        &title,
                        &body,
                    );
                }
            }
            Some(FocusTarget::Gold) => {
                // Anchor the tooltip just above the coin pile so the
                // hover label points at the actual gold rather than the
                // unrelated score-panel cartouche. When there's no gold
                // (no pile drawn) we fall back to skipping the tooltip
                // entirely — `FocusTarget::Gold` is only reachable from
                // the focus rect graph, which is also gated on a
                // populated `coin_pile_rect`.
                if let Some(rect) = coin_pile_rect {
                    push_tooltip(
                        &mut hover_quads,
                        &mut hover_text,
                        rect[0] + rect[2] * 0.5,
                        rect[1],
                        layout.window_w,
                        layout.window_h,
                        "Gold",
                        &format!(
                            "${}. Earned from clearing blinds. Spend in the shop on relics, ribbons, talismans, and pack rerolls.",
                            run.gold,
                        ),
                    );
                }
            }
            Some(FocusTarget::YakuTablet(i)) => {
                // Mirror the cursor-hover yaku tooltip path: same title +
                // body, same anchor (just above the projected tablet
                // rect). Falls back to the visible_previews entry rather
                // than re-running the preview pipeline.
                let (title, body) = if is_chicken_hand && i == 0 {
                    (
                        "\u{1F414} Chicken Hand".to_string(),
                        "A valid hand with no yaku. Scores base chips \u{00D7} 1 mult. \
                         Build toward a yaku to multiply your score."
                            .to_string(),
                    )
                } else if let Some(p) = visible_previews.get(i).copied() {
                    (
                        format!(
                            "{}  (+{} mult, +{} chips)",
                            p.kind.name(),
                            p.kind.mult_bonus(),
                            p.kind.chip_bonus()
                        ),
                        format!(
                            "{}.  Loadout: {}",
                            yaku_card_shape_text(p.kind),
                            if run.yaku_loadout.contains(&p.kind)
                                || matches!(
                                    p.kind,
                                    crate::core::yaku::YakuKind::FullHand
                                        | crate::core::yaku::YakuKind::Yakuhai
                                )
                            {
                                "yes (full strength)"
                            } else {
                                "no (scores at 50%)"
                            },
                        ),
                    )
                } else {
                    ("".to_string(), "".to_string())
                };
                if !title.is_empty() {
                    let (ax, ay) = match ctx.projected_yaku_tablet_rects.get(i).copied() {
                        Some([px, py, pw, _ph]) if pw > 0.0 && px.is_finite() && py.is_finite() => {
                            (px + pw * 0.5, py)
                        }
                        _ => (layout.window_w * 0.5, layout.window_h * 0.5),
                    };
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
            }
            Some(FocusTarget::Dora) => {
                push_tooltip(
                    &mut hover_quads,
                    &mut hover_text,
                    dora_rect[0] + dora_rect[2] * 0.5,
                    dora_rect[1],
                    layout.window_w,
                    layout.window_h,
                    "Dora Indicator",
                    "The face-up tile on the brass stand marks the round's bonus. Each tile in your hand matching the dora (the next tile after the indicator) adds extra mult when scored.",
                );
            }
            _ => {}
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
                                // Velocity tuned against the curtain density
                                // below: previous values (28 lateral / -55 z)
                                // were too gentle to push the curtain off-
                                // grid before the overlay finished fading,
                                // leaving the round draped in residual smoke
                                // that took the natural dissipation many
                                // seconds to clear. The debug `B` gust uses
                                // 1400 lateral / -120 z; we sit well below
                                // that so this still reads as a soft breath
                                // rather than a hurricane, but well above the
                                // old values so the field actually clears.
                                let lateral = 220.0 * edge_bias * envelope * row_strength;
                                wind_gusts.push(crate::render::draw_cmd::WindGust {
                                    center_px: (cx, cy),
                                    lift,
                                    velocity: [
                                        lateral,
                                        (6.0 + 4.0 * rf) * envelope * row_strength,
                                        -180.0 * envelope * row_strength,
                                    ],
                                    radius,
                                    density: -0.10 * envelope * row_strength,
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
        frame.background(super::BackgroundId::Black);
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
        let plaque_lift = layout.window_h * 0.44;
        // Push the plaque deeper into the scene (more negative world_z) so
        // it reads as hanging at the back of the room rather than right
        // above the player. pixel_y → world_z is a direct mapping in the
        // renderer's `pixel_to_world`, so subtracting from pixel_y here
        // moves the plaque back along the table's depth axis.
        let plaque_back_offset = layout.window_h * 0.18;
        // Debug visibility: gated at the call site (rather than post-filtered)
        // because the status placard below also uses `DrawCmd::Plaque(_)` and
        // a cmd-level `retain` couldn't tell them apart.
        if !ctx.debug_visibility.hide_blind_plaque {
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
        }
        // Ofuda only appears on boss blinds (where there's a rule to show).
        // Positioned on the LEFT flank of the score plaque, well outboard
        // of the left score-panel candle so the paper face never visually
        // collides with the wax. The previous anchor (right edge of the
        // modifier strip) sat directly behind the right candle and the
        // candle silhouette ate the title/rule decal. Hung high enough on
        // the back wall to read as a posted notice rather than a charm
        // dangling onto the action row.
        if !ofuda_title_text.is_empty() {
            let sp = layout.score_panel;
            let ms_rect = layout.modifier_strip;
            // Width/height of the ofuda paper face. A touch wider than the
            // old extents so the wrapped rule body has room to read.
            let ofuda_w = ms_rect.w * 0.22;
            let ofuda_h = ms_rect.h * 1.6;
            // Park the right edge of the paper a clear gap to the left of
            // the left score-panel candle. The candle stands at roughly
            // `sp.x - candle_w * 0.5 - edge_pad` (see candle layout above)
            // with `candle_w ≈ 83 * scale_c`, so a `100 * scale_c` clearance
            // keeps the paper outside the candle's footprint even with
            // jitter, and the `max(...)` clamp keeps it on-screen on
            // narrow windows where the score panel is already near the
            // left edge.
            let scale_c = (layout.window_w / 600.0).max(0.5);
            let candle_clearance = 100.0 * scale_c;
            let min_left_margin = ofuda_w * 0.5 + 8.0;
            let ofuda_cx = (sp.x - candle_clearance - ofuda_w * 0.5).max(min_left_margin);
            // Push it up the back wall: smaller pixel-y → farther into z
            // (recessed against the wall behind the table) and a taller
            // lift so the paper hangs above the score-plaque elevation
            // rather than beside it. The mesh is now upright (no
            // toward-camera tilt — see ofuda_tilt_x in the renderer), so
            // raising it visually moves it up the back wall on screen.
            let ofuda_cy = sp.y - sp.h * 0.4;
            let ofuda_lift = layout.window_h * 0.4;
            frame.ofuda(crate::render::draw_cmd::OfudaPlacement {
                center_pos: [ofuda_cx, ofuda_cy, ofuda_lift],
                // Real washi paper is ~0.1mm but the ofuda needs visible
                // depth under candlelight; ~3mm reads as a stiff paper
                // talisman without looking like a wooden tablet.
                extents: [ofuda_w, ofuda_h, layout.mm(3.0)],
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
            let peg_block_w = (sp.w * 0.21).max(112.0);
            let peg_block_h = layout.mm(21.0);
            let peg_block_d = layout.mm(35.0);
            // Anchor pegs at the right end of the plaque, inset so
            // they sit on the plaque face rather than over the edge.
            let peg_block_x = sp.x + sp.w * 0.85;
            let peg_block_y = sp.y + sp.h * 0.5 - plaque_back_offset;
            let peg_block_lift = plaque_lift;
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
        // Floating extruded-glyph score popups (per-step "+50" / "×3").
        // Pushed after the falling bones so they render on top of the
        // tumbling pile and the player's eye tracks the number, not the
        // backdrop.
        if self.score_popups.is_active() {
            frame.extruded_glyph_batch(self.score_popups.placements(now));
        }
        // Gold-flash overlay: a fullscreen quad that fades from peak alpha
        // to zero over ~400ms, fired when the cascade lands on its final
        // beat. Tints the entire screen warm so the closing crescendo
        // really *lands* visually. Drawn before the modifier-strip text
        // so 2D HUD labels stay readable through the flash.
        if let Some(t0) = self.gold_flash_at {
            const FLASH_MS: f32 = 400.0;
            let elapsed_ms = now.saturating_duration_since(t0).as_secs_f32() * 1000.0;
            if elapsed_ms < FLASH_MS {
                let t = elapsed_ms / FLASH_MS;
                // Ease-out-cubic decay: 1 → 0
                let env = (1.0 - t).powi(3);
                let alpha = 0.22 * env;
                frame.quad(GpuInstance {
                    rect: [0.0, 0.0, layout.window_w, layout.window_h],
                    color: [1.0, 0.85, 0.45, alpha],
                });
            }
        }
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
        if let Some(mirror) = bronze_mirror_placement {
            frame.mirror(mirror);
        }
        // Phase 5: brass talisman/zodiac dish on the right side of the
        // table. Dish is sized to wrap the consumables strip; pendants are
        // pushed in slot order.
        if let Some((sx, sy, sw, sh)) = talisman_dish_strip {
            let dish_pad_x = sw * 0.10;
            let dish_pad_y = sh * 0.40;
            frame.dish_explicit(crate::render::draw_cmd::DishExplicit {
                center_pos: [sx + sw * 0.5, sy + sh * 0.5, 0.0],
                // Brass tray rim ~10mm tall — small decorative dish.
                extents: [
                    sw + dish_pad_x * 2.0,
                    layout.mm(10.0),
                    sh + dish_pad_y * 2.0,
                ],
                pick_id: Some(PICK_CONSUMABLE_DISH),
            });
            if !ribbon_dish_placements.is_empty() {
                frame.zodiac_batch(ribbon_dish_placements);
            }
            if !talisman_dish_placements.is_empty() {
                frame.talisman_batch(talisman_dish_placements);
            }
        }

        // Phase 7: ambient table objects — physical coin pile (gold),
        // facedown wall stack (tiles remaining), and the dora indicator
        // stand. None of these are clickable; they're pure atmosphere that
        // makes the score line and wall counter physically present.
        //
        // Coin pile — sits at the back-right of the play area, just past
        // the right-hand score-panel candle, so it shares the rear pool of
        // candlelight with the score plaque. Coin count = min(gold,
        // MAX_COIN_SLOTS) so the pile visibly grows as the player
        // accumulates gold but caps before overflowing the slot pool.
        if run.gold > 0 {
            let coin_count = (run.gold.max(0) as usize).min(48);
            // Size the coins at true real-world proportions: a Japanese
            // 100-yen coin is 22.6mm × 1.7mm. We bump the thickness a bit
            // (≈3.5mm) so the disc reads clearly under candlelight at
            // typical viewing distances — at literal 1.7mm the coins
            // disappear into the dish floor.
            let coin_radius = layout.mm(11.3);
            let coin_thickness = layout.mm(3.5).max(2.0);
            // Brass dish under the pile so the coins read as resting in a
            // tray rather than scattered loose on the table. The pile-build
            // loop below seeds each coin's `support_y` at `dish_rim`, so the
            // coins always land on top of the rim regardless of how tall it
            // is — restoring the rim doesn't bury them. The scatter footprint
            // is intentionally tight (only ~3 coin radii) so 48 coins build
            // up into a visible mound rather than spreading out flat across
            // a wide tray, while still leaving the settling pass enough
            // room to lay the bottom layer down before stacking.
            let scatter_half = coin_radius * 3.0;
            // Pile center tracks the back-right candle: recompute that
            // candle's layout locally (mirroring the candle pass above) so
            // the dish stays glued to it as the score panel resizes, then
            // shift one candle-radius + dish-half outward so the brass rim
            // sits just to the right of the candle silhouette. The clamp
            // keeps the tray inside the window even at narrow widths.
            let coin_sp = layout.score_panel;
            let coin_candle_w = layout.mm(78.0) * 0.72;
            let coin_edge_pad = layout.mm(12.0);
            let coin_back_z_push = coin_candle_w;
            let right_candle_x = (coin_sp.x + coin_sp.w + coin_candle_w + coin_edge_pad)
                .min(layout.window_w - coin_candle_w * 0.5 - 4.0);
            let right_candle_y = coin_sp.y + coin_sp.h * 0.5 - coin_back_z_push;
            let dish_half_w = scatter_half + coin_radius * 2.0;
            // Sit past the candle's right edge from the camera's POV. We
            // intentionally do *not* clamp to the window — the table
            // extends beyond the visible frame in 3D, and clamping would
            // pull the dish back on top of the candle whenever the right
            // candle is already pinned to the window edge.
            let pile_cx = right_candle_x + coin_candle_w * 0.5 + dish_half_w + coin_edge_pad;
            let pile_cy = right_candle_y;
            let dish_rim = (coin_thickness * 2.5).max(10.0);
            let dish_w = scatter_half * 2.0 + coin_radius * 4.0;
            let dish_d = scatter_half * 2.0 + coin_radius * 4.0;
            frame.dish_explicit(crate::render::draw_cmd::DishExplicit {
                center_pos: [pile_cx, pile_cy, 0.0],
                extents: [dish_w, dish_rim, dish_d],
                pick_id: None,
            });
            // Settled pile on top of the dish. For each coin we generate
            // several candidate (lx, lz) positions and pick the one with
            // the *lowest* resulting support height — this causes the
            // first coins to tile the dish floor before any stacking
            // begins, killing the unrealistic vertical pillars the naive
            // single-shot random placement produced. Two coin discs of
            // radius `r` overlap whenever their centers are within `2r`,
            // and overlapping coins must rest on each other rather than
            // intersect, so the support test uses exactly `2r`.
            let mut coins: Vec<crate::render::draw_cmd::CoinPlacement> =
                Vec::with_capacity(coin_count);
            let mut placed: Vec<(f32, f32, f32)> = Vec::with_capacity(coin_count);
            let overlap_r = coin_radius * 2.0;
            let overlap_r2 = overlap_r * overlap_r;
            const CANDIDATES_PER_COIN: u32 = 12;
            // Seed a stable RNG so the pile shape doesn't jitter every
            // frame as `gold` stays constant. Using a fixed seed (rather
            // than seeding from `coin_count`) means new coins drop into
            // the existing pile instead of reshuffling the whole stack
            // when gold ticks up.
            use rand::rngs::StdRng;
            use rand::{RngExt, SeedableRng};
            let mut rng = StdRng::seed_from_u64(0xC01_C0FFEE);
            for _ in 0..coin_count {
                let mut best: Option<(f32, f32, f32, f32)> = None;
                for _ in 0..CANDIDATES_PER_COIN {
                    let lx = rng.random_range(-scatter_half..scatter_half);
                    let lz = rng.random_range(-scatter_half..scatter_half);
                    let rot_y = rng.random_range(-std::f32::consts::PI..std::f32::consts::PI);
                    let mut support_y = dish_rim;
                    for (ox, oz, top_y) in &placed {
                        let ddx = lx - ox;
                        let ddz = lz - oz;
                        if ddx * ddx + ddz * ddz < overlap_r2 && *top_y > support_y {
                            support_y = *top_y;
                        }
                    }
                    match best {
                        None => best = Some((lx, lz, support_y, rot_y)),
                        Some((_, _, by, _)) if support_y < by => {
                            best = Some((lx, lz, support_y, rot_y));
                        }
                        _ => {}
                    }
                }
                let (lx, lz, support_y, rot_y) = best.unwrap();
                let world_y = support_y + coin_thickness * 0.5;
                placed.push((lx, lz, world_y + coin_thickness * 0.5));
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
            // Hover region for the coin pile → "Gold" glossary entry.
            // We reuse the `coin_pile_rect` computed at the top of
            // `draw_frame` so the glossary anchor, the focus-rect graph
            // entry for `FocusTarget::Gold`, and the gold tooltip all
            // share one source of truth — keeping the focus ring, hover
            // tooltip, and physical pile visually locked together.
            if let Some(rect) = coin_pile_rect {
                frame.glossary_anchor(rect, "Gold");
            }
        }

        // Flying coin animations (gold changes).
        {
            let flying = self.flying_coins.placements();
            if !flying.is_empty() {
                frame.coin_batch(flying);
            }
        }

        // Dora indicator stand — hidden until the tile pipeline supports
        // placing a face-up indicator tile on the plinth.
        // let dora_cx = dora_rect[0] + dora_rect[2] * 0.5;
        // let dora_cy = dora_rect[1] + dora_rect[3] * 0.5;
        // frame.dora_stand(crate::render::draw_cmd::DoraStandPlacement {
        //     world_pos: [dora_cx, dora_cy, 0.0],
        //     extents: [dora_rect[2], layout.mm(40.0), layout.mm(20.0)],
        // });

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

        // Append the deferred focus rect entries (hand tiles, relics,
        // pegs, gold) before the centralized focus ring so the lookup
        // can find them. The button-bar and consumable strip already
        // pushed their entries inline above.
        for (i, rect) in ctx.projected_hand_rects.iter() {
            focus_rect_graph.push((FocusTarget::HandTile(*i), *rect));
        }
        if ctx.projected_hand_rects.is_empty() {
            for (i, slot) in hand_slots.iter().enumerate() {
                focus_rect_graph.push((FocusTarget::HandTile(i), [slot.0, slot.1, slot.2, slot.3]));
            }
        }
        for (i, r) in ctx.projected_relic_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 {
                focus_rect_graph.push((FocusTarget::Relic(i), *r));
            }
        }
        if let Some(r) = ctx.projected_peg_rects[0] {
            if r[2] > 1.0 && r[3] > 1.0 {
                focus_rect_graph.push((FocusTarget::Peg(PegKind::Hands), r));
            }
        }
        if let Some(r) = ctx.projected_peg_rects[1] {
            if r[2] > 1.0 && r[3] > 1.0 {
                focus_rect_graph.push((FocusTarget::Peg(PegKind::Discards), r));
            }
        }
        // Anchor the gold focus rect to the actual 3D coin pile (when
        // there is gold to display). The pile rect was computed up at
        // the top of `draw_frame` so the focus ring, hover tooltip, and
        // physical pile draw all share one source of truth.
        if let Some(rect) = coin_pile_rect {
            focus_rect_graph.push((FocusTarget::Gold, rect));
        }
        // Yaku tablets — push the projected rects into the focus graph
        // so spatial nav can land on them. We use the projected rects
        // (one frame stale) to match where the player actually sees the
        // tablets after camera projection; on the very first frame they
        // may be missing, in which case the tablet is briefly skipped.
        for (i, r) in ctx.projected_yaku_tablet_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite() {
                focus_rect_graph.push((FocusTarget::YakuTablet(i), *r));
            }
        }
        // Dora indicator — display-only focus target so a controller
        // player can read what the brass plinth represents.
        focus_rect_graph.push((FocusTarget::Dora, dora_rect));

        // Centralized focus ring: a single brass frame around whatever
        // `self.focus` is currently pointing at, pushed into the hover
        // layer so it sits above hand tile faces. The hand strip itself
        // gets a richer focus highlight from the renderer (driven by
        // `frame.focus`), so we suppress the 2D ring for HandTile to
        // avoid double-ringing.
        if let Some(target) = self.focus {
            if !matches!(target, FocusTarget::HandTile(_)) {
                let rect_lookup = focus_rect_graph
                    .iter()
                    .find_map(|(t, r)| (*t == target).then_some(*r));
                if let Some(rect) = rect_lookup {
                    push_focus_ring(rect, scale, &mut hover_quads);
                }
            }
        }

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
        frame.candle_light_count = candle_placements.len() as u32;
        // Real candle flame: ~30 mm tall. The renderer passes this through
        // to the volumetric lightbake shader as the analytic flame envelope
        // height in world units.
        frame.flame_height_world = layout.mm(30.0);
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

        // Stash the focus rect graph for the next frame's `update()` to
        // hit-test the cursor and run spatial navigation against.
        *self.last_focus_rects.borrow_mut() = focus_rect_graph;

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

/// True if `(cx, cy)` lies inside the `[x, y, w, h]` rect.
#[inline]

/// Plain-language hand-shape description for a yaku, mirrored from the
/// glossary so the gameplay tooltip and the help overlay agree.
fn yaku_card_shape_text(yk: crate::core::yaku::YakuKind) -> &'static str {
    // Suit emoji match tile_suit_emoji: 🎴 Characters, 🎋 Bamboo, 🔴 Circles.
    // Honor emoji: 🐉 Dragon, 🌬 Wind.
    use crate::core::yaku::YakuKind;
    match yk {
        YakuKind::Tanyao => {
            "All tiles 2\u{2013}8, no honors or terminals (e.g. \u{1f3b4}234 \u{1f38b}567 \u{1f534}88)"
        }
        YakuKind::Toitoi => {
            "All triplets and kongs, no sequences (e.g. \u{1f3b4}222 \u{1f38b}555 \u{1f534}999)"
        }
        YakuKind::FullHand => "Complete 14-tile hand: 4 melds + 1 pair",
        YakuKind::Yakuhai => {
            "Triplet of any dragon or round wind (e.g. \u{1f409}\u{1f409}\u{1f409})"
        }
        YakuKind::Iipeikou => {
            "Two identical sequences in one suit (e.g. \u{1f38b}123 \u{1f38b}123)"
        }
        YakuKind::SanshokuDoujun => {
            "Same sequence in all 3 suits (e.g. \u{1f3b4}456 \u{1f38b}456 \u{1f534}456)"
        }
        YakuKind::Ittsu => {
            "1\u{2013}9 straight in one suit (e.g. \u{1f38b}123 \u{1f38b}456 \u{1f38b}789)"
        }
        YakuKind::Honitsu => {
            "One number suit + honors only (e.g. \u{1f38b}234 \u{1f38b}678 \u{1f32c}\u{1f32c}\u{1f32c})"
        }
        YakuKind::Chinitsu => {
            "All one number suit, no honors (e.g. \u{1f38b}123 \u{1f38b}456 \u{1f38b}789 \u{1f38b}11)"
        }
        YakuKind::Junchan => {
            "Every meld has a 1 or 9 (e.g. \u{1f38b}123 \u{1f3b4}789 \u{1f534}111 \u{1f38b}99)"
        }
        YakuKind::Honroutou => {
            "Only 1s, 9s, and honors (e.g. \u{1f38b}111 \u{1f3b4}999 \u{1f32c}\u{1f32c}\u{1f32c})"
        }
        YakuKind::Chiitoitsu => {
            "Seven distinct pairs (e.g. \u{1f3b4}11 \u{1f3b4}33 \u{1f38b}55 \u{1f38b}77 \u{1f534}22 \u{1f534}44 \u{1f32c}\u{1f32c})"
        }
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
