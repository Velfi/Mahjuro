//! Scoring cascade: step-by-step reveal of a ScoreBreakdown with timing.
//!
//! The cascade progresses through phases:
//! 1. **ShowBase** — display the base points for a beat
//! 2. **ShowStep(i)** — reveal each relic/rule contribution one at a time
//! 3. **ShowTotal** — hold on the final total
//! 4. **Done** — cascade finished, input unblocked

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::core::scoring::{ScoreBreakdown, StepKind};

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
    /// Lifetime of the discard departure animation (ms). Higher = slower float-away.
    #[serde(default = "default_depart_ms")]
    pub depart_lifetime_ms: u64,
    /// Duration for drawn tiles to settle into position (ms). Higher = slower slide-in.
    #[serde(default = "default_draw_ms")]
    pub draw_settle_ms: u64,
    /// Duration for sort/drag tile repositioning (ms). Higher = slower shuffle.
    #[serde(default = "default_sort_ms")]
    pub sort_settle_ms: u64,
    /// Delay after a deal before the post-deal smoke "blow it away" gust
    /// fires (ms). The gameplay scene reads this each frame so live edits
    /// in the debug tuning overlay take effect on the next deal.
    #[serde(default = "default_wind_delay_ms")]
    pub wind_delay_ms: u64,
    /// Duration of the post-deal smoke gust + candle-dim envelope (ms).
    /// Shapes the 4t(1-t) bell that drives both the wind impulses and the
    /// momentary candle dimming.
    #[serde(default = "default_wind_duration_ms")]
    pub wind_duration_ms: u64,
}

fn default_depart_ms() -> u64 {
    700
}
fn default_draw_ms() -> u64 {
    500
}
fn default_sort_ms() -> u64 {
    400
}
fn default_wind_delay_ms() -> u64 {
    3000
}
fn default_wind_duration_ms() -> u64 {
    1400
}

impl Default for CascadeTuning {
    fn default() -> Self {
        Self {
            base_hold_ms: 420,
            step_hold_ms: 240,
            total_hold_ms: 520,
            tick_duration_ms: 170,
            depart_lifetime_ms: 700,
            draw_settle_ms: 500,
            sort_settle_ms: 400,
            wind_delay_ms: 3000,
            wind_duration_ms: 1400,
        }
    }
}

/// How long the cascade visibly hangs between the last step and the final
/// total beat. Acts as a poor-man's time dilation: the screen briefly
/// freezes on the previous step's values so the player anticipates the
/// closing crescendo.
const PRE_TOTAL_FREEZE_MS: u64 = 70;

/// Duration of the hand-off tween — the chips/×/mult trio merges into a
/// single `= TOTAL` label, then physically flies up from under the plaque
/// into the score reel while the reel ticks up to the new round score.
const HANDOFF_MS: u64 = 540;
/// Fraction of `HANDOFF_MS` spent on the in-place merge before flight begins.
const HANDOFF_MERGE_FRAC: f32 = 0.33;

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
    /// Brief freeze after the last step finishes, before snapping into
    /// `ShowTotal`. Acts as a dramatic anticipation pause for the closing
    /// crescendo. Holds the last step's displayed values.
    PreTotalFreeze,
    /// Holding on the final total.
    ShowTotal,
    /// Chips/×/mult trio merges into `= TOTAL`, then physically flies up
    /// into the score reel. The reel ticks from `score_before` to the new
    /// round score over this same window.
    HandOff,
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
    /// `Some(t)` during the in-place merge sub-phase of `HandOff`
    /// (0..1, eased). After this reaches 1, the flight sub-phase begins.
    pub handoff_merge_t: Option<f32>,
    /// `Some(t)` during the flight sub-phase of `HandOff` (0..1, eased).
    /// `None` during merge and all other phases.
    pub handoff_flight_t: Option<f32>,
    /// The running chip pile × mult product that the merged label displays.
    /// Matches what the score reel will land on.
    pub handoff_total: u64,
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
        }
    }

    /// Advance the cascade state machine. Call once per frame.
    pub fn update(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.phase_started);
        match &self.phase {
            Phase::ShowBaseIntro => {
                if elapsed >= self.tuning.base_hold() {
                    if !self.breakdown.base_steps.is_empty() {
                        self.phase = Phase::ShowBaseStep(0);
                    } else if self.breakdown.steps.is_empty() {
                        self.phase = Phase::ShowTotal;
                    } else {
                        self.phase = Phase::ShowStep(0);
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
                        self.phase = Phase::PreTotalFreeze;
                    }
                    self.phase_started = now;
                }
            }
            Phase::ShowStep(i) => {
                let i = *i;
                if elapsed >= self.tuning.step_hold() {
                    if i + 1 < self.breakdown.steps.len() {
                        self.phase = Phase::ShowStep(i + 1);
                    } else {
                        // Insert a brief freeze before the final beat so
                        // the screen visibly hangs and the player feels
                        // the closing crescendo land.
                        self.phase = Phase::PreTotalFreeze;
                    }
                    self.phase_started = now;
                }
            }
            Phase::PreTotalFreeze => {
                if elapsed >= Duration::from_millis(PRE_TOTAL_FREEZE_MS) {
                    self.phase = Phase::ShowTotal;
                    self.phase_started = now;
                }
            }
            Phase::ShowTotal => {
                if elapsed >= self.tuning.total_hold() {
                    self.phase = Phase::HandOff;
                    self.phase_started = now;
                }
            }
            Phase::HandOff => {
                if elapsed >= Duration::from_millis(HANDOFF_MS) {
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

    /// Is the cascade currently on the final-total beat (or already done)?
    /// The scene uses this to detect the edge into `ShowTotal` for audio.
    pub fn is_in_total(&self) -> bool {
        matches!(self.phase, Phase::ShowTotal | Phase::HandOff | Phase::Done)
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
            Phase::PreTotalFreeze => {
                (elapsed.as_millis() as f32 / PRE_TOTAL_FREEZE_MS as f32).min(1.0)
            }
            Phase::ShowTotal => {
                (elapsed.as_secs_f32() / self.tuning.total_hold().as_secs_f32()).min(1.0)
            }
            Phase::HandOff => (elapsed.as_millis() as f32 / HANDOFF_MS as f32).min(1.0),
            Phase::Done => 1.0,
        };

        // Determine the ticking score target and which step (if any) just fired.
        //
        // The reel stays pinned at `score_before` while the chips/×/mult trio
        // does the reveal work. Only during `HandOff` does the reel actually
        // tick — and it ticks all the way from `score_before` to the new
        // round total in one sweep, synced to the fly-up of the merged label.
        let (score_target, reveal_ordinal) = match &self.phase {
            Phase::ShowBaseIntro => (self.score_before, None),
            Phase::ShowBaseStep(i) => {
                let i = *i;
                (self.score_before, Some(i))
            }
            Phase::ShowStep(i) => {
                let i = *i;
                (self.score_before, Some(self.breakdown.base_steps.len() + i))
            }
            Phase::PreTotalFreeze => (self.score_before, None),
            Phase::ShowTotal => (self.score_before, None),
            Phase::HandOff => {
                let from = self.score_before;
                let to = self.score_before + self.earned;
                // Linear fraction through the handoff window.
                let handoff_linear =
                    (elapsed.as_millis() as f32 / HANDOFF_MS as f32).clamp(0.0, 1.0);
                // Reel ticks during the flight sub-phase, so the digits
                // climb as the label physically flies into the reel.
                let tick = ((handoff_linear - HANDOFF_MERGE_FRAC) / (1.0 - HANDOFF_MERGE_FRAC))
                    .clamp(0.0, 1.0);
                (lerp_u64(from, to, tick), None)
            }
            Phase::Done => (self.score_before + self.earned, None),
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
                Phase::PreTotalFreeze => {
                    // Hold the last step's chips/mult so the readout visibly
                    // freezes through the anticipation pause.
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
                Phase::ShowTotal | Phase::HandOff | Phase::Done => {
                    // Hold the final values.
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

        // ── Hand-off tween bookkeeping ─────────────────────────────────────
        let (handoff_merge_t, handoff_flight_t) = match &self.phase {
            Phase::HandOff => {
                let linear = (elapsed.as_millis() as f32 / HANDOFF_MS as f32).clamp(0.0, 1.0);
                let merge = (linear / HANDOFF_MERGE_FRAC).clamp(0.0, 1.0);
                // Ease-out cubic on the merge so the trio snaps together
                // crisply, then settles for a beat before flight begins.
                let merge_eased = 1.0 - (1.0 - merge).powi(3);
                let flight_raw =
                    ((linear - HANDOFF_MERGE_FRAC) / (1.0 - HANDOFF_MERGE_FRAC)).clamp(0.0, 1.0);
                // Ease-out cubic on flight: accelerates off the pad, softens
                // into the reel.
                let flight_eased = 1.0 - (1.0 - flight_raw).powi(3);
                (
                    Some(merge_eased),
                    if flight_raw > 0.0 {
                        Some(flight_eased)
                    } else {
                        None
                    },
                )
            }
            _ => (None, None),
        };
        let handoff_total = self.score_before + self.earned;

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
            handoff_merge_t,
            handoff_flight_t,
            handoff_total,
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
