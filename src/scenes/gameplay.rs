//! Gameplay scene — the main tile-playing loop.
//!
//! **Concurrent UI / flow state** (orthogonal concerns; check the right gate per feature):
//! - App-level [`crate::UpdateCtx::transitioning`] — global scene replace in progress; input blocked.
//! - `pending_chamber` — opening round; hand not yet applied until the candle light-ramp completes.
//! - `pause_menu` — pause root + optional embedded options.
//! - `cascade_queue` — scoring presentation; also drives [`crate::scenes::SceneBehavior::has_blocking_overlay`].
//! - `journal_transition` + overlay push — Yaku Journal from the table book.
//! - `tutorial_overlay` — in-run hints when the tutorial is active.

mod action_prompts;
mod animation_state;
mod cascade_controller;
mod cascade_hud;
pub(crate) use cascade_hud::CascadeShowcase;
pub(crate) mod discard_animation;
mod focus;
mod glb_anchors;
mod hand_layout;
mod input_handler;
mod onboarding_hints;
mod scene_behavior;
mod score_counter;

use std::collections::VecDeque;
use std::time::Instant;

use super::journal_transition::JournalTransition;
use super::pause_menu::PauseMenu;
use super::{ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};
use crate::core::scoring::StepKind;
use crate::game::cascade::{CascadeTuning, ScoringCascade};
use crate::game::engine::{CommandData, GameCommand, GameEngine};
use crate::game::run::DiscardUndoSnapshot;
use crate::render::animation::ENTITY_SCORE_PANEL;
use crate::render::draw_cmd::{DrawCmd, Object3d, Object3dKind, ShowcaseTilePlacement, UiFrame};
use crate::render::flying_coins::FlyingCoinSystem;
use crate::render::particles::ParticleSystem;
use crate::render::score_popups::ScorePopupSystem;
use crate::render::score_reel::ScoreReel;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel, build_instances_from_layout};
use crate::ui::input::UiAction;

use cascade_hud::{CascadeHudState, build_cascade_hud_placements};
use focus::{FocusTarget, PegKind, focus_kind};
use hand_layout::hand_slots_for_count;

use crate::ui::focus_nav::push_focus_ring;

pub struct GameplayScene {
    /// Queue of scoring cascade animations. The front entry is the active
    /// cascade; when it finishes, it is popped and the next starts
    /// automatically. This prevents a second scoring event (e.g. auto-trigger
    /// after commit) from silently overwriting an in-flight cascade.
    cascade_queue: VecDeque<(ScoringCascade, Option<CascadeShowcase>)>,
    /// Displayed score — ticked by the cascade, snaps to real score when idle.
    displayed_score: u64,
    /// Previous frame's displayed score; used to detect changes and fire the
    /// score-pop tween on the score panel.
    prev_displayed_score: u64,
    /// Particle effects for scoring.
    particles: ParticleSystem,
    /// Animated coins that fly into/out of the dish when gold changes.
    flying_coins: FlyingCoinSystem,
    /// Gold value last frame — compared each update to detect changes.
    prev_gold: i32,
    /// Floating 3D extruded-glyph score popups ("+50", "×3", running total).
    /// Spawned at the source of each cascade step (relic rect or modifier
    /// strip centre) and drift toward the score panel before despawning.
    score_popups: ScorePopupSystem,
    /// Odometer-style floating digit reel showing the current score.
    score_reel: ScoreReel,
    /// Last target score the reel was sized for. When this differs from
    /// `run.target_score` (scene init or a new round) the reel is reset so
    /// it starts with exactly as many zero columns as the target has digits.
    score_reel_target: u64,
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
    /// When set, a discard removed tiles but [`RunState::refill_hand`] has not
    /// run yet — the UI waits for the river animation first. See
    /// [`RunState::discard_refill_pending`]. Timing uses
    /// [`discard_animation::DiscardAnimationBatch::total_duration`] with a
    /// ceiling from [`CascadeTuning::discard_refill_cap_ms`].
    pending_discard_refill: Option<Instant>,
    /// In-flight tiles animating from hand into the discard river.
    active_discard_anim: Option<discard_animation::DiscardAnimationBatch>,
    /// Tiles resting in the river until the next discard replaces them.
    river_settled_tiles: Vec<discard_animation::RiverSettledTile>,
    /// Previous river pile sinking away before despawn.
    river_sink_batch: Option<discard_animation::RiverSinkBatch>,
    /// Copy of cascade tuning for draw (updated each frame from [`UpdateCtx`]).
    cached_cascade_tuning: CascadeTuning,
    /// Latest cursor position (window coords), captured each update for hover picking.
    cursor_pos: (f32, f32),
    /// Wall-clock time used to advance candle flicker (independent of the
    /// game's `Instant::now()` references so the candles keep moving even if
    /// the game logic is paused).
    candle_time: f32,
    /// Per-relic glow start times. Populated as the cascade reveals each step
    /// whose source matches a relic display name. The glow fades over
    /// `RELIC_GLOW_LIFETIME` and the entry is evicted afterward.
    relic_glow_starts: rustc_hash::FxHashMap<crate::core::relic::RelicId, Instant>,
    /// Tracks the most recent step index whose reveal edge we've already
    /// processed (relic glow + ScoreStepRevealed bus event). Prevents
    /// re-firing reveal-edge effects every frame while the cascade holds
    /// on the same step.
    last_revealed_step: Option<usize>,
    /// Whether the current cascade has already emitted its `ScoreCascadeFinal`
    /// event. Reset when a new cascade starts.
    cascade_final_emitted: bool,
    /// Snapshot of the cascade's current frame, captured during update and
    /// consumed by the draw path to render the chips/×/mult trio under the
    /// plaque and (during HandOff) tween it up into the score reel. `None`
    /// when no cascade is active.
    cascade_hud: Option<CascadeHudState>,
    /// Future timestamps at which a queued `DoraScored` chime should fire.
    /// Populated with one entry per dora tile when the cascade reveals a
    /// "Dora ×N" step, so multiple dora audibly play in sequence rather
    /// than a single stacked hit. Drained each frame as entries come due.
    pending_dora_chimes: Vec<Instant>,
    /// Yaku Journal overlay — Balatro-style run-stats page listing every
    /// yaku with its level, leveled bonuses, and run play count. Opened
    /// by clicking the Journal book on the table.
    /// Hand size observed last frame. Used to detect deal events for the
    /// opening light-ramp anchor.
    prev_hand_len: usize,
    /// Lights-out transition: ramps from 0.0 (dark) to 1.0 (full brightness)
    /// after the opening deal lands. Multiplied into candle intensity, flame
    /// alpha, and point-light brightness so the scene fades in via the
    /// candles "sparking on" rather than a smoke curtain.
    light_ramp: f32,
    /// Wall time of the deal that started the ramp. Kept until `light_ramp`
    /// reaches 1.0 so the ramp cannot strand mid-way.
    light_ramp_anchor: Option<Instant>,
    /// Blind to apply to the run once the opening light-ramp completes. Set
    /// by callers that enter the gameplay scene from pick-blind / onboarding
    /// / retry paths; consumed by `update()` when `light_ramp >= 1.0`. While
    /// `Some`, the scene renders the *previous* round's state behind the
    /// opening transition — the player doesn't see the round's hand,
    /// target, or on-start relic triggers until the fade-in finishes, so
    /// Sweepstakes coin showers / DoraCrown reveals / future on-round-start
    /// effects are visible rather than hidden by the dark transition.
    pending_chamber: Option<crate::core::rules::ChamberKind>,
    /// Debug-only: when true, the renderer overlays world-axes bars at the
    /// camera target so we can see which direction is +X / +Y / +Z while
    /// dialing in placements. Toggled from the native Debug menu.
    debug_show_axes: bool,
    /// Cascade Lab embeds this scene for presentation only — block table
    /// input/hover while the lab panel owns interaction.
    lab_mode: bool,
    /// Candle flare intensity — spikes when a single hand scores more than
    /// the entire blind target, then decays exponentially back to 0.0.
    /// Multiplied into candle intensity, radius, and flame brightness so
    /// the room visibly flares up on a monster hand.
    candle_flare: f32,
    /// Controller "pick up relic with A" drag source. While set, d-pad/stick
    /// navigation moves the focused drop target and releasing A commits the
    /// swap if focus is on another relic.
    held_relic_drag: Option<usize>,
    /// Hold-to-cash-in: press time while the player is charging a cash-in
    /// (gamepad trigger / keyboard **T** / Confirm on the Cash In button).
    /// Mirrors the shop's hold-to-sell — completes at
    /// [`cash_in_hold_seconds()`], cancelled on the matching release.
    cash_in_hold_started: Option<Instant>,
    /// Hand-strip marquee multi-select. `Some` while Confirm is held over a
    /// hand tile (LMB / Space / Enter / gamepad A). Each focus or pointer
    /// movement updates `current_slot` and re-applies the marquee against
    /// the snapshot taken at press time. Released on `ConfirmRelease`.
    marquee: Option<crate::ui::input::MarqueeSelect>,
    /// One-shot FoV pop triggered when a placement completes the structure.
    final_tiles_fov_pop_at: Option<Instant>,
    /// Per-slot tile identity tracker for pop-in animation. Each entry is
    /// the raw `u32` id of the tile in that slot; when it changes (deal,
    /// refill) we reset the pop-in tween for that slot.
    hand_tile_uids: Vec<u32>,
    /// Vertical pop-in offset per slot (0 → 1 ramps up on new deal, then
    /// held at 1 when settled). Multiplied by slot height to get world offset.
    hand_slide_y: Vec<f32>,
    /// Horizontal shuffle offset per slot (in pixels). Used when tiles are
    /// sorted so they slide smoothly into their new positions.
    hand_slide_x: Vec<f32>,
    /// Cover-open / zoom sequence before pushing [`super::YakuJournalScene`], matching the shop book.
    journal_transition: Option<JournalTransition>,
    journal_open_amount: f32,
    journal_open_target: f32,
    /// Set after the forward transition pushes the journal; cleared when the closing animation starts.
    journal_was_open: bool,
    /// Candle + score-reel tunables (not authored as GLB spawn empties).
    pub positions: crate::ui::scene_layout::GameplayPositions,
    /// Snapshot of run state before the last discard; kept even when the
    /// accessibility undo option is off so turning it on can still undo that discard.
    /// Cleared by any other gameplay action.
    pub(super) discard_undo: Option<DiscardUndoSnapshot>,
    /// Hand slot indices to wiggle + pulse red after a rejected meld commit.
    invalid_meld_flash_slots: Vec<usize>,
    /// Wall time when [`invalid_meld_flash_slots`] was last set.
    invalid_meld_flash_at: Option<Instant>,
    /// Wall time when a boss-rule rejection last fired (boss icon wiggle decay).
    invalid_boss_flash_at: Option<Instant>,
    /// Wall time when a tutorial-gated action was rejected (lessons panel wiggle).
    tutorial_panel_wiggle_at: Option<Instant>,
    /// Cached each `update`: selection is invalid only because of boss rules.
    boss_rule_feedback_live: bool,
}

/// How long after the opening deal before the candles begin sparking on.
const LIGHT_RAMP_DELAY_SECS: f32 = 0.3;
/// Duration over which candles ramp from dark to full brightness.
const LIGHT_RAMP_DURATION_SECS: f32 = 0.8;

/// How long a relic glow lingers after activation.
const RELIC_GLOW_LIFETIME: std::time::Duration = std::time::Duration::from_millis(900);

/// Hold-to-cash-in duration (gamepad trigger / keyboard **T** / Confirm on the
/// Cash In button). Drives the HUD progress ring + cash-in gate.
#[inline]
pub(crate) fn cash_in_hold_seconds() -> f32 {
    crate::ui::prompt_hold_ring::hold_act_seconds()
}

/// Peak flare intensity when a single hand exceeds the entire blind.
const CANDLE_FLARE_PEAK: f32 = 1.5;
/// Exponential decay rate for the candle flare (per second). Higher = faster fade.
const CANDLE_FLARE_DECAY: f32 = 2.5;
/// Duration of the quick camera pop when the structure is completed.
const FINAL_TILES_FOV_POP_SECS: f32 = 0.22;
/// Peak FoV reduction in degrees during the completion pop.
const FINAL_TILES_FOV_POP_DEGREES: f32 = 3.6;
/// How long rejected meld tiles keep their red glow pulse.
const INVALID_MELD_FLASH_SECS: f32 = 0.55;
/// How long the boss icon keeps its reject wiggle after a failed commit.
const INVALID_BOSS_FLASH_SECS: f32 = 0.55;
/// How long the lessons tutorial panel wiggles after a blocked action.
const TUTORIAL_PANEL_WIGGLE_SECS: f32 = 0.55;
/// Fullscreen gold tint on the cascade's final beat (see `gold_flash_at`).
const GOLD_FLASH_SECS: f32 = 0.4;

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

/// Click id for the optional post-discard Undo control (2D HUD button).
const UNDO_DISCARD_CLICK_ID: u32 = 0x9280;
/// Click id for the lower-right wall supply HUD.
const WALL_HUD_CLICK_ID: u32 = 0x9281;

/// Screen center of physical relic tray slot `idx` (`player_relic` empties).
pub(crate) fn relic_tray_slot_screen_center(
    w: f32,
    h: f32,
    env_height_scale: f32,
    idx: usize,
) -> Option<(f32, f32)> {
    glb_anchors::relic_tray_screen_center(w, h, env_height_scale, idx).ok()
}

impl Default for GameplayScene {
    fn default() -> Self {
        Self::new()
    }
}

impl GameplayScene {
    pub(super) fn display_tile(
        tile: crate::core::tile::Tile,
        run: &crate::game::run::RunState,
    ) -> crate::core::tile::Tile {
        GameEngine::display_tile(tile, run)
    }

    pub(super) fn display_tiles(
        tiles: impl IntoIterator<Item = crate::core::tile::Tile>,
        run: &crate::game::run::RunState,
    ) -> Vec<crate::core::tile::Tile> {
        GameEngine::display_tiles(tiles, run)
    }

    /// Debug-only: fire a burst of demo score popups that stream from
    /// hand-tile source positions to the reel anchor. Used to eyeball the
    /// polychrome streaming effect without playing a hand.
    pub fn debug_demo_cascade(
        &mut self,
        layout: &crate::ui::layout::LayoutResult,
        run: &crate::game::run::RunState,
    ) {
        let interaction = GameEngine::read_interaction(run);
        if score_counter::try_resolve_score_cascade_layout(layout, &self.positions, 1.0).is_none() {
            log::warn!(
                "[Debug] demo cascade: gameplay.glb score frame unavailable; using fallbacks"
            );
        }
        let fly_dest = score_counter::resolve_score_popup_fly_dest(layout, &self.positions, 1.0);
        let dest = (fly_dest.px, fly_dest.py);
        let dest_lift = Some(fly_dest.lift_z);

        // Source positions: hand slot centers if available, else fall back
        // to a horizontal spread across the modifier strip.
        let hand_slots = hand_slots_for_count(layout, interaction.hand_len.max(5));
        let sources: Vec<(f32, f32)> = if !hand_slots.is_empty() {
            hand_slots
                .iter()
                .take(5)
                .map(|&(x, y, w, h)| (x + w * 0.5, y + h * 0.5))
                .collect()
        } else {
            let ms = layout.modifier_strip;
            (0..5)
                .map(|i| (ms.x + ms.w * (0.15 + 0.175 * i as f32), ms.y + ms.h * 0.5))
                .collect()
        };

        let steps: [(StepKind, &str, f32); 5] = [
            (StepKind::Chips, "+50", 50.0),
            (StepKind::Mult, "+3.0x", 3.0),
            (StepKind::Chips, "+120", 120.0),
            (StepKind::Yen, "Lucky Cat", 10.0),
            (StepKind::Mult, "+1.5x", 1.5),
        ];
        for (i, (kind, label, mag)) in steps.iter().enumerate() {
            let src = sources[i % sources.len()];
            let timing = crate::render::score_popups::PopupMotionTiming::shipping_default();
            self.score_popups.spawn(
                *label,
                crate::render::world_space::LayoutAnchorPx {
                    px: src.0,
                    py: src.1,
                    lift_z: crate::render::score_popups::TABLE_POPUP_LIFT_Z,
                },
                dest,
                dest_lift,
                *kind,
                *mag,
                timing,
            );
        }
        log::info!("[Debug] Spawned demo score cascade");
    }

    /// Cascade reveal, score popups flying to the reel, odometer rolls, and the
    /// final-beat gold flash.
    pub(super) fn scoring_presentation_active(&self, now: Instant) -> bool {
        !self.cascade_queue.is_empty()
            || self.score_reel.is_animating(now)
            || self.score_popups.is_active()
            || self.gold_flash_active(now)
    }

    /// Scoring presentation finished and celebration particles settled — safe to
    /// fire a deferred `RoundComplete` / `GameOver`.
    pub fn ready_for_round_end(&self, now: Instant) -> bool {
        !self.scoring_presentation_active(now) && !self.particles.is_active()
    }

    fn gold_flash_active(&self, now: Instant) -> bool {
        self.gold_flash_at.is_some_and(|t0| {
            now.saturating_duration_since(t0).as_secs_f32() < GOLD_FLASH_SECS
        })
    }

    /// Whether the cascade is actively animating (for redraw requests).
    pub fn is_animating(&self) -> bool {
        let now = Instant::now();
        self.scoring_presentation_active(now)
            || self.particles.is_active()
            || self.flying_coins.is_active()
            || self.pending_discard_refill.is_some()
            || discard_animation::discard_animation_active(self)
            || !self.relic_glow_starts.is_empty()
            || self.final_tiles_fov_pop_active()
            || self.boss_rule_feedback_active(now)
            || self.tutorial_panel_wiggle_active(now)
    }

    fn final_tiles_fov_pop_active(&self) -> bool {
        let Some(t) = self.final_tiles_fov_pop_at else {
            return false;
        };
        Instant::now().saturating_duration_since(t).as_secs_f32() < FINAL_TILES_FOV_POP_SECS
    }

    pub(super) fn trigger_invalid_meld_flash(
        &mut self,
        run: &crate::game::run::RunState,
        hand: &[crate::core::tile::Tile],
        selected: &[bool],
    ) {
        let selected_tiles: Vec<_> = hand
            .iter()
            .zip(selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(t, _)| *t)
            .collect();
        if selected_tiles.is_empty() {
            return;
        }
        let rules = run.validation_rules_for_structure_commits();
        let bad_ids: rustc_hash::FxHashSet<u32> =
            crate::core::hand::non_contributing_tile_ids(&selected_tiles, &rules)
                .into_iter()
                .collect();
        self.invalid_meld_flash_slots = hand
            .iter()
            .enumerate()
            .filter(|(i, t)| selected.get(*i).copied().unwrap_or(false) && bad_ids.contains(&t.id))
            .map(|(i, _)| i)
            .collect();
        if !self.invalid_meld_flash_slots.is_empty() {
            self.invalid_meld_flash_at = Some(Instant::now());
        }
        if run.selection_blocked_by_ordeal_rules(&selected_tiles) {
            self.invalid_boss_flash_at = Some(Instant::now());
        }
    }

    pub(super) fn clear_boss_rule_feedback(&mut self) {
        self.invalid_boss_flash_at = None;
    }

    pub(super) fn trigger_tutorial_panel_wiggle(&mut self) {
        self.tutorial_panel_wiggle_at = Some(Instant::now());
    }

    pub(super) fn reject_tutorial_gated_action(
        &mut self,
        bus: &mut crate::game::event_bus::EventBus,
    ) {
        bus.push(crate::game::event_bus::GameEvent::UiSound(
            crate::sfx_id::SfxId::InvalidAction,
        ));
        self.trigger_tutorial_panel_wiggle();
    }

    fn tutorial_panel_wiggle_active(&self, now: Instant) -> bool {
        self.tutorial_panel_wiggle_at.is_some_and(|t0| {
            now.saturating_duration_since(t0).as_secs_f32() < TUTORIAL_PANEL_WIGGLE_SECS
        })
    }

    pub(super) fn tutorial_panel_wiggle_x(&self, now: Instant) -> f32 {
        let Some(t0) = self.tutorial_panel_wiggle_at else {
            return 0.0;
        };
        let elapsed = now.saturating_duration_since(t0).as_secs_f32();
        if elapsed >= TUTORIAL_PANEL_WIGGLE_SECS {
            return 0.0;
        }
        let fade = 1.0 - (elapsed / TUTORIAL_PANEL_WIGGLE_SECS);
        10.0 * fade * (elapsed * 34.0).sin()
    }

    /// True while the boss icon should pulse (live boss-blocked selection or reject decay).
    fn boss_rule_feedback_active(&self, now: Instant) -> bool {
        self.invalid_boss_flash_phase(now).0 > 0.0 || self.boss_rule_feedback_live
    }

    /// `(glow 0..1, wiggle px)` for boss-icon feedback.
    pub(super) fn boss_rule_feedback(&self, now: Instant, live_blocked: bool) -> (f32, f32) {
        let (reject_glow, reject_wiggle) = self.invalid_boss_flash_phase(now);
        if reject_glow > 0.0 {
            return (reject_glow, reject_wiggle);
        }
        if live_blocked {
            let pulse = 0.55 + 0.45 * (self.candle_time * 5.2).sin();
            let wiggle = 3.0 * (self.candle_time * 11.0).sin();
            return (pulse, wiggle);
        }
        (0.0, 0.0)
    }

    fn invalid_boss_flash_phase(&self, now: Instant) -> (f32, f32) {
        let Some(t0) = self.invalid_boss_flash_at else {
            return (0.0, 0.0);
        };
        let elapsed = now.saturating_duration_since(t0).as_secs_f32();
        if elapsed >= INVALID_BOSS_FLASH_SECS {
            return (0.0, 0.0);
        }
        let fade = 1.0 - (elapsed / INVALID_BOSS_FLASH_SECS);
        let pulse = 0.65 + 0.35 * (elapsed * 16.0).sin();
        let wiggle = 9.0 * fade * (elapsed * 34.0).sin();
        (fade * pulse, wiggle)
    }

    /// `(strength 0..1, elapsed seconds)` for the active invalid-meld feedback.
    fn invalid_meld_flash_phase(&self, now: Instant) -> (f32, f32) {
        let Some(t0) = self.invalid_meld_flash_at else {
            return (0.0, 0.0);
        };
        let elapsed = now.saturating_duration_since(t0).as_secs_f32();
        if elapsed >= INVALID_MELD_FLASH_SECS {
            return (0.0, elapsed);
        }
        let fade = 1.0 - (elapsed / INVALID_MELD_FLASH_SECS);
        let pulse = 0.65 + 0.35 * (elapsed * 16.0).sin();
        (fade * pulse, elapsed)
    }

    fn final_tiles_fov_pop_offset_deg(&self, now: Instant) -> f32 {
        let Some(t) = self.final_tiles_fov_pop_at else {
            return 0.0;
        };
        let elapsed = now.saturating_duration_since(t).as_secs_f32();
        if elapsed >= FINAL_TILES_FOV_POP_SECS {
            return 0.0;
        }
        let progress = (elapsed / FINAL_TILES_FOV_POP_SECS).clamp(0.0, 1.0);
        FINAL_TILES_FOV_POP_DEGREES * (progress * std::f32::consts::PI).sin()
    }

    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            cascade_queue: VecDeque::new(),
            displayed_score: 0,
            prev_displayed_score: 0,
            particles: ParticleSystem::new(),
            flying_coins: FlyingCoinSystem::new(),
            prev_gold: 0,
            score_popups: ScorePopupSystem::new(),
            score_reel: ScoreReel::new(),
            score_reel_target: 0,
            gold_flash_at: None,
            last_frame: now,
            pause_menu: PauseMenu::new(),
            focus: None,
            last_focus_rects: std::cell::RefCell::new(Vec::new()),
            pending_discard_refill: None,
            active_discard_anim: None,
            river_settled_tiles: Vec::new(),
            river_sink_batch: None,
            cached_cascade_tuning: CascadeTuning::default(),
            cursor_pos: (0.0, 0.0),
            candle_time: 0.0,
            relic_glow_starts: rustc_hash::FxHashMap::default(),
            last_revealed_step: None,
            cascade_final_emitted: false,
            cascade_hud: None,
            pending_dora_chimes: Vec::new(),
            prev_hand_len: 0,
            light_ramp: 0.0,
            // Start the candle light-ramp immediately on scene entry so the
            // fade-in isn't gated on the first deal. With `pending_chamber`,
            // the deal is deferred until the ramp completes, which would
            // otherwise deadlock (ramp waits on deal, deal waits on ramp).
            light_ramp_anchor: Some(now),
            pending_chamber: None,
            debug_show_axes: false,
            lab_mode: false,
            candle_flare: 0.0,
            held_relic_drag: None,
            cash_in_hold_started: None,
            marquee: None,
            final_tiles_fov_pop_at: None,
            hand_tile_uids: Vec::new(),
            hand_slide_y: Vec::new(),
            hand_slide_x: Vec::new(),
            journal_transition: None,
            journal_open_amount: 0.0,
            journal_open_target: 0.0,
            journal_was_open: false,
            positions: crate::ui::scene_layout::GameplayPositions::default(),
            discard_undo: None,
            invalid_meld_flash_slots: Vec::new(),
            invalid_meld_flash_at: None,
            invalid_boss_flash_at: None,
            tutorial_panel_wiggle_at: None,
            boss_rule_feedback_live: false,
        }
    }

    pub(super) fn clear_discard_undo(&mut self) {
        self.discard_undo = None;
    }

    /// Enter the gameplay scene for a round that has not yet been applied to
    /// the run. `apply_chamber` fires once the opening transition completes
    /// (`light_ramp >= 1.0`) so on-round-start effects — Sweepstakes gold,
    /// DoraCrown reveal, relic activation glows — play while the smoke
    /// curtain clears instead of being hidden behind it.
    pub fn with_pending_chamber(blind: crate::core::rules::ChamberKind) -> Self {
        let mut s = Self::new();
        s.pending_chamber = Some(blind);
        s
    }

    /// [`with_pending_chamber`] plus [`RunState::preset_round_hud_for_chamber_entry`]
    /// so score/target rollers are correct on the first gameplay frame.
    pub fn enter_pending_chamber(
        run: &mut crate::game::run::RunState,
        blind: crate::core::rules::ChamberKind,
    ) -> Self {
        run.preset_round_hud_for_chamber_entry(blind);
        Self::with_pending_chamber(blind)
    }

    /// Chamber whose BGM should play: the round being entered (`pending_chamber`)
    /// or the active round on resume (`active_chamber`).
    pub fn music_chamber_kind(
        &self,
        active_chamber: crate::core::rules::ChamberKind,
    ) -> crate::core::rules::ChamberKind {
        self.pending_chamber.unwrap_or(active_chamber)
    }

    /// Full scoring cascade + tutorial milestones after round score increases (commit autotrigger or manual trigger).
    #[allow(private_interfaces)]
    pub(crate) fn begin_scoring_cascade(
        &mut self,
        ctx: &mut UpdateCtx<'_>,
        score_before: u64,
        gained: u64,
        showcase: Option<CascadeShowcase>,
    ) {
        let gameplay = GameEngine::read(ctx.run);
        if gained == 0 {
            self.displayed_score = gameplay.round_score;
            return;
        }
        if gained >= gameplay.target_score as u64 {
            self.candle_flare = CANDLE_FLARE_PEAK;
            ctx.bus.push(crate::game::event_bus::GameEvent::CandleFlare);
        }
        if let Some(breakdown) = GameEngine::last_breakdown(ctx.run) {
            if !breakdown.steps.is_empty() || breakdown.base_points > 0 {
                let starting_fresh = self.cascade_queue.is_empty();
                let queue_len_before = self.cascade_queue.len();
                log::debug!(
                    "[score] begin_scoring_cascade: score_before={} gained={} total={} base_points={} steps={}+{} queue_before={} fresh={}",
                    score_before,
                    gained,
                    breakdown.total,
                    breakdown.base_points,
                    breakdown.base_steps.len(),
                    breakdown.steps.len(),
                    queue_len_before,
                    starting_fresh,
                );
                let cascade = ScoringCascade::with_tuning(
                    breakdown,
                    score_before,
                    gained,
                    ctx.cascade_tuning.clone(),
                );
                self.cascade_queue.push_back((cascade, showcase));
                if starting_fresh {
                    self.last_revealed_step = None;
                    self.cascade_final_emitted = false;
                    self.cascade_hud = None;
                    self.pending_dora_chimes.clear();
                }
                let (px, py) = score_counter::try_resolve_score_cascade_layout(
                    ctx.layout,
                    &self.positions,
                    ctx.room_gltf_height_scale,
                )
                .map(|c| (c.counter.reel.px, c.counter.reel.py))
                .unwrap_or_else(|| {
                    let sp = ctx.layout.score_panel;
                    (sp.x + sp.w * 0.5, sp.y + sp.h * 0.5)
                });
                let mag = (gained as f32).max(1.0).log2();
                let count = ((16.0 + mag * 8.0) as usize).clamp(16, 128);
                self.particles
                    .explode(px, py, count, [1.0, 0.86, 0.32, 1.0], 1.1);
                self.particles
                    .explode(px, py, count / 3, color::PARCHMENT, 0.9);
                if self.candle_flare > 0.0 {
                    self.particles
                        .explode(px, py, count * 2, [1.0, 0.55, 0.15, 1.0], 1.6);
                    self.particles
                        .explode(px, py, count, [1.0, 0.92, 0.55, 1.0], 1.3);
                }
            } else {
                self.displayed_score = gameplay.round_score;
            }
        } else {
            self.displayed_score = gameplay.round_score;
        }
    }

    #[allow(private_interfaces)]
    pub(super) fn popup_source(
        step: &crate::core::scoring::ScoreStep,
        layout: &crate::ui::layout::LayoutResult,
        positions: &crate::ui::scene_layout::GameplayPositions,
        run: &crate::game::run::RunState,
        cascade_showcase: Option<&CascadeShowcase>,
        env_height_scale: f32,
    ) -> crate::render::world_space::LayoutAnchorPx {
        use crate::render::score_popups::TABLE_POPUP_LIFT_Z;

        if step.kind == StepKind::Final
            && let Some(cascade) =
                score_counter::try_resolve_score_cascade_layout(layout, positions, env_height_scale)
        {
            return cascade.counter.reel;
        }

        if let Some(rid) = crate::core::relic::relic_by_name(&step.source)
            && let Some(anchor) = Self::relic_popup_anchor(layout, run, rid, env_height_scale)
        {
            return anchor;
        }

        if !step.tile_ids.is_empty() {
            if let Some(anchor) =
                Self::showcase_tile_popup_center(layout, run, cascade_showcase, &step.tile_ids)
            {
                return anchor;
            }
            if let Some(anchor) =
                Self::hand_tile_popup_center(layout, run, env_height_scale, &step.tile_ids)
            {
                return anchor;
            }
        }

        if run
            .available_yaku
            .iter()
            .any(|yaku| yaku.name() == step.source)
        {
            return Self::yaku_popup_source(layout, run, cascade_showcase, &step.source);
        }

        let ms = layout.modifier_strip;
        let (px, py) = match step.kind {
            StepKind::Chips => (ms.x + ms.w * 0.30, ms.y + ms.h * 0.60),
            StepKind::Mult => (ms.x + ms.w * 0.70, ms.y + ms.h * 0.60),
            StepKind::Yen | StepKind::Final => (ms.x + ms.w * 0.50, ms.y + ms.h * 0.45),
        };
        crate::render::world_space::LayoutAnchorPx {
            px,
            py,
            lift_z: TABLE_POPUP_LIFT_Z,
        }
    }

    fn relic_popup_anchor(
        layout: &crate::ui::layout::LayoutResult,
        run: &crate::game::run::RunState,
        rid: crate::core::relic::RelicId,
        env_height_scale: f32,
    ) -> Option<crate::render::world_space::LayoutAnchorPx> {
        let active_ids = GameEngine::active_relics(run);
        let idx = active_ids.iter().position(|&id| id == rid)?;
        glb_anchors::relic_tray_anchor(layout.window_w, layout.window_h, env_height_scale, idx).ok()
    }

    fn yaku_popup_source(
        layout: &crate::ui::layout::LayoutResult,
        run: &crate::game::run::RunState,
        _cascade_showcase: Option<&CascadeShowcase>,
        yaku_name: &str,
    ) -> crate::render::world_space::LayoutAnchorPx {
        use crate::render::score_popups::TABLE_POPUP_LIFT_Z;

        let interaction = GameEngine::read_interaction(run);
        let env_h = crate::render::room_glb::SHOP_ENV_HEIGHT_SCALE;
        if let Some(a) =
            glb_anchors::try_resolve_gameplay_glb_anchors(layout, interaction.hand_len, env_h)
        {
            let mut active: Vec<_> = run.available_yaku.to_vec();
            active.sort();
            let idx = active
                .iter()
                .position(|yaku| yaku.name() == yaku_name)
                .unwrap_or(0);
            let n = active.len().max(1) as f32;
            let t = if active.len() <= 1 {
                0.5
            } else {
                idx as f32 / (n - 1.0)
            };
            let a_l = a.yaku_marker_poses[0].anchor;
            let a_r = a.yaku_marker_poses[1].anchor;
            let anchor = crate::render::gameplay_glb::lerp_marker_anchor(a_l, a_r, t);
            return crate::render::world_space::LayoutAnchorPx {
                px: anchor[0],
                py: anchor[1],
                lift_z: anchor[2],
            };
        }
        crate::render::world_space::LayoutAnchorPx {
            px: layout.window_w * 0.5,
            py: layout.window_h * 0.35,
            lift_z: TABLE_POPUP_LIFT_Z,
        }
    }

    fn hand_tile_popup_center(
        layout: &crate::ui::layout::LayoutResult,
        run: &crate::game::run::RunState,
        env_height_scale: f32,
        tile_ids: &[u32],
    ) -> Option<crate::render::world_space::LayoutAnchorPx> {
        let interaction = GameEngine::read_interaction(run);
        let anchors = glb_anchors::try_resolve_gameplay_glb_anchors(
            layout,
            interaction.hand_len,
            env_height_scale,
        )?;
        let mut centers: Vec<[f32; 3]> = Vec::new();
        for (i, tile) in run.hand().iter().enumerate() {
            if !tile_ids.contains(&tile.id) {
                continue;
            }
            let (anchor, _, _) = anchors.hand_world_slots.get(i)?;
            centers.push(*anchor);
        }
        (!centers.is_empty()).then(|| input_handler::median_layout_anchor(&centers))
    }

    fn showcase_tile_popup_center(
        layout: &crate::ui::layout::LayoutResult,
        run: &crate::game::run::RunState,
        cascade_showcase: Option<&CascadeShowcase>,
        tile_ids: &[u32],
    ) -> Option<crate::render::world_space::LayoutAnchorPx> {
        let gameplay = GameEngine::read(run);
        let showcase = if let Some(showcase) = cascade_showcase {
            showcase.clone()
        } else if gameplay.has_structure {
            CascadeShowcase {
                tiles: Self::display_tiles(gameplay.structure_tiles.iter().copied(), run),
                sets: gameplay.structure_sets.clone(),
            }
        } else {
            return None;
        };
        let interaction = GameEngine::read_interaction(run);
        let layout_scale = (layout.window_w.min(layout.window_h)) / 600.0;
        let env_h = crate::render::room_glb::SHOP_ENV_HEIGHT_SCALE;
        let anchors =
            glb_anchors::try_resolve_gameplay_glb_anchors(layout, interaction.hand_len, env_h)?;
        input_handler::structure_showcase_tile_popup_center(
            &anchors.structure_marker_poses,
            layout,
            layout_scale,
            &showcase,
            tile_ids,
            gameplay.has_structure,
            cascade_showcase.is_some(),
        )
    }

    pub(crate) fn lab_sync_score_display(&mut self, round_score: u64) {
        self.displayed_score = round_score;
        self.prev_displayed_score = round_score;
        self.score_reel.reset_for_target(round_score);
    }

    pub(crate) fn lab_reset_score_state(&mut self) {
        self.cascade_queue.clear();
        self.score_popups.clear();
        self.displayed_score = 0;
        self.prev_displayed_score = 0;
        self.score_reel.reset_for_target(0);
    }

    pub(crate) fn lab_cascade_active(&self) -> bool {
        !self.cascade_queue.is_empty()
    }

    #[inline]
    pub(crate) fn cash_in_hold_in_progress(&self) -> bool {
        self.cash_in_hold_started.is_some()
    }

    /// Normalized hold-to-cash-in progress for the HUD ring + rumble (0..=1).
    /// Stays at 0 while cash-in is not allowed.
    #[inline]
    pub(crate) fn cash_in_hold_progress(
        &self,
        now: Instant,
        trigger_enabled: bool,
    ) -> Option<f32> {
        let started = self.cash_in_hold_started?;
        if !trigger_enabled {
            return Some(0.0);
        }
        Some(
            (now.saturating_duration_since(started).as_secs_f32() / cash_in_hold_seconds())
                .clamp(0.0, 1.0),
        )
    }

    /// Freeze the cash-in hold clock while the action cannot succeed.
    #[inline]
    pub(crate) fn tick_cash_in_hold_anchor(&mut self, now: Instant, trigger_enabled: bool) {
        if let Some(start) = self.cash_in_hold_started {
            self.cash_in_hold_started = Some(crate::ui::prompt_hold_ring::freeze_hold_anchor(
                start,
                now,
                trigger_enabled,
            ));
        }
    }

    /// Cancel an in-progress hold-to-cash-in and stop its windup sound.
    pub(crate) fn clear_cash_in_hold(&mut self, bus: &mut crate::game::event_bus::EventBus) {
        if self.cash_in_hold_started.take().is_some() {
            crate::ui::prompt_hold_ring::end_hold(bus);
        }
    }

    pub(crate) fn enter_lab_mode(&mut self) {
        self.lab_mode = true;
        self.focus = None;
        self.marquee = None;
        self.held_relic_drag = None;
    }

    pub(crate) fn exit_lab_mode(&mut self) {
        self.lab_mode = false;
    }

    pub(crate) fn lab_mode(&self) -> bool {
        self.lab_mode
    }

    /// Cascade Lab cash-in — same path for the 3D button and lab panel.
    pub(crate) fn lab_cash_in(&mut self, ctx: &mut UpdateCtx<'_>) -> bool {
        if self.lab_cascade_active() {
            return false;
        }
        if !ctx.run.can_trigger_structure_now() {
            log::warn!(
                "[CascadeLab] structure not cash-in ready (sets={} discards={})",
                ctx.run.structure_sets().len(),
                ctx.run.discards_remaining,
            );
            return false;
        }
        let score_before = GameEngine::read(ctx.run).round_score;
        let gameplay = GameEngine::read(ctx.run);
        let cascade_showcase = Some(CascadeShowcase {
            tiles: Self::display_tiles(gameplay.structure_tiles.iter().copied(), ctx.run),
            sets: gameplay.structure_sets.clone(),
        });
        let earned = ctx.run.trigger_structure_manual(ctx.bus);
        if earned == 0 {
            log::warn!("[CascadeLab] cash-in scored 0");
            ctx.anim
                .shake(crate::render::animation::ENTITY_HAND_STRIP, 6.0, 160);
            return false;
        }
        let gained = ctx.run.round_score.saturating_sub(score_before);
        if gained > 0 {
            ctx.anim.pulse(crate::render::animation::ENTITY_SCORE_PANEL);
            self.begin_scoring_cascade(ctx, score_before, gained, cascade_showcase);
            true
        } else {
            false
        }
    }
}

/// Structure melds/tokens must draw before the hand mesh or the rack depth-tests over them.
/// Finds the first `ShowcaseTileBatch` (the hand tile batch) and inserts structure
/// tiles immediately before it so they render behind the rack.
fn insert_structure_before_hand(
    mut frame: UiFrame,
    structure_showcase: Vec<ShowcaseTilePlacement>,
    vis: crate::scenes::DebugVisibility,
) -> UiFrame {
    let structure_showcase = if vis.hide_structure_tiles {
        Vec::new()
    } else {
        structure_showcase
    };
    let pos = frame
        .cmds
        .iter()
        .position(|c| matches!(c, DrawCmd::ShowcaseTileBatch(_)));
    let insert_at = pos.unwrap_or(frame.cmds.len());
    if !structure_showcase.is_empty() {
        frame
            .cmds
            .insert(insert_at, DrawCmd::ShowcaseTileBatch(structure_showcase));
    }
    frame
}

/// Debug-only invariant: kept as a no-op placeholder; hand tile markers
/// are no longer used (tiles are rendered via `ShowcaseTileBatch`).
#[inline]
fn debug_assert_marker_uniqueness(_frame: &UiFrame) {}
