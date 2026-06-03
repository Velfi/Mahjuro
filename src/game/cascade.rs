//! Scoring cascade: step-by-step reveal of a ScoreBreakdown with timing.
//!
//! The cascade progresses through phases:
//! 1. **ShowBase** — display the base points for a beat
//! 2. **ShowStep(i)** — reveal each relic/rule contribution one at a time
//! 3. **ShowTotal** — hold on the final total
//! 4. **Done** — cascade finished, input unblocked

use std::time::{Duration, Instant};

use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

use crate::core::scoring::{ScoreBreakdown, StepKind};
use crate::core::yaku::{YakuKind, yaku_kind_by_display_name};

/// Brief beat after a yaku name voice line before the cascade advances to the next yaku.
pub const YAKU_NAME_POST_PAUSE_MS: u64 = 250;

/// Tunable timing parameters for the scoring cascade animation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CascadeTuning {
    /// How long the base-points phase is displayed (ms).
    pub base_hold_ms: u64,
    /// How long each relic/rule step is displayed (ms).
    pub step_hold_ms: u64,
    /// How long the final total is held before unblocking (ms).
    pub total_hold_ms: u64,
    /// Duration for the score counter tick-up animation (ms).
    pub tick_duration_ms: u64,
    /// Fallback ceiling for post-discard refill (ms). Refill waits for the 3D
    /// discard animation to finish, but never longer than this value.
    #[serde(
        alias = "depart_lifetime_ms",
        default = "default_discard_refill_cap_ms"
    )]
    pub discard_refill_cap_ms: u64,
    /// Per-tile lift phase before the arc into the river (ms).
    #[serde(default = "default_discard_lift_ms")]
    pub discard_lift_ms: u64,
    /// Curved flight from hand to river (ms).
    #[serde(default = "default_discard_flight_ms")]
    pub discard_flight_ms: u64,
    /// Small settle on the water surface (ms).
    #[serde(default = "default_discard_landing_ms")]
    pub discard_landing_ms: u64,
    /// Upper bound on randomized per-tile launch spread (ms); clamped to 8–200.
    #[serde(default = "default_discard_stagger_ms")]
    pub discard_stagger_ms: u64,
    /// How long the previous river pile sinks before despawn (ms).
    #[serde(default = "default_discard_river_sink_ms")]
    pub discard_river_sink_ms: u64,
    /// Duration for drawn tiles to settle into position (ms). Higher = slower slide-in.
    #[serde(default = "default_draw_ms")]
    pub draw_settle_ms: u64,
    /// Duration for sort/drag tile repositioning (ms). Higher = slower shuffle.
    #[serde(default = "default_sort_ms")]
    pub sort_settle_ms: u64,
    /// Score popup: scale-up phase (ms).
    #[serde(default = "default_popup_pop_ms")]
    pub popup_pop_ms: u64,
    /// Score popup: hold at source after pop (ms).
    #[serde(default = "default_popup_loiter_ms")]
    pub popup_loiter_ms: u64,
    /// Score popup: flight into the score roller (ms).
    #[serde(default = "default_popup_fly_ms")]
    pub popup_fly_ms: u64,
    /// Score popup: peak scale overshoot above 1.0 (e.g. 0.22 → 122%).
    #[serde(default = "default_popup_overshoot")]
    pub popup_overshoot: f32,
}

fn default_discard_refill_cap_ms() -> u64 {
    700
}
fn default_discard_lift_ms() -> u64 {
    140
}
fn default_discard_flight_ms() -> u64 {
    380
}
fn default_discard_landing_ms() -> u64 {
    160
}
fn default_discard_stagger_ms() -> u64 {
    200
}
fn default_discard_river_sink_ms() -> u64 {
    450
}
fn default_draw_ms() -> u64 {
    500
}
fn default_sort_ms() -> u64 {
    400
}
fn default_popup_pop_ms() -> u64 {
    140
}
fn default_popup_loiter_ms() -> u64 {
    320
}
fn default_popup_fly_ms() -> u64 {
    920
}
fn default_popup_overshoot() -> f32 {
    0.22
}

/// Per-popup motion timing copied onto each spawn.
#[derive(Clone, Copy, Debug)]
pub struct PopupTiming {
    pub pop_ms: f32,
    pub loiter_ms: f32,
    pub fly_ms: f32,
    pub overshoot: f32,
}

impl PopupTiming {
    pub fn lifetime_secs(&self) -> f32 {
        (self.pop_ms + self.loiter_ms + self.fly_ms) / 1000.0
    }
}

impl From<&CascadeTuning> for PopupTiming {
    fn from(t: &CascadeTuning) -> Self {
        Self {
            pop_ms: t.popup_pop_ms as f32,
            loiter_ms: t.popup_loiter_ms as f32,
            fly_ms: t.popup_fly_ms as f32,
            overshoot: t.popup_overshoot,
        }
    }
}

impl Default for CascadeTuning {
    fn default() -> Self {
        Self {
            base_hold_ms: 420,
            step_hold_ms: 240,
            total_hold_ms: 520,
            tick_duration_ms: 170,
            discard_refill_cap_ms: 700,
            discard_lift_ms: 140,
            discard_flight_ms: 380,
            discard_landing_ms: 160,
            discard_stagger_ms: 200,
            discard_river_sink_ms: 450,
            draw_settle_ms: 500,
            sort_settle_ms: 400,
            popup_pop_ms: 140,
            popup_loiter_ms: 320,
            popup_fly_ms: 920,
            popup_overshoot: 0.22,
        }
    }
}

/// Brief beat after the last step before the cascade unblocks.
impl CascadeTuning {
    pub fn base_hold(&self) -> Duration {
        Duration::from_millis(self.base_hold_ms)
    }
    pub fn step_hold(&self) -> Duration {
        Duration::from_millis(self.step_hold_ms)
    }
    pub fn total_hold(&self) -> Duration {
        Duration::from_millis(self.total_hold_ms)
    }
    pub fn tick_duration(&self) -> Duration {
        Duration::from_millis(self.tick_duration_ms)
    }
}

#[derive(Clone, Debug)]
enum Phase {
    /// Brief hold before the first base-step lands.
    ShowBaseIntro,
    /// Showing base meld contribution at index `i`.
    ShowBaseStep(usize),
    /// Showing relic/rule step at index `i`.
    ShowStep(usize),
    /// Brief hold after the last scoring step.
    PostStepHold,
    /// Cascade complete.
    Done,
}

/// Active scoring cascade animation.
#[derive(Clone, Debug)]
pub struct ScoringCascade {
    pub breakdown: ScoreBreakdown,
    phase: Phase,
    phase_started: Instant,
    /// The round score *before* this hand was scored.
    pub score_before: u64,
    /// Points earned this hand.
    pub earned: u64,
    /// Timing parameters.
    tuning: CascadeTuning,
    /// Do not leave the final step of a yaku until this instant (voice + post pause).
    yaku_hold_until: Instant,
    /// Yaku whose name voice has already fired this cascade.
    yaku_voiced: FxHashSet<YakuKind>,
}

/// First scoring step for a yaku (chips line, or mult-only when chips are zero).
pub fn first_yaku_step(breakdown: &ScoreBreakdown, step_index: usize) -> Option<YakuKind> {
    let step = breakdown.steps.get(step_index)?;
    let yk = yaku_kind_by_display_name(&step.source)?;
    if step_index > 0 {
        let prev = &breakdown.steps[step_index - 1];
        if prev.source == step.source {
            return None;
        }
    }
    Some(yk)
}

/// Final scoring step for a yaku (always the mult line when chips precede it).
pub fn last_yaku_step(breakdown: &ScoreBreakdown, step_index: usize) -> Option<YakuKind> {
    let step = breakdown.steps.get(step_index)?;
    let yk = yaku_kind_by_display_name(&step.source)?;
    let next_same = breakdown
        .steps
        .get(step_index + 1)
        .is_some_and(|next| next.source == step.source);
    if next_same {
        None
    } else {
        Some(yk)
    }
}

/// What the UI should display for the current cascade frame.
pub struct CascadeFrame {
    /// The displayed round score (ticking up).
    pub displayed_score: u64,
    /// Whether the cascade is still running (blocks input).
    pub active: bool,
    /// Monotonic reveal slot across base steps first, then regular steps.
    /// Used by gameplay to fire one-shot effects for every visible beat.
    pub reveal_ordinal: Option<usize>,
    /// Current chip pile, smoothly interpolated for the active phase.
    pub displayed_chips: i32,
    /// Current mult, smoothly interpolated for the active phase.
    pub displayed_mult: f64,
    /// The most recently fired step (source label + which axis it hit).
    /// `None` while still on the base/total beats.
    pub latest_step: Option<(String, StepKind)>,
    /// 0..1 progress through the current phase, used for pulse easing.
    pub phase_t: f32,
    /// Which axis (if any) just got an update — drives the bump animation.
    pub pulse_axis: Option<StepKind>,
    /// Tile ids associated with the currently displayed step. The gameplay
    /// scene uses these to pulse the contributing tiles in sequence.
    pub highlight_tile_ids: Vec<u32>,
    /// Vertical wave phase (−1..1) for the active scoring source.
    pub wave_t: f32,
    /// Yaku tablet label to wave when a yaku step is active.
    pub active_yaku: Option<String>,
    /// Relic contributing to the active step (for glow + wave).
    pub active_relic: Option<crate::core::relic::RelicId>,
    /// Running chip pile × mult at the end of this hand.
    pub hand_total: u64,
}

impl ScoringCascade {
    pub fn with_tuning(
        breakdown: ScoreBreakdown,
        score_before: u64,
        earned: u64,
        tuning: CascadeTuning,
    ) -> Self {
        Self {
            breakdown,
            phase: Phase::ShowBaseIntro,
            phase_started: Instant::now(),
            score_before,
            earned,
            tuning,
            yaku_hold_until: Instant::now(),
            yaku_voiced: FxHashSet::default(),
        }
    }

    /// Extend the hold after the current yaku's final step until `until`.
    pub fn extend_yaku_hold(&mut self, until: Instant) {
        if until > self.yaku_hold_until {
            self.yaku_hold_until = until;
        }
    }

    /// Record that this yaku's name voice has started so we only play it once.
    pub fn mark_yaku_voiced(&mut self, yk: YakuKind) -> bool {
        self.yaku_voiced.insert(yk)
    }

    /// Advance the cascade state machine. Call once per frame.
    pub fn update(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.phase_started);
        match &self.phase {
            Phase::ShowBaseIntro => {
                if elapsed >= self.tuning.base_hold() {
                    if !self.breakdown.base_steps.is_empty() {
                        self.phase = Phase::ShowBaseStep(0);
                    } else if !self.breakdown.steps.is_empty() {
                        self.phase = Phase::ShowStep(0);
                    } else {
                        self.phase = Phase::PostStepHold;
                    }
                    self.phase_started = now;
                }
            }
            Phase::ShowBaseStep(i) => {
                let i = *i;
                if elapsed >= self.tuning.step_hold() {
                    if i + 1 < self.breakdown.base_steps.len() {
                        self.phase = Phase::ShowBaseStep(i + 1);
                    } else if !self.breakdown.steps.is_empty() {
                        self.phase = Phase::ShowStep(0);
                    } else {
                        self.phase = Phase::PostStepHold;
                    }
                    self.phase_started = now;
                }
            }
            Phase::ShowStep(i) => {
                let i = *i;
                if elapsed >= self.tuning.step_hold() {
                    if last_yaku_step(&self.breakdown, i).is_some() && now < self.yaku_hold_until {
                        return;
                    }
                    if i + 1 < self.breakdown.steps.len() {
                        self.phase = Phase::ShowStep(i + 1);
                    } else {
                        self.phase = Phase::PostStepHold;
                    }
                    self.phase_started = now;
                }
            }
            Phase::PostStepHold => {
                if elapsed >= self.tuning.total_hold() {
                    self.phase = Phase::Done;
                    self.phase_started = now;
                }
            }
            Phase::Done => {}
        }
    }

    /// Is the cascade still animating?
    pub fn is_active(&self) -> bool {
        !matches!(self.phase, Phase::Done)
    }

    /// Is the cascade on the closing beat (or already done)?
    pub fn is_in_total(&self) -> bool {
        matches!(self.phase, Phase::PostStepHold | Phase::Done)
    }

    pub fn popup_timing(&self) -> PopupTiming {
        PopupTiming::from(&self.tuning)
    }

    /// Build the current frame for rendering.
    pub fn frame(&self, now: Instant) -> CascadeFrame {
        let elapsed = now.saturating_duration_since(self.phase_started);

        // How far through the score tick are we?
        let tick_t = (elapsed.as_secs_f32() / self.tuning.tick_duration().as_secs_f32()).min(1.0);

        // Phase-relative 0..1 (used by the renderer for the pulse envelope).
        let phase_t = match &self.phase {
            Phase::ShowBaseIntro => {
                (elapsed.as_secs_f32() / self.tuning.base_hold().as_secs_f32()).min(1.0)
            }
            Phase::ShowBaseStep(_) => {
                (elapsed.as_secs_f32() / self.tuning.step_hold().as_secs_f32()).min(1.0)
            }
            Phase::ShowStep(_) => {
                (elapsed.as_secs_f32() / self.tuning.step_hold().as_secs_f32()).min(1.0)
            }
            Phase::PostStepHold => {
                (elapsed.as_secs_f32() / self.tuning.total_hold().as_secs_f32()).min(1.0)
            }
            Phase::Done => 1.0,
        };

        let wave_t = if matches!(
            self.phase,
            Phase::ShowBaseStep(_) | Phase::ShowStep(_)
        ) {
            (phase_t * std::f32::consts::TAU * 1.35).sin()
        } else {
            0.0
        };

        // Reel ticks on every scoring beat: round score = score_before + hand running total.
        let (score_target, reveal_ordinal) = match &self.phase {
            Phase::ShowBaseIntro => (self.score_before, None),
            Phase::ShowBaseStep(i) => {
                let i = *i;
                let step = &self.breakdown.base_steps[i];
                let from = if i > 0 {
                    self.score_before + self.breakdown.base_steps[i - 1].running_total
                } else {
                    self.score_before
                };
                let to = self.score_before + step.running_total;
                (lerp_u64(from, to, tick_t), Some(i))
            }
            Phase::ShowStep(i) => {
                let i = *i;
                let step = &self.breakdown.steps[i];
                let from = if i > 0 {
                    self.score_before + self.breakdown.steps[i - 1].running_total
                } else {
                    self.score_before + self.breakdown.base_steps.last().map_or(0, |s| s.running_total)
                };
                let to = self.score_before + step.running_total;
                (lerp_u64(from, to, tick_t), Some(self.breakdown.base_steps.len() + i))
            }
            Phase::PostStepHold | Phase::Done => (self.score_before + self.earned, None),
        };

        // ── Two-axis chips/mult readout ────────────────────────────────────
        //
        // Compute the *currently displayed* chip pile and mult value, smoothly
        // interpolating across the active phase. This lets the renderer draw
        // them as two big counters that tick up rather than as a text scroll.
        let (displayed_chips, displayed_mult, latest_step, pulse_axis, highlight_tile_ids) =
            match &self.phase {
                Phase::ShowBaseIntro => (0, 1.0, None, None, Vec::new()),
                Phase::ShowBaseStep(i) => {
                    let i = *i;
                    let step = &self.breakdown.base_steps[i];
                    let prev_chips = if i > 0 {
                        self.breakdown.base_steps[i - 1].running_chips as f64
                    } else {
                        0.0
                    };
                    let to_chips = step.running_chips as f64;
                    let t = tick_t as f64;
                    let chips = (prev_chips + (to_chips - prev_chips) * t).round() as i32;
                    (
                        chips,
                        1.0,
                        Some((step.source.clone(), step.kind)),
                        Some(StepKind::Chips),
                        step.tile_ids.clone(),
                    )
                }
                Phase::ShowStep(i) => {
                    let i = *i;
                    let step = &self.breakdown.steps[i];
                    let (prev_chips, prev_mult) = if i > 0 {
                        let p = &self.breakdown.steps[i - 1];
                        (p.running_chips as f64, p.running_mult)
                    } else {
                        (self.breakdown.base_chips as f64, 1.0)
                    };
                    let to_chips = step.running_chips as f64;
                    let to_mult = step.running_mult;
                    let t = tick_t as f64;
                    let chips = (prev_chips + (to_chips - prev_chips) * t).round() as i32;
                    let mult = prev_mult + (to_mult - prev_mult) * t;
                    let pulse = if step.kind == StepKind::Final {
                        None // final beat handled by total color
                    } else {
                        Some(step.kind)
                    };
                    (
                        chips,
                        mult,
                        Some((step.source.clone(), step.kind)),
                        pulse,
                        step.tile_ids.clone(),
                    )
                }
                Phase::PostStepHold | Phase::Done => {
                    let chips = self
                        .breakdown
                        .steps
                        .last()
                        .map(|s| s.running_chips)
                        .unwrap_or(self.breakdown.base_chips);
                    let mult = self
                        .breakdown
                        .steps
                        .last()
                        .map(|s| s.running_mult)
                        .unwrap_or(1.0);
                    (chips, mult, None, None, Vec::new())
                }
            };

        let (active_yaku, active_relic) = latest_step
            .as_ref()
            .map(|(source, _)| {
                let yaku = crate::core::yaku::yaku_kind_by_display_name(source)
                    .map(|_| source.clone());
                let relic = crate::core::relic::relic_by_name(source);
                (yaku, relic)
            })
            .unwrap_or((None, None));
        let hand_total = self.score_before + self.earned;

        CascadeFrame {
            displayed_score: score_target,
            active: self.is_active(),
            reveal_ordinal,
            displayed_chips,
            displayed_mult,
            latest_step,
            phase_t,
            pulse_axis,
            highlight_tile_ids,
            wave_t,
            active_yaku,
            active_relic,
            hand_total,
        }
    }
}

fn lerp_u64(from: u64, to: u64, t: f32) -> u64 {
    if to >= from {
        from + ((to - from) as f64 * t as f64) as u64
    } else {
        from - ((from - to) as f64 * t as f64) as u64
    }
}
