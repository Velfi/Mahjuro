//! Gameplay scene — the main tile-playing loop.
//!
//! **Concurrent UI / flow state** (orthogonal concerns; check the right gate per feature):
//! - App-level [`crate::UpdateCtx::transitioning`] — global scene replace in progress; input blocked.
//! - `pending_blind` — opening round; hand not yet applied until the candle light-ramp completes.
//! - `pause_menu` — pause root + optional embedded options + meld-guide request.
//! - `cascade_queue` — scoring presentation; also drives [`crate::scenes::SceneBehavior::has_blocking_overlay`].
//! - `journal_transition` + overlay push — Yaku Journal from the table book.
//! - `tutorial_overlay` — in-run hints when the tutorial is active.

mod action_bar_layout;
mod action_prompts;
mod animation_state;
mod candle;
mod cascade_controller;
mod cascade_hud;
mod focus;
mod hand_layout;
mod input_handler;
mod onboarding_hints;
mod scene_behavior;
mod score_counter;

use std::collections::VecDeque;
use std::time::Instant;

use crate::core::hand::suggest_completions;
use crate::core::scoring::StepKind;
use crate::game::cascade::ScoringCascade;
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
use crate::ui::scene_layout::GameplayPositions;

use super::journal_transition::JournalTransition;
use super::pause_menu::PauseMenu;
use super::{ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

use action_bar_layout::{
    ActionBarLayout, action_hud_world_z_py_nudge, compute_gameplay_hud_layout,
};
use candle::CandleState;
use cascade_hud::{
    CascadeHandoffStage, CascadeHudState, CascadeShowcase, CascadeTokenLayout,
    build_cascade_hud_placements,
};
use focus::{FocusTarget, PegKind, focus_kind};
use hand_layout::hand_slots_for_count;

/// `pick_id` for the consumable inventory dish (Zodiacs + Talismans). Used
/// to look up the dish's projected screen rect from `ctx.aux_dish_rects`
/// so the per-slot hit-test, focus ring, and click target
/// anchor track the visible (perspective-projected) dish position
/// instead of the raw pixel anchor we hand the renderer.
const PICK_CONSUMABLE_DISH: u32 = 1;

use crate::ui::focus_nav::push_focus_ring;

/// Memoized inputs + result for `suggest_completions`. The lookup key is a
/// cheap fingerprint of the hand identities and the selection bitmask so we
/// can decide whether to reuse the cached `hints`.
#[derive(Default)]
struct SuggestHintCache {
    /// Hand tile uids (`Tile.id`) at the time the cache was last filled.
    /// Cheaper to compare than copying full `Tile` values.
    hand_uids: Vec<u32>,
    /// Bitmask of selected hand indices. Hands max out at 16 tiles, so a
    /// `u32` is plenty of headroom.
    selection_mask: u32,
    hints: Vec<usize>,
}

impl SuggestHintCache {
    fn matches(&self, hand: &[crate::core::tile::Tile], selection_mask: u32) -> bool {
        if self.selection_mask != selection_mask {
            return false;
        }
        if self.hand_uids.len() != hand.len() {
            return false;
        }
        self.hand_uids
            .iter()
            .zip(hand.iter())
            .all(|(uid, tile)| *uid == tile.id)
    }
}

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
    /// Memoized result of [`crate::core::hand::suggest_completions`]. The
    /// hint computation runs `validate_selection` for every unselected tile
    /// (each call performs full backtracking validation), so we cache it
    /// against the inputs that affect its output. `draw_frame` takes `&self`
    /// so we use a `RefCell` to update the cache from the read path.
    suggest_hint_cache: std::cell::RefCell<SuggestHintCache>,
    /// Tile indices that should depart this frame (set during update, consumed during draw).
    pending_departures: Vec<usize>,
    /// When set, the hand has been discarded-from but not yet refilled. The
    /// auto-draw fires once `Instant::now()` reaches this deadline, giving the
    /// discard departure animation time to play out.
    pending_refill: Option<Instant>,
    /// Latest cursor position (window coords), captured each update for hover picking.
    cursor_pos: (f32, f32),
    /// Score panel (2) + hand strip — upper pair (2) + lower pair (2) + footlight (1).
    /// Updated every frame; consumed in `draw()` to position flames + lights.
    candles: [CandleState; 7],
    /// Wall-clock time used to advance candle flicker (independent of the
    /// game's `Instant::now()` references so the candles keep moving even if
    /// the game logic is paused).
    candle_time: f32,
    /// Deal-wind snuff envelope in [~0.12, 1.0]; shared by flames and point lights.
    candle_wind_dim: f32,
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
    /// Sub-phase of the hand-off tween last observed, so we can fire
    /// one-shot sounds on edges (merge-land, flight-start, reel-land).
    cascade_handoff_stage: CascadeHandoffStage,
    /// Future timestamps at which a queued `DoraScored` chime should fire.
    /// Populated with one entry per dora tile when the cascade reveals a
    /// "Dora ×N" step, so multiple dora audibly play in sequence rather
    /// than a single stacked hit. Drained each frame as entries come due.
    pending_dora_chimes: Vec<Instant>,
    /// Yaku Journal overlay — Balatro-style run-stats page listing every
    /// yaku with its level, leveled bonuses, and run play count. Opened
    /// by clicking the Journal book on the table.
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
    /// Lights-out transition: ramps from 0.0 (dark) to 1.0 (full brightness)
    /// after the opening deal lands. Multiplied into candle intensity, flame
    /// alpha, and point-light brightness so the scene fades in via the
    /// candles "sparking on" rather than a smoke curtain.
    light_ramp: f32,
    /// Wall time of the deal that started the ramp. Kept until `light_ramp`
    /// reaches 1.0 so clearing `last_deal_at` after the opening wind
    /// cannot strand the ramp mid-way when wind ends before ~1.1s.
    light_ramp_anchor: Option<Instant>,
    /// Blind to apply to the run once the opening light-ramp completes. Set
    /// by callers that enter the gameplay scene from pick-blind / onboarding
    /// / retry paths; consumed by `update()` when `light_ramp >= 1.0`. While
    /// `Some`, the scene renders the *previous* round's state behind the
    /// opening transition — the player doesn't see the round's hand,
    /// target, or on-start relic triggers until the fade-in finishes, so
    /// Sweepstakes coin showers / DoraCrown reveals / future on-round-start
    /// effects are visible rather than hidden by the dark transition.
    pending_blind: Option<crate::core::rules::BlindKind>,
    /// Debug-only: when true, the renderer overlays world-axes bars at the
    /// camera target so we can see which direction is +X / +Y / +Z while
    /// dialing in placements. Toggled from the native Debug menu.
    debug_show_axes: bool,
    /// Candle flare intensity — spikes when a single hand scores more than
    /// the entire blind target, then decays exponentially back to 0.0.
    /// Multiplied into candle intensity, radius, and flame brightness so
    /// the room visibly flares up on a monster hand.
    candle_flare: f32,
    /// Controller "pick up relic with A" drag source. While set, d-pad/stick
    /// navigation moves the focused drop target and releasing A commits the
    /// swap if focus is on another relic.
    held_relic_drag: Option<usize>,
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
    /// Normalized screen-relative positions for the gameplay scene.
    /// Loaded from JSON on construction; falls back to compiled defaults.
    pub positions: crate::ui::scene_layout::GameplayPositions,
    /// When set, the player can undo the last discard (accessibility option)
    /// until any other gameplay action invalidates it.
    pub(super) discard_undo: Option<DiscardUndoSnapshot>,
    /// Hand slot indices to wiggle + pulse red after a rejected meld commit.
    invalid_meld_flash_slots: Vec<usize>,
    /// Wall time when [`invalid_meld_flash_slots`] was last set.
    invalid_meld_flash_at: Option<Instant>,
}

/// How long the debug `B` gust stays active after a press.
const DEBUG_WIND_DURATION: f32 = 0.9;

/// How long after the opening deal before the candles begin sparking on.
const LIGHT_RAMP_DELAY_SECS: f32 = 0.3;
/// Duration over which candles ramp from dark to full brightness.
const LIGHT_RAMP_DURATION_SECS: f32 = 0.8;

/// How long a relic glow lingers after activation.
const RELIC_GLOW_LIFETIME: std::time::Duration = std::time::Duration::from_millis(900);

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
        let counter = score_counter::score_counter_layout(layout, &self.positions);
        let dest = (counter.reel.px, counter.reel.py);
        let dest_lift = Some(counter.reel.lift_z);

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
            (StepKind::Gold, "Lucky Cat", 10.0),
            (StepKind::Mult, "+1.5x", 1.5),
        ];
        for (i, (kind, label, mag)) in steps.iter().enumerate() {
            let src = sources[i % sources.len()];
            self.score_popups
                .spawn((*label).to_string(), src, dest, dest_lift, *kind, *mag);
        }
        log::info!("[Debug] Spawned demo score cascade");
    }

    /// Whether the cascade is actively animating (for redraw requests).
    pub fn is_animating(&self) -> bool {
        !self.cascade_queue.is_empty()
            || self.particles.is_active()
            || self.flying_coins.is_active()
            || self.score_reel.is_animating(Instant::now())
            || self.pending_refill.is_some()
            || !self.relic_glow_starts.is_empty()
            || self.post_deal_gust_active()
            || self.debug_wind_active()
            || self.final_tiles_fov_pop_active()
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

    fn trigger_tablet_wiggle_deg(&self, now: Instant) -> f32 {
        let t = now.saturating_duration_since(self.last_frame).as_secs_f32() + self.candle_time;
        let burst = (0.5 + 0.5 * (t * 3.1).sin()).powi(2);
        burst * 8.0 * (t * 20.0).sin()
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
            suggest_hint_cache: std::cell::RefCell::new(SuggestHintCache::default()),
            pending_departures: Vec::new(),
            pending_refill: None,
            cursor_pos: (0.0, 0.0),
            candles: [
                CandleState::new(0.0),
                CandleState::new(1.7),
                CandleState::new(3.9),
                CandleState::new(5.2),
                CandleState::new(2.1),
                CandleState::new(4.4),
                CandleState::new(2.6),
            ],
            candle_time: 0.0,
            candle_wind_dim: 1.0,
            relic_glow_starts: rustc_hash::FxHashMap::default(),
            last_revealed_step: None,
            cascade_final_emitted: false,
            cascade_hud: None,
            cascade_handoff_stage: CascadeHandoffStage::Pre,
            pending_dora_chimes: Vec::new(),
            prev_hand_len: 0,
            last_deal_at: None,
            wind_delay_secs: 3.0,
            wind_duration_secs: 1.4,
            debug_wind_at: None,
            light_ramp: 0.0,
            // Start the candle light-ramp immediately on scene entry so the
            // fade-in isn't gated on the first deal. With `pending_blind`,
            // the deal is deferred until the ramp completes, which would
            // otherwise deadlock (ramp waits on deal, deal waits on ramp).
            light_ramp_anchor: Some(now),
            pending_blind: None,
            debug_show_axes: false,
            candle_flare: 0.0,
            held_relic_drag: None,
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
        }
    }

    pub(super) fn clear_discard_undo(&mut self) {
        self.discard_undo = None;
    }

    /// Enter the gameplay scene for a round that has not yet been applied to
    /// the run. `apply_blind` fires once the opening transition completes
    /// (`light_ramp >= 1.0`) so on-round-start effects — Sweepstakes gold,
    /// DoraCrown reveal, relic activation glows — play while the smoke
    /// curtain clears instead of being hidden behind it.
    pub fn with_pending_blind(blind: crate::core::rules::BlindKind) -> Self {
        let mut s = Self::new();
        s.pending_blind = Some(blind);
        s
    }

    /// Full scoring cascade + tutorial milestones after round score increases (commit autotrigger or manual trigger).
    #[allow(private_interfaces)]
    pub(super) fn begin_scoring_cascade(
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
                let cascade = ScoringCascade::with_tuning(
                    breakdown,
                    score_before,
                    gained,
                    ctx.cascade_tuning.clone(),
                );
                let starting_fresh = self.cascade_queue.is_empty();
                log::info!(
                    "[score] begin_scoring_cascade: gained={} starting_fresh={} queue_len_before={}",
                    gained,
                    starting_fresh,
                    self.cascade_queue.len(),
                );
                self.cascade_queue.push_back((cascade, showcase));
                if starting_fresh {
                    self.last_revealed_step = None;
                    self.cascade_final_emitted = false;
                    self.cascade_hud = None;
                    self.cascade_handoff_stage = CascadeHandoffStage::Pre;
                    self.pending_dora_chimes.clear();
                }
                let sp = ctx.layout.score_panel;
                let px = sp.x + sp.w * 0.5;
                let py = sp.y + sp.h * 0.5;
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

    /// Screen-space geometry for the chips (left) and mult (right) cascade
    /// accumulator tokens drawn inside the modifier strip. Single source of
    /// truth for both the draw path (`CascadeToken` placements) and popup
    /// streaming destinations, so chip/mult popups always land on the tokens
    /// the player is watching pulse.
    #[allow(private_interfaces)]
    pub(super) fn cascade_token_layout(
        layout: &crate::ui::layout::LayoutResult,
    ) -> CascadeTokenLayout {
        let ms = layout.modifier_strip;
        let src_h = ms.h * 0.36;
        let pill_y = ms.y + src_h;
        let pill_h = (ms.h - src_h - 2.0).max(8.0);
        let inner_w = ms.w * 0.80;
        let inner_x = ms.x + (ms.w - inner_w) * 0.5;
        let cross_w = pill_h * 0.7;
        let pill_w = ((inner_w - cross_w) * 0.5).max(20.0);
        let cy = pill_y + pill_h * 0.5;
        let chips_cx = inner_x + pill_w * 0.5;
        let mult_cx = inner_x + pill_w + cross_w + pill_w * 0.5;
        CascadeTokenLayout {
            chips_center: (chips_cx, cy),
            mult_center: (mult_cx, cy),
            pill_w,
            pill_h,
        }
    }

    #[allow(private_interfaces)]
    pub(super) fn popup_source_xy(
        step: &crate::core::scoring::ScoreStep,
        layout: &crate::ui::layout::LayoutResult,
        run: &crate::game::run::RunState,
        cascade_showcase: Option<&CascadeShowcase>,
        gameplay_positions: &GameplayPositions,
    ) -> (f32, f32) {
        if step.kind == StepKind::Final {
            let sp = layout.score_panel;
            return (sp.x + sp.w * 0.5, sp.y + sp.h * 0.25);
        }

        if let Some(rid) = crate::core::relic::relic_by_name(&step.source)
            && let Some(center) = Self::relic_popup_center(layout, run, rid, gameplay_positions)
        {
            return center;
        }

        if !step.tile_ids.is_empty()
            && let Some(center) =
                Self::showcase_tile_popup_center(layout, run, cascade_showcase, &step.tile_ids)
        {
            return center;
        }

        if run
            .available_yaku
            .iter()
            .any(|yaku| yaku.name() == step.source)
        {
            return Self::yaku_popup_center(layout, run, cascade_showcase);
        }

        if step.source == "Structure depth" {
            return Self::trigger_tablet_popup_center(layout, run, cascade_showcase);
        }

        let ms = layout.modifier_strip;
        match step.kind {
            StepKind::Chips => (ms.x + ms.w * 0.30, ms.y + ms.h * 0.60),
            StepKind::Mult => (ms.x + ms.w * 0.70, ms.y + ms.h * 0.60),
            StepKind::Gold | StepKind::Final => (ms.x + ms.w * 0.50, ms.y + ms.h * 0.45),
        }
    }

    fn relic_popup_center(
        layout: &crate::ui::layout::LayoutResult,
        run: &crate::game::run::RunState,
        rid: crate::core::relic::RelicId,
        gameplay_positions: &GameplayPositions,
    ) -> Option<(f32, f32)> {
        let active_ids = GameEngine::active_relics(run);
        let idx = active_ids.iter().position(|&id| id == rid)?;
        input_handler::relic_tray_screen_center_xy(gameplay_positions, layout, run, idx)
    }

    fn yaku_popup_center(
        layout: &crate::ui::layout::LayoutResult,
        run: &crate::game::run::RunState,
        cascade_showcase: Option<&CascadeShowcase>,
    ) -> (f32, f32) {
        let gameplay = GameEngine::read(run);
        let interaction = GameEngine::read_interaction(run);
        let hand_slots = hand_slots_for_count(layout, interaction.hand_len);
        let has_structure = gameplay.has_structure;
        let hud_layout = compute_gameplay_hud_layout(
            layout,
            &hand_slots,
            has_structure,
            has_structure || cascade_showcase.is_some(),
        );
        (
            hud_layout.action_bar.container_x + hud_layout.action_bar.container_w * 0.5,
            hud_layout.yaku_row_y + hud_layout.yaku_panel_h * 0.5,
        )
    }

    fn trigger_tablet_popup_center(
        layout: &crate::ui::layout::LayoutResult,
        run: &crate::game::run::RunState,
        cascade_showcase: Option<&CascadeShowcase>,
    ) -> (f32, f32) {
        let gameplay = GameEngine::read(run);
        let interaction = GameEngine::read_interaction(run);
        let hand_slots = hand_slots_for_count(layout, interaction.hand_len);
        let has_structure = gameplay.has_structure;
        let hud_layout = compute_gameplay_hud_layout(
            layout,
            &hand_slots,
            has_structure,
            has_structure || cascade_showcase.is_some(),
        );
        let trigger_btn_rect = hud_layout.action_bar.trigger_btn_rect;
        (
            trigger_btn_rect.0 + trigger_btn_rect.2 * 0.5,
            trigger_btn_rect.1 + trigger_btn_rect.3 * 0.5,
        )
    }

    fn showcase_tile_popup_center(
        layout: &crate::ui::layout::LayoutResult,
        run: &crate::game::run::RunState,
        cascade_showcase: Option<&CascadeShowcase>,
        tile_ids: &[u32],
    ) -> Option<(f32, f32)> {
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
        let hand_slots = hand_slots_for_count(layout, interaction.hand_len);
        let layout_scale = (layout.window_w.min(layout.window_h)) / 600.0;
        let has_structure = gameplay.has_structure;
        let showcase_present = has_structure || cascade_showcase.is_some();
        let hud_layout =
            compute_gameplay_hud_layout(layout, &hand_slots, has_structure, showcase_present);
        let ab = hud_layout.action_bar;
        let pad = (8.0 * layout_scale).max(6.0);
        let preview_pill_w = (22.0 * layout_scale).max(18.0);
        let preview_gap_x = (8.0 * layout_scale).max(5.0);
        let preview_lane_w = if has_structure && cascade_showcase.is_none() {
            preview_pill_w * 2.6 + preview_gap_x + pad
        } else {
            0.0
        };
        let intra_gap = (3.0 * layout_scale).max(2.0);
        let inter_gap = (10.0 * layout_scale).max(7.0);
        let total_tiles: usize = showcase.sets.iter().map(|s| s.tile_ids.len()).sum();
        let intra_count: usize = showcase
            .sets
            .iter()
            .map(|s| s.tile_ids.len().saturating_sub(1))
            .sum();
        let inter_count = showcase.sets.len().saturating_sub(1);
        let available_w = ab.container_w
            - pad * 2.0
            - preview_lane_w
            - intra_count as f32 * intra_gap
            - inter_count as f32 * inter_gap;
        let tile_size =
            (available_w / total_tiles.max(1) as f32).clamp(22.0, (44.0 * layout_scale).max(28.0));
        let meld_top = hud_layout.structure_strip_top + hud_layout.structure_tag_h;
        let center_py = meld_top + hud_layout.structure_meld_h * 0.5;
        let mut x_cursor = ab.container_x + pad;
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        let mut count = 0usize;
        for (mi, set) in showcase.sets.iter().enumerate() {
            for &tid in &set.tile_ids {
                let px = x_cursor + tile_size * 0.5;
                if tile_ids.contains(&tid) {
                    sum_x += px;
                    sum_y += center_py;
                    count += 1;
                }
                x_cursor += tile_size + intra_gap;
            }
            if mi + 1 < showcase.sets.len() {
                x_cursor += inter_gap - intra_gap;
            }
        }
        (count > 0).then_some((sum_x / count as f32, sum_y / count as f32))
    }
}

/// Structure melds/tokens must draw before the hand mesh or the rack depth-tests over them.
/// Finds the first `ShowcaseTileBatch` (the hand tile batch) and inserts structure
/// tiles immediately before it so they render behind the rack.
fn insert_structure_before_hand(
    mut frame: UiFrame,
    structure_showcase: Vec<ShowcaseTilePlacement>,
    structure_pile_tokens: Vec<Object3d>,
) -> UiFrame {
    let pos = frame
        .cmds
        .iter()
        .position(|c| matches!(c, DrawCmd::ShowcaseTileBatch(_)));
    let insert_at = pos.unwrap_or(frame.cmds.len());
    if !structure_pile_tokens.is_empty() {
        frame
            .cmds
            .insert(insert_at, DrawCmd::Object3dBatch(structure_pile_tokens));
    }
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
