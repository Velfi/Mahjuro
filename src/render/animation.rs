//! Time-based tweens layered on layout rest poses.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Entity ids for HUD regions (MVP).
pub const ENTITY_HAND_STRIP: u32 = 1;
pub const ENTITY_SCORE_PANEL: u32 = 2;

#[derive(Clone, Copy, Debug)]
struct Tween {
    started: Instant,
    duration: Duration,
    /// Scale multiplier 1 → peak → 1
    peak: f32,
}

impl Tween {
    fn scale_at(&self, now: Instant) -> f32 {
        let t = now.saturating_duration_since(self.started).as_secs_f32()
            / self.duration.as_secs_f32().max(0.0001);
        if t >= 1.0 {
            return 1.0;
        }
        // ease out sine bump
        let x = (t * std::f32::consts::PI).sin();
        1.0 + (self.peak - 1.0) * x
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Transform2D {
    pub offset_x: f32,
    pub offset_y: f32,
    pub scale: f32,
}

pub struct AnimationController {
    tweens: HashMap<u32, Tween>,
    now: Instant,
}

impl AnimationController {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            tweens: HashMap::new(),
            now,
        }
    }

    /// Advance clock; drops finished tweens.
    pub fn update(&mut self, now: Instant) {
        self.now = now;
        self.tweens
            .retain(|_, tw| now.saturating_duration_since(tw.started) < tw.duration);
    }

    pub fn pulse(&mut self, entity: u32) {
        self.tweens.insert(
            entity,
            Tween {
                started: self.now,
                duration: Duration::from_millis(280),
                peak: 1.06,
            },
        );
    }

    pub fn transform_for(&self, entity: u32) -> Transform2D {
        let scale = self
            .tweens
            .get(&entity)
            .map(|t| t.scale_at(self.now))
            .unwrap_or(1.0);
        Transform2D {
            offset_x: 0.0,
            offset_y: 0.0,
            scale,
        }
    }

    pub fn is_idle(&self) -> bool {
        self.tweens.is_empty()
    }
}

/// Apply scale around rect center.
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
