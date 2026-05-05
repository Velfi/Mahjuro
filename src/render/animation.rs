//! Time-based tweens layered on layout rest poses.
//!
//! Supports multiple tween kinds per entity: scale, offset, rotation, opacity, shake.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Entity ids for HUD regions.
pub const ENTITY_HAND_STRIP: u32 = 1;
pub const ENTITY_SCORE_PANEL: u32 = 2;

/// Tween property kind — each entity can have one tween per kind simultaneously.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum TweenKind {
    Scale,
    OffsetXY,
    Shake,
    Opacity,
}

/// Easing function type.
#[derive(Clone, Copy, Debug)]
enum EasingFn {
    EaseOutSine,
    EaseOutCubic,
    Linear,
}

impl EasingFn {
    fn apply(self, t: f32) -> f32 {
        match self {
            EasingFn::Linear => t,
            EasingFn::EaseOutSine => (t * std::f32::consts::FRAC_PI_2).sin(),
            EasingFn::EaseOutCubic => 1.0 - (1.0 - t).powi(3),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Tween {
    started: Instant,
    duration: Duration,
    easing: EasingFn,
    // Parameters depending on kind:
    /// For Scale: peak scale multiplier (1 → peak → 1)
    peak: f32,
    /// For OffsetXY: target offset
    target_x: f32,
    target_y: f32,
    /// For Shake: amplitude in pixels
    amplitude: f32,
    /// For Opacity: start and end values
    opacity_start: f32,
    opacity_end: f32,
}

impl Tween {
    fn progress(&self, now: Instant) -> f32 {
        let t = now.saturating_duration_since(self.started).as_secs_f32()
            / self.duration.as_secs_f32().max(0.0001);
        t.min(1.0)
    }

    fn is_done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    fn scale_at(&self, now: Instant) -> f32 {
        let t = self.progress(now);
        let eased = self.easing.apply(t);
        // Bump: 1 → peak → 1 using sine
        let x = (eased * std::f32::consts::PI).sin();
        1.0 + (self.peak - 1.0) * x
    }

    fn offset_at(&self, now: Instant) -> (f32, f32) {
        let t = self.progress(now);
        let eased = self.easing.apply(t);
        (self.target_x * eased, self.target_y * eased)
    }

    fn shake_at(&self, now: Instant) -> f32 {
        let t = self.progress(now);
        if t >= 1.0 {
            return 0.0;
        }
        // Decaying oscillation
        let decay = 1.0 - t;
        let oscillation = (t * 30.0).sin();
        self.amplitude * decay * oscillation
    }

    fn opacity_at(&self, now: Instant) -> f32 {
        let t = self.progress(now);
        let eased = self.easing.apply(t);
        self.opacity_start + (self.opacity_end - self.opacity_start) * eased
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Transform2D {
    pub offset_x: f32,
    pub offset_y: f32,
    pub scale: f32,
    pub opacity: f32,
}

pub struct AnimationController {
    tweens: HashMap<(u32, TweenKind), Tween>,
    now: Instant,
}

impl AnimationController {
    pub fn new() -> Self {
        Self {
            tweens: HashMap::new(),
            now: Instant::now(),
        }
    }

    /// Advance clock; drops finished tweens.
    pub fn update(&mut self, now: Instant) {
        self.now = now;
        self.tweens.retain(|_, tw| !tw.is_done(now));
    }

    /// Scale pulse (280ms ease-out sine, peak 1.06).
    pub fn pulse(&mut self, entity: u32) {
        self.tweens.insert(
            (entity, TweenKind::Scale),
            Tween {
                started: self.now,
                duration: Duration::from_millis(280),
                easing: EasingFn::EaseOutSine,
                peak: 1.06,
                target_x: 0.0,
                target_y: 0.0,
                amplitude: 0.0,
                opacity_start: 1.0,
                opacity_end: 1.0,
            },
        );
    }

    /// Strong score-pop (220ms, peak 1.18). Used when the displayed score
    /// value itself changes — bigger and snappier than `pulse` so it reads
    /// as "the number just got bigger" rather than a generic acknowledgement.
    pub fn score_pop(&mut self, entity: u32) {
        self.tweens.insert(
            (entity, TweenKind::Scale),
            Tween {
                started: self.now,
                duration: Duration::from_millis(220),
                easing: EasingFn::EaseOutCubic,
                peak: 1.18,
                target_x: 0.0,
                target_y: 0.0,
                amplitude: 0.0,
                opacity_start: 1.0,
                opacity_end: 1.0,
            },
        );
    }

    /// Horizontal shake (decaying oscillation).
    pub fn shake(&mut self, entity: u32, amplitude: f32, duration_ms: u64) {
        self.tweens.insert(
            (entity, TweenKind::Shake),
            Tween {
                started: self.now,
                duration: Duration::from_millis(duration_ms),
                easing: EasingFn::Linear,
                peak: 1.0,
                target_x: 0.0,
                target_y: 0.0,
                amplitude,
                opacity_start: 1.0,
                opacity_end: 1.0,
            },
        );
    }

    /// Slide to a target offset over duration.
    pub fn slide_to(&mut self, entity: u32, target_x: f32, target_y: f32, duration_ms: u64) {
        self.tweens.insert(
            (entity, TweenKind::OffsetXY),
            Tween {
                started: self.now,
                duration: Duration::from_millis(duration_ms),
                easing: EasingFn::EaseOutCubic,
                peak: 1.0,
                target_x,
                target_y,
                amplitude: 0.0,
                opacity_start: 1.0,
                opacity_end: 1.0,
            },
        );
    }

    /// Fade opacity from start to end over duration.
    pub fn fade(&mut self, entity: u32, from: f32, to: f32, duration_ms: u64) {
        self.tweens.insert(
            (entity, TweenKind::Opacity),
            Tween {
                started: self.now,
                duration: Duration::from_millis(duration_ms),
                easing: EasingFn::EaseOutSine,
                peak: 1.0,
                target_x: 0.0,
                target_y: 0.0,
                amplitude: 0.0,
                opacity_start: from,
                opacity_end: to,
            },
        );
    }

    /// Compute combined transform for an entity from all active tweens.
    pub fn transform_for(&self, entity: u32) -> Transform2D {
        let mut t = Transform2D {
            offset_x: 0.0,
            offset_y: 0.0,
            scale: 1.0,
            opacity: 1.0,
        };

        if let Some(tw) = self.tweens.get(&(entity, TweenKind::Scale)) {
            t.scale = tw.scale_at(self.now);
        }
        if let Some(tw) = self.tweens.get(&(entity, TweenKind::OffsetXY)) {
            let (ox, oy) = tw.offset_at(self.now);
            t.offset_x += ox;
            t.offset_y += oy;
        }
        if let Some(tw) = self.tweens.get(&(entity, TweenKind::Shake)) {
            t.offset_x += tw.shake_at(self.now);
        }
        if let Some(tw) = self.tweens.get(&(entity, TweenKind::Opacity)) {
            t.opacity = tw.opacity_at(self.now);
        }

        t
    }

    #[allow(dead_code)] // Was used for redraw gating; kept as an introspection helper.
    pub fn is_idle(&self) -> bool {
        self.tweens.is_empty()
    }
}

/// Apply scale around rect center, with offset.
pub fn apply_transform_rect(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    t: Transform2D,
) -> (f32, f32, f32, f32) {
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    let nw = w * t.scale;
    let nh = h * t.scale;
    let nx = cx - nw * 0.5 + t.offset_x;
    let ny = cy - nh * 0.5 + t.offset_y;
    (nx, ny, nw, nh)
}
