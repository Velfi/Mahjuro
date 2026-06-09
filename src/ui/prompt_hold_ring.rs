//! Circular progress ring for hold-to-act prompts (shader-based annular arc).

use std::sync::OnceLock;
use std::time::Instant;

use crate::game::event_bus::{EventBus, GameEvent};
use crate::render::theme::color;
use crate::render::wgpu_renderer::ArcRingQuadInstance;
use crate::sfx_id::SfxId;

/// Fallback when audio is unavailable; matches `assets/audio/sfx/reel_up.ogg`.
pub const HOLD_ACT_FALLBACK_SECONDS: f32 = 1.384_667;

static HOLD_ACT_SECONDS: OnceLock<f32> = OnceLock::new();

/// Hold-to-act duration (seconds). Initialized from [`SfxId::HoldWindup`] at boot;
/// falls back to [`HOLD_ACT_FALLBACK_SECONDS`] until then or when audio is missing.
#[inline]
pub fn hold_act_seconds() -> f32 {
    *HOLD_ACT_SECONDS.get().unwrap_or(&HOLD_ACT_FALLBACK_SECONDS)
}

/// Called once after SFX clips are loaded so hold timers track `reel_up.ogg`.
pub fn set_hold_act_seconds(seconds: f32) {
    let _ = HOLD_ACT_SECONDS.set(seconds.max(0.05));
}

/// Start a hold-to-act timer and play the windup cue (`reel_up`) or rejection ping
/// (`can_ping`) when the action cannot succeed yet.
#[inline]
pub fn begin_hold(now: Instant, bus: &mut EventBus, valid: bool) -> Instant {
    bus.push(if valid {
        GameEvent::HoldWindupStart
    } else {
        GameEvent::UiSound(SfxId::UiCancel)
    });
    now
}

/// Stop the hold windup bed when the player releases early or abandons the hold.
#[inline]
pub fn end_hold(bus: &mut EventBus) {
    bus.push(GameEvent::HoldWindupStop);
}

/// While the hold action is invalid, the anchor does not advance (progress stays at 0).
#[inline]
pub fn freeze_hold_anchor(started: Instant, now: Instant, valid: bool) -> Instant {
    if valid { started } else { now }
}

/// Build one square GPU instance that draws a ring track plus a clockwise
/// progress arc (from top) around `(cx, cy)`. When `invalid` is true the ring
/// pulses red and progress is forced to zero.
pub fn hold_prompt_ring(
    cx: f32,
    cy: f32,
    radius: f32,
    thickness: f32,
    progress: f32,
    invalid: bool,
) -> ArcRingQuadInstance {
    let stroke = thickness.max(2.0);
    let half = stroke * 0.5;
    let outer = radius + half;
    let inner_norm = ((radius - half) / outer).clamp(0.0, 0.999);

    let (fill_color, track_color, progress, invalid_flag) = if invalid {
        (
            color::alpha([1.0, 0.14, 0.08, 1.0], 0.92),
            color::alpha([0.35, 0.08, 0.06, 1.0], 0.48),
            0.0,
            1.0,
        )
    } else {
        (
            color::alpha(color::CHAMPAGNE, 0.92),
            color::alpha(color::STONE, 0.38),
            progress.clamp(0.0, 1.0),
            0.0,
        )
    };

    ArcRingQuadInstance {
        rect: [cx - outer, cy - outer, outer * 2.0, outer * 2.0],
        fill_color,
        track_color,
        params: [inner_norm, progress, invalid_flag, 0.0],
    }
}
