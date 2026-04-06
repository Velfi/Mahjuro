//! Scoring cascade: step-by-step reveal of a ScoreBreakdown with timing.
//!
//! The cascade progresses through phases:
//! 1. **ShowBase** — display the base points for a beat
//! 2. **ShowStep(i)** — reveal each relic/rule contribution one at a time
//! 3. **ShowTotal** — hold on the final total
//! 4. **Done** — cascade finished, input unblocked

use std::time::{Duration, Instant};

use crate::core::scoring::ScoreBreakdown;

/// How long each phase is displayed before auto-advancing.
const BASE_HOLD: Duration = Duration::from_millis(400);
const STEP_HOLD: Duration = Duration::from_millis(350);
const TOTAL_HOLD: Duration = Duration::from_millis(600);

/// Duration for the score counter to tick from old value to new value within a phase.
const TICK_DURATION: Duration = Duration::from_millis(250);

#[derive(Clone, Debug)]
enum Phase {
    /// Showing base points.
    ShowBase,
    /// Showing relic/rule step at index `i`.
    ShowStep(usize),
    /// Holding on the final total.
    ShowTotal,
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
    pub score_before: u32,
    /// Points earned this hand.
    pub earned: u32,
}

/// What the UI should display for the current cascade frame.
pub struct CascadeFrame {
    /// Lines to show in the cascade panel (accumulated so far).
    pub lines: Vec<CascadeLine>,
    /// The displayed round score (ticking up).
    pub displayed_score: u32,
    /// Whether the cascade is still running (blocks input).
    pub active: bool,
    /// Index of the step that just appeared (for pulse animation). None if base or total.
    pub new_step_index: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct CascadeLine {
    pub text: String,
    pub color: CascadeColor,
    /// Whether this line is the one currently being revealed (for highlight).
    pub is_current: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CascadeColor {
    Base,
    Step,
    Total,
}

impl ScoringCascade {
    pub fn new(breakdown: ScoreBreakdown, score_before: u32, earned: u32) -> Self {
        Self {
            breakdown,
            phase: Phase::ShowBase,
            phase_started: Instant::now(),
            score_before,
            earned,
        }
    }

    /// Advance the cascade state machine. Call once per frame.
    pub fn update(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.phase_started);
        match &self.phase {
            Phase::ShowBase => {
                if elapsed >= BASE_HOLD {
                    if self.breakdown.steps.is_empty() {
                        self.phase = Phase::ShowTotal;
                    } else {
                        self.phase = Phase::ShowStep(0);
                    }
                    self.phase_started = now;
                }
            }
            Phase::ShowStep(i) => {
                let i = *i;
                if elapsed >= STEP_HOLD {
                    if i + 1 < self.breakdown.steps.len() {
                        self.phase = Phase::ShowStep(i + 1);
                    } else {
                        self.phase = Phase::ShowTotal;
                    }
                    self.phase_started = now;
                }
            }
            Phase::ShowTotal => {
                if elapsed >= TOTAL_HOLD {
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

    /// Skip to done (e.g. if player presses a key to skip).
    pub fn skip(&mut self) {
        self.phase = Phase::Done;
    }

    /// Build the current frame for rendering.
    pub fn frame(&self, now: Instant) -> CascadeFrame {
        let mut lines = Vec::new();
        let elapsed = now.saturating_duration_since(self.phase_started);

        // How far through the score tick are we?
        let tick_t = (elapsed.as_secs_f32() / TICK_DURATION.as_secs_f32()).min(1.0);

        // Always show base points line.
        let base_current = matches!(self.phase, Phase::ShowBase);
        lines.push(CascadeLine {
            text: format!("Base: {}", self.breakdown.base_points),
            color: CascadeColor::Base,
            is_current: base_current,
        });

        // Determine how many steps to show and the score target for ticking.
        let (steps_shown, score_target, new_step_index) = match &self.phase {
            Phase::ShowBase => {
                // Ticking from score_before toward score_before + base_points
                let target = self.score_before + self.breakdown.base_points.max(0) as u32;
                (0, lerp_u32(self.score_before, target, tick_t), None)
            }
            Phase::ShowStep(i) => {
                let i = *i;
                // Show steps 0..=i
                let running = self.breakdown.steps[i].running_total.max(0) as u32;
                let prev_running = if i > 0 {
                    self.breakdown.steps[i - 1].running_total.max(0) as u32
                } else {
                    self.breakdown.base_points.max(0) as u32
                };
                let from = self.score_before + prev_running;
                let to = self.score_before + running;
                (i + 1, lerp_u32(from, to, tick_t), Some(i))
            }
            Phase::ShowTotal | Phase::Done => {
                let total = self.score_before + self.earned;
                (self.breakdown.steps.len(), total, None)
            }
        };

        // Add step lines.
        for (idx, step) in self.breakdown.steps.iter().enumerate() {
            if idx >= steps_shown {
                break;
            }
            let is_current = new_step_index == Some(idx)
                && matches!(self.phase, Phase::ShowStep(_));
            lines.push(CascadeLine {
                text: format!("{} {}", step.source, step.effect),
                color: CascadeColor::Step,
                is_current,
            });
        }

        // Show total line when in ShowTotal or Done.
        if matches!(self.phase, Phase::ShowTotal | Phase::Done) {
            lines.push(CascadeLine {
                text: format!("= {}", self.earned),
                color: CascadeColor::Total,
                is_current: matches!(self.phase, Phase::ShowTotal),
            });
        }

        CascadeFrame {
            lines,
            displayed_score: score_target,
            active: self.is_active(),
            new_step_index,
        }
    }
}

fn lerp_u32(from: u32, to: u32, t: f32) -> u32 {
    if to >= from {
        from + ((to - from) as f32 * t) as u32
    } else {
        from - ((from - to) as f32 * t) as u32
    }
}
